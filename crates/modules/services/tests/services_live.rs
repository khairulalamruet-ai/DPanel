//! The services API over HTTP against real PostgreSQL and Redis (T10.12). The node is played by
//! the test. Ignored by default; run with DATABASE_URL, REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_cache::RedisHandle;
use builtpanel_database::modules as db;
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_services::{ServicesState, admin_router, client_router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

struct Fake {
    manifest: Manifest,
    routes: Box<dyn Fn() -> ModuleRoutes + Send + Sync>,
}

impl Module for Fake {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn routes(&self) -> ModuleRoutes {
        (self.routes)()
    }
}

struct Env {
    app: Router,
    sessions: SessionStore,
}

struct Browser {
    cookie: String,
    csrf: String,
}

async fn env(pool: &PgPool, wait: Duration) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let manifest = Manifest::parse(
        "services",
        "[module]\nname = \"logs\"\nversion = \"1.0.0\"\ntitle = { en = \"Services\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions) = (pool.clone(), sessions.clone());
            move || ModuleRoutes {
                admin: Some(admin_router().with_state(ServicesState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    wait,
                })),
                client: Some(client_router().with_state(ServicesState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    wait,
                })),
                ..ModuleRoutes::none()
            }
        }),
    });
    let registry = Registry::build(vec![module]).unwrap();
    let values: Vec<Value> = registry
        .modules()
        .iter()
        .map(|m| serde_json::to_value(m.manifest()).unwrap())
        .collect();
    let records: Vec<db::ModuleRecord<'_>> = registry
        .modules()
        .iter()
        .zip(&values)
        .map(|(m, json)| db::ModuleRecord {
            name: &m.manifest().module.name,
            version: &m.manifest().module.version,
            category: m.manifest().module.category.as_str(),
            core: m.manifest().module.core,
            manifest: json,
        })
        .collect();
    db::sync(pool, &records).await.unwrap();
    let mount = ModuleMount::new(registry, Router::new());
    mount.rebuild();
    mount.apply(&db::enabled_names(pool).await.unwrap().into_iter().collect());
    Env {
        app: mount.service(),
        sessions,
    }
}

impl Env {
    async fn sign_in(&self, account_id: Uuid) -> Browser {
        let (t, data) = self
            .sessions
            .create(NewSession {
                account_id,
                ip: None,
                user_agent: None,
            })
            .await
            .unwrap();
        Browser {
            cookie: set_cookie(&t).split(';').next().unwrap().to_owned(),
            csrf: data.csrf_token.expose().to_owned(),
        }
    }

    async fn call(&self, who: &Browser, method: &str, path: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, &who.cookie)
            .header(CSRF_HEADER, &who.csrf)
            .body(Body::empty())
            .unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

async fn client(pool: &PgPool, name: &str, uid: i32) -> (Uuid, Uuid) {
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', $1, $1 || '@example.com', $2) RETURNING id",
    )
    .bind(name)
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap();
    let node: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name) VALUES ('node1') ON CONFLICT (name) DO UPDATE SET name = 'node1' RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, $4, $5, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node)
    .bind(plan)
    .bind(name)
    .bind(uid)
    .fetch_one(pool)
    .await
    .unwrap();
    (account, tenant)
}

/// Plays the node: lists two containers, restarts what is its own and refuses the rest.
fn node(pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let rows: Vec<(Uuid, Value)> = sqlx::query_as(
                "SELECT id, payload FROM operations WHERE kind IN ('list_containers', 'restart_container', 'restart_node_service', 'read_service_log', 'list_processes', 'signal_process', 'report_server_info', 'set_timezone', 'set_swap', 'drop_caches', 'reboot_node') AND status = 'pending'",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            for (id, payload) in rows {
                sqlx::query(
                    "UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1",
                )
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
                let kind: String = sqlx::query_scalar("SELECT kind FROM operations WHERE id = $1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                let (status, out, err) = if kind == "list_containers" {
                    (
                        "succeeded",
                        Some(json!({"type": "containers", "items": [
                            {"name": "bp-alice-php", "role": "php", "image": "php:8.4", "state": "running", "status": "Up 2 hours", "created": 1_700_000_000},
                            {"name": "bp-alice-web", "role": "web", "image": "nginx", "state": "exited", "status": "Exited (0) 1 hour ago", "created": 1_700_000_100}
                        ]})),
                        None,
                    )
                } else if kind == "report_server_info" {
                    (
                        "succeeded",
                        Some(json!({"type": "server_info", "info": {
                            "hostname": "builtpanel", "os": "Ubuntu 24.04", "kernel": "6.8.0", "timezone": "Etc/UTC",
                            "uptime_seconds": 1000, "memory_total_bytes": 2000, "memory_available_bytes": 1000,
                            "swap_total_bytes": 500, "swap_used_bytes": 100, "ssh": [["permitrootlogin", "yes"]]
                        }})),
                        None,
                    )
                } else if kind == "set_timezone" && payload["zone"] != "Asia/Dhaka" {
                    (
                        "failed",
                        None,
                        Some(
                            json!({"code": "invalid_arguments", "message": "not a time zone of this machine", "retryable": false}),
                        ),
                    )
                } else if ["set_timezone", "set_swap", "drop_caches", "reboot_node"]
                    .contains(&kind.as_str())
                {
                    (
                        "succeeded",
                        Some(json!({"type": "server_action_done", "action": kind})),
                        None,
                    )
                } else if kind == "list_processes" {
                    (
                        "succeeded",
                        Some(json!({"type": "processes", "items": [
                            {"container": "bp-alice-php", "pid": 1, "uid": 10000, "state": "S (sleeping)", "command": "php-fpm: master process"},
                            {"container": "bp-alice-php", "pid": 17, "uid": 10000, "state": "R (running)", "command": "php-fpm: pool www"}
                        ]})),
                        None,
                    )
                } else if kind == "signal_process" {
                    if payload["container"] == "bp-alice-php" && payload["pid"] == 17 {
                        (
                            "succeeded",
                            Some(
                                json!({"type": "process_signalled", "pid": 17, "outcome": if payload["kill"] == true { "killed" } else { "exited" }}),
                            ),
                            None,
                        )
                    } else {
                        (
                            "failed",
                            None,
                            Some(
                                json!({"code": "invalid_arguments", "message": "this account has no such container", "retryable": false}),
                            ),
                        )
                    }
                } else if kind == "read_service_log" {
                    if payload["name"] == "database" {
                        (
                            "succeeded",
                            Some(
                                json!({"type": "log_lines", "truncated": false, "lines": [{"at_ms": 5, "text": "ready for connections"}]}),
                            ),
                            None,
                        )
                    } else {
                        (
                            "failed",
                            None,
                            Some(
                                json!({"code": "invalid_arguments", "message": "not a core service of the node", "retryable": false}),
                            ),
                        )
                    }
                } else if kind == "restart_node_service" {
                    if payload["name"] == "database" {
                        (
                            "succeeded",
                            Some(
                                json!({"type": "container_restarted", "name": "database", "state": "Up 1 second"}),
                            ),
                            None,
                        )
                    } else {
                        (
                            "failed",
                            None,
                            Some(
                                json!({"code": "invalid_arguments", "message": "not a core service of the node", "retryable": false}),
                            ),
                        )
                    }
                } else if payload["name"] == "bp-alice-web" {
                    (
                        "succeeded",
                        Some(
                            json!({"type": "container_restarted", "name": "bp-alice-web", "state": "Up 1 second"}),
                        ),
                        None,
                    )
                } else {
                    (
                        "failed",
                        None,
                        Some(
                            json!({"code": "invalid_arguments", "message": "this account has no such container", "retryable": false}),
                        ),
                    )
                };
                sqlx::query("UPDATE operations SET status = $2, output = $3, error = $4, finished_at = now() WHERE id = $1")
                    .bind(id)
                    .bind(status)
                    .bind(out)
                    .bind(err)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn an_account_lists_and_restarts_its_services_and_nobody_elses(pool: PgPool) {
    let e = env(&pool, Duration::from_secs(5)).await;
    let (alice_account, _) = client(&pool, "alice", 10_000).await;
    let alice = e.sign_in(alice_account).await;
    let admin_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(&pool, admin_id, "super_admin", None)
        .await
        .unwrap();
    let root = e.sign_in(admin_id).await;
    let played = node(pool.clone());

    assert_eq!(
        e.call(&root, "GET", "/api/v1/client/services").await.0,
        StatusCode::FORBIDDEN
    );
    let (status, list) = e.call(&alice, "GET", "/api/v1/client/services").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["items"][0]["name"], "bp-alice-php");
    assert_eq!(list["items"][1]["state"], "exited");
    // The node was asked about this account, by its id.
    let asked: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'list_containers'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE username = 'alice'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(asked["tenant_id"], tenant.to_string());

    let (status, done) = e
        .call(
            &alice,
            "POST",
            "/api/v1/client/containers/bp-alice-web/restart",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["state"], "Up 1 second");
    // What the node refuses is a 404, whatever the reason.
    for name in ["bp-bob-web", "bp-db"] {
        let (status, _) = e
            .call(
                &alice,
                "POST",
                &format!("/api/v1/client/containers/{name}/restart"),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{name}");
    }
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'service.restarted'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audited, 1);
    // A suspended account cannot restart.
    sqlx::query("UPDATE tenants SET status = 'suspended' WHERE username = 'alice'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = e
        .call(
            &alice,
            "POST",
            "/api/v1/client/containers/bp-alice-web/restart",
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("tenant_not_running"))
    );

    // ---- Processes (the account runs again).
    sqlx::query("UPDATE tenants SET status = 'active' WHERE username = 'alice'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, procs) = e.call(&alice, "GET", "/api/v1/client/processes").await;
    assert_eq!(status, StatusCode::OK, "{procs}");
    assert_eq!(procs["items"][1]["command"], "php-fpm: pool www");
    assert_eq!(
        e.call(&root, "GET", "/api/v1/client/processes").await.0,
        StatusCode::FORBIDDEN
    );
    let sig =
        |container: &str, pid: u32| format!("/api/v1/client/processes/{container}/{pid}/signal");
    let post = |who: &Browser, path: String, body: Value| {
        let app = e.app.clone();
        let (cookie, csrf) = (who.cookie.clone(), who.csrf.clone());
        async move {
            let req = Request::builder()
                .method("POST")
                .uri(path)
                .header(header::COOKIE, cookie)
                .header(CSRF_HEADER, csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let (status, out) = post(&alice, sig("bp-alice-php", 17), json!({})).await;
    assert_eq!(
        (status, out["outcome"].as_str()),
        (StatusCode::OK, Some("exited")),
        "{out}"
    );
    let (_, out) = post(&alice, sig("bp-alice-php", 17), json!({"signal": "kill"})).await;
    assert_eq!(out["outcome"], "killed");
    // Process 1, a bad signal: refused before the node is asked. Another container: 404.
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'signal_process'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        post(&alice, sig("bp-alice-php", 1), json!({})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(&alice, sig("bp-alice-php", 17), json!({"signal": "stop"}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'signal_process'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        post(&alice, sig("bp-bob-php", 17), json!({})).await.0,
        StatusCode::NOT_FOUND
    );
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'process.signalled'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audited, 2);

    // ---- Administrators: the node's core services.
    let node: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE username = 'alice'")
        .fetch_one(&pool)
        .await
        .unwrap();
    for (name, state) in [("database", "running"), ("sftp", "stopped")] {
        sqlx::query(
            "INSERT INTO node_services (node_id, name, state, detail) VALUES ($1, $2, $3, 'x')",
        )
        .bind(node)
        .bind(name)
        .bind(state)
        .execute(&pool)
        .await
        .unwrap();
    }
    let base = format!("/api/v1/admin/nodes/{node}/services");
    // Accounts and administrators without the permission see nothing.
    let plain_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'plain', 'plain@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    let plain = e.sign_in(plain_id).await;
    for who in [&alice, &plain] {
        assert_eq!(e.call(who, "GET", &base).await.0, StatusCode::FORBIDDEN);
        assert_eq!(
            e.call(who, "POST", &format!("{base}/database/restart"))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, list) = e.call(&root, "GET", &base).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["items"][0]["name"], "database");
    assert_eq!(list["items"][1]["state"], "stopped");
    let (status, log) = e
        .call(&root, "GET", &format!("{base}/database/log?lines=20"))
        .await;
    assert_eq!(status, StatusCode::OK, "{log}");
    assert_eq!(log["lines"][0]["text"], "ready for connections");
    assert_eq!(
        e.call(&root, "GET", &format!("{base}/database/log?lines=0"))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        e.call(&root, "GET", &format!("{base}/shadow/log")).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, done) = e
        .call(&root, "POST", &format!("{base}/database/restart"))
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        e.call(&root, "POST", &format!("{base}/caddy/restart"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'node_service.restarted'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);
    // The operations are the node's, not an account's.
    let tenant_ops: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind IN ('restart_node_service', 'read_service_log') AND tenant_id IS NOT NULL").fetch_one(&pool).await.unwrap();
    assert_eq!(tenant_ops, 0);
    // ---- The server tools.
    let node_name: String = sqlx::query_scalar("SELECT name FROM nodes WHERE id = $1")
        .bind(node)
        .fetch_one(&pool)
        .await
        .unwrap();
    let server = format!("/api/v1/admin/nodes/{node}/server");
    let (status, info) = e.call(&root, "GET", &server).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(
        (info["hostname"].as_str(), info["ssh"][0]["name"].as_str()),
        (Some("builtpanel"), Some("permitrootlogin"))
    );
    for who in [&alice, &plain] {
        assert_eq!(e.call(who, "GET", &server).await.0, StatusCode::FORBIDDEN);
    }
    let post_json = |who: &Browser, path: String, body: Value| {
        let app = e.app.clone();
        let (cookie, csrf) = (who.cookie.clone(), who.csrf.clone());
        async move {
            let req = Request::builder()
                .method("POST")
                .uri(path)
                .header(header::COOKIE, cookie)
                .header(CSRF_HEADER, csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    assert_eq!(
        post_json(&plain, format!("{server}/drop-caches"), json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_json(&alice, format!("{server}/swap"), json!({"megabytes": 1}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, done) = post_json(
        &root,
        format!("{server}/timezone"),
        json!({"zone": "Asia/Dhaka"}),
    )
    .await;
    assert_eq!(
        (status, done["action"].as_str()),
        (StatusCode::OK, Some("set_timezone")),
        "{done}"
    );
    assert_eq!(
        post_json(
            &root,
            format!("{server}/timezone"),
            json!({"zone": "Nowhere/Land"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post_json(&root, format!("{server}/swap"), json!({"megabytes": 512}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        post_json(
            &root,
            format!("{server}/swap"),
            json!({"megabytes": 99_999})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post_json(&root, format!("{server}/drop-caches"), json!({}))
            .await
            .0,
        StatusCode::OK
    );
    // A reboot needs the node's name, as typed.
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'reboot_node'")
            .fetch_one(&pool)
            .await
            .unwrap();
    for wrong in ["", "other", &node_name.to_uppercase()] {
        let (status, _) =
            post_json(&root, format!("{server}/reboot"), json!({"confirm": wrong})).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{wrong:?}");
    }
    let same: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'reboot_node'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, same, "nothing was asked of the node");
    let (status, done) = post_json(
        &root,
        format!("{server}/reboot"),
        json!({"confirm": node_name}),
    )
    .await;
    assert_eq!(
        (status, done["action"].as_str()),
        (StatusCode::OK, Some("reboot_node")),
        "{done}"
    );
    let audited: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_logs WHERE action LIKE 'node_server.%' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        audited,
        [
            "node_server.timezone_set",
            "node_server.swap_set",
            "node_server.caches_dropped",
            "node_server.reboot"
        ]
    );
    played.abort();
}
