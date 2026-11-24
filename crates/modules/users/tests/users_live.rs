//! Users over HTTP, against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_auth_core::password::{Argon2Params, PasswordHasher};
use builtpanel_cache::RedisHandle;
use builtpanel_database::operations;
use builtpanel_database::{modules as db, rbac::assign_role};
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_users::{UsersState, router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const USERS: &str = "/api/v1/admin/users";

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

fn manifest(name: &str) -> Manifest {
    Manifest::parse(
        name,
        &format!(
            "[module]\nname = \"{name}\"\nversion = \"1.0.0\"\ntitle = {{ en = \"{name} title\" }}\ncategory = \"core\"\ncore = true\n"
        ),
    )
    .unwrap()
}

struct Env {
    app: Router,
    sessions: SessionStore,
}

struct Browser {
    cookie: String,
    csrf: String,
}

async fn env(pool: &PgPool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let users = Arc::new(Fake {
        manifest: manifest("users"),
        routes: Box::new({
            let (pool, sessions) = (pool.clone(), sessions.clone());
            move || ModuleRoutes {
                admin: Some(router().with_state(UsersState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    hasher: PasswordHasher::new(Argon2Params::default()).unwrap(),
                    purge_after_days: 7,
                })),
                ..ModuleRoutes::none()
            }
        }),
    });
    let registry = Registry::build(vec![users]).unwrap();
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

async fn account(pool: &PgPool, name: &str, roles: &[&str]) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', $1, $1 || '@example.com', $2) RETURNING id",
    )
    .bind(name)
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap();
    for role in roles {
        assign_role(pool, id, role, None).await.unwrap();
    }
    id
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

    async fn call(
        &self,
        who: Option<&Browser>,
        method: &str,
        path: &str,
        body: Option<Value>,
        csrf: bool,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(b) = who {
            req = req.header(header::COOKIE, &b.cookie);
            if csrf {
                req = req.header(CSRF_HEADER, &b.csrf);
            }
        }
        let req = match body {
            Some(v) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

async fn add_node(pool: &PgPool, name: &str, online: bool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO nodes (name, status) VALUES ($1, CASE WHEN $2 THEN 'online' ELSE 'offline' END) RETURNING id",
    )
    .bind(name)
    .bind(online)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// What the dispatcher and the node do: the operation is delivered, then ends.
async fn finish(pool: &PgPool, op_id: &str, ok: bool) {
    let id: Uuid = op_id.parse().unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1 AND status = 'pending'")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    if ok {
        assert!(operations::succeed(pool, id, &json!({})).await.unwrap());
    } else {
        operations::fail(pool, id, &json!({"code": "boom"}), None)
            .await
            .unwrap();
    }
}

async fn operation(pool: &PgPool, op_id: &str) -> (String, Value, String) {
    sqlx::query_as::<_, (String, Value, String)>(
        "SELECT kind, payload, status FROM operations WHERE id = $1",
    )
    .bind(op_id.parse::<Uuid>().unwrap())
    .fetch_one(pool)
    .await
    .unwrap()
}

fn new_user(name: &str) -> Value {
    json!({
        "username": name,
        "email": format!("{name}@example.com"),
        "password": "correct horse battery staple 42"
    })
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn creating_a_user_makes_the_account_the_tenant_and_the_operation_together(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    assert_eq!(
        e.call(None, "GET", USERS, None, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = e.call(Some(&plain), "GET", USERS, None, false).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );

    // No node online: nothing is created, not even the account.
    let (status, body) = e
        .call(Some(&root), "POST", USERS, Some(new_user("alice")), true)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "no_node_available")
    );
    let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE kind = 'client'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(accounts, 0, "the whole request is one transaction");
    add_node(&pool, "offline1", false).await;
    let node = add_node(&pool, "node1", true).await;

    let (status, body) = e
        .call(Some(&root), "POST", USERS, Some(new_user("alice")), true)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let op_id = body["operation_id"].as_str().unwrap().to_owned();
    let user = &body["user"];
    assert_eq!(user["username"], "alice");
    assert_eq!(user["account_status"], "active");
    assert_eq!(user["tenant"]["status"], "provisioning");
    assert_eq!(user["tenant"]["username"], "alice");
    assert_eq!(user["tenant"]["plan_name"], "default");
    assert_eq!(user["tenant"]["node_id"], node.to_string());
    let (kind, payload, status) = operation(&pool, &op_id).await;
    assert_eq!(
        (kind.as_str(), status.as_str()),
        ("set_up_tenant", "pending")
    );
    assert_eq!(payload["type"], "set_up_tenant");
    assert_eq!(payload["username"], "alice");
    assert_eq!(payload["uid"], 10_000);
    assert_eq!(payload["tenant_id"], user["tenant"]["id"]);
    let role: Option<String> = sqlx::query_scalar(
        "SELECT r.name FROM account_roles ar JOIN roles r ON r.id = ar.role_id WHERE ar.account_id = $1",
    )
    .bind(user["id"].as_str().unwrap().parse::<Uuid>().unwrap())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(role.as_deref(), Some("client"));

    // The ledger moves the tenant: success makes it active.
    finish(&pool, &op_id, true).await;
    let (_, got) = e
        .call(
            Some(&root),
            "GET",
            &format!("{USERS}/{}", user["id"].as_str().unwrap()),
            None,
            false,
        )
        .await;
    assert_eq!(got["tenant"]["status"], "active");

    // Taken names, bad input, unknown plan: nothing half-made.
    for (input, want) in [
        (new_user("alice"), "username_taken"),
        (
            json!({"username": "alice2", "email": "alice@example.com", "password": "correct horse battery staple 42", "system_username": "alicetwo"}),
            "email_taken",
        ),
        (
            json!({"username": "bob", "email": "bob2@example.com", "password": "correct horse battery staple 42", "system_username": "alice"}),
            "system_username_taken",
        ),
    ] {
        let (status, body) = e.call(Some(&root), "POST", USERS, Some(input), true).await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::CONFLICT, want),
            "{body}"
        );
    }
    let (status, body) = e
        .call(
            Some(&root),
            "POST",
            USERS,
            Some(json!({"username": "Bad Name", "email": "nope", "password": "short", "plan_id": Uuid::now_v7()})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let fields: Vec<&str> = body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["field"].as_str().unwrap())
        .collect();
    for want in ["username", "email", "plan_id"] {
        assert!(fields.contains(&want), "{want} in {fields:?}");
    }
    // With a good name and email, a weak password is named too.
    let (status, body) = e
        .call(
            Some(&root),
            "POST",
            USERS,
            Some(json!({"username": "weakling", "email": "weak@example.com", "password": "short"})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["field"] == "password"),
        "{body}"
    );
    let tenants: i64 = sqlx::query_scalar("SELECT count(*) FROM tenants")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tenants, 1, "failed requests left nothing behind");

    // The same tenant on a second user takes the next free id on its node.
    let (status, body) = e
        .call(Some(&root), "POST", USERS, Some(new_user("carol")), true)
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, payload, _) = operation(&pool, body["operation_id"].as_str().unwrap()).await;
    assert_eq!(payload["uid"], 10_001);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_failed_setup_is_visible_and_can_be_retried(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    add_node(&pool, "node1", true).await;
    let (_, body) = e
        .call(Some(&root), "POST", USERS, Some(new_user("dave")), true)
        .await;
    let id = body["user"]["id"].as_str().unwrap().to_owned();
    let path = format!("{USERS}/{id}");
    // Not failed yet: nothing to retry.
    let (status, body2) = e
        .call(
            Some(&root),
            "POST",
            &format!("{path}/retry-setup"),
            None,
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body2)),
        (StatusCode::CONFLICT, "tenant_busy")
    );

    finish(&pool, body["operation_id"].as_str().unwrap(), false).await;
    let (_, got) = e.call(Some(&root), "GET", &path, None, false).await;
    assert_eq!(got["tenant"]["status"], "failed");
    let (status, again) = e
        .call(
            Some(&root),
            "POST",
            &format!("{path}/retry-setup"),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["user"]["tenant"]["status"], "provisioning");
    assert_ne!(again["operation_id"], body["operation_id"]);
    finish(&pool, again["operation_id"].as_str().unwrap(), true).await;
    let (_, got) = e.call(Some(&root), "GET", &path, None, false).await;
    assert_eq!(got["tenant"]["status"], "active");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn suspend_resume_plan_change_and_delete_follow_the_ledger(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    add_node(&pool, "node1", true).await;
    let (_, body) = e
        .call(Some(&root), "POST", USERS, Some(new_user("erin")), true)
        .await;
    let id = body["user"]["id"].as_str().unwrap().to_owned();
    let path = format!("{USERS}/{id}");
    let act = |suffix: &str| format!("{path}/{suffix}");

    // While it is being set up nothing else can happen to it.
    for suffix in ["suspend", "unsuspend"] {
        let (status, b) = e.call(Some(&root), "POST", &act(suffix), None, true).await;
        assert_eq!((status, code(&b)), (StatusCode::CONFLICT, "tenant_busy"));
    }
    finish(&pool, body["operation_id"].as_str().unwrap(), true).await;

    // Suspend: the account is locked at once, the tenant when the node says so.
    let victim = e.sign_in(id.parse().unwrap()).await;
    let (status, b) = e
        .call(Some(&root), "POST", &act("suspend"), None, true)
        .await;
    assert_eq!(status, StatusCode::OK, "{b}");
    assert_eq!(b["user"]["account_status"], "suspended");
    assert_eq!(b["user"]["tenant"]["status"], "active");
    let (kind, payload, _) = operation(&pool, b["operation_id"].as_str().unwrap()).await;
    assert_eq!(
        (kind.as_str(), payload["username"].as_str()),
        ("suspend_tenant", Some("erin"))
    );
    let (status, _) = e.call(Some(&victim), "GET", USERS, None, false).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "their sessions ended");
    finish(&pool, b["operation_id"].as_str().unwrap(), true).await;
    let (_, got) = e.call(Some(&root), "GET", &path, None, false).await;
    assert_eq!(got["tenant"]["status"], "suspended");

    // Change plan while suspended: limits are queued, the plan is recorded.
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO plans (name, description, feature_set_id, cpu_millis, memory_bytes, pids, disk_bytes, inodes)
         SELECT 'small', '', feature_set_id, 500, 268435456, 100, 1073741824, 50000 FROM plans WHERE is_default
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let (status, b) = e
        .call(
            Some(&root),
            "POST",
            &act("plan"),
            Some(json!({"plan_id": plan_id})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{b}");
    assert_eq!(b["user"]["tenant"]["plan_name"], "small");
    let (kind, payload, _) = operation(&pool, b["operation_id"].as_str().unwrap()).await;
    assert_eq!(kind, "set_resource_limits");
    assert_eq!(
        (
            payload["cpu_millis"].clone(),
            payload["memory_bytes"].clone(),
            payload["pids"].clone()
        ),
        (json!(500), json!(268_435_456u64), json!(100))
    );
    let (status, b) = e
        .call(
            Some(&root),
            "POST",
            &act("plan"),
            Some(json!({"plan_id": Uuid::now_v7()})),
            true,
        )
        .await;
    assert_eq!(
        (status, code(&b)),
        (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed")
    );

    // Resume.
    let (status, b) = e
        .call(Some(&root), "POST", &act("unsuspend"), None, true)
        .await;
    assert_eq!(status, StatusCode::OK, "{b}");
    assert_eq!(b["user"]["account_status"], "active");
    finish(&pool, b["operation_id"].as_str().unwrap(), true).await;
    let (_, got) = e.call(Some(&root), "GET", &path, None, false).await;
    assert_eq!(got["tenant"]["status"], "active");

    // Delete: soft. The stack goes at once, the data waits for the purge.
    let (status, b) = e.call(Some(&root), "DELETE", &path, None, true).await;
    assert_eq!(status, StatusCode::OK, "{b}");
    assert_eq!(b["user"]["account_status"], "disabled");
    assert_eq!(b["user"]["tenant"]["status"], "deleting");
    assert!(b["user"]["tenant"]["purge_after"].is_string());
    let (kind, payload, _) = operation(&pool, b["operation_id"].as_str().unwrap()).await;
    assert_eq!(kind, "delete_tenant");
    assert_eq!(payload["purge_data"], false);
    let (status, b2) = e.call(Some(&root), "DELETE", &path, None, true).await;
    assert_eq!((status, code(&b2)), (StatusCode::CONFLICT, "tenant_busy"));
    finish(&pool, b["operation_id"].as_str().unwrap(), true).await;
    let (_, got) = e.call(Some(&root), "GET", &path, None, false).await;
    assert_eq!(
        got["tenant"]["status"], "deleting",
        "the data is still there"
    );

    // The purge, later: its success removes the tenant row; the ledger keeps the history.
    let tenant_id = got["tenant"]["id"].as_str().unwrap().to_owned();
    let node_id: Uuid = got["tenant"]["node_id"].as_str().unwrap().parse().unwrap();
    let purge = json!({"type": "delete_tenant", "tenant_id": tenant_id, "username": "erin", "uid": 10000, "purge_data": true});
    let queued = operations::enqueue(
        &pool,
        &operations::NewOperation {
            node_id,
            kind: "delete_tenant",
            tenant_id: Some(tenant_id.parse().unwrap()),
            payload: &purge,
            idempotency_key: "purge-test",
            created_by: None,
            ttl_secs: 3600,
            max_attempts: 3,
        },
    )
    .await
    .unwrap();
    finish(&pool, &queued.operation().id.to_string(), true).await;
    let (_, got) = e.call(Some(&root), "GET", &path, None, false).await;
    assert_eq!(got["tenant"], Value::Null, "the hosting account is gone");
    assert_eq!(got["account_status"], "disabled");
    let ops: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'delete_tenant'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ops, 2);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn users_are_listed_filtered_searched_updated_and_get_new_passwords(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    add_node(&pool, "node1", true).await;
    let mut ids = Vec::new();
    for name in ["frank", "grace", "heidi"] {
        let (status, b) = e
            .call(Some(&root), "POST", USERS, Some(new_user(name)), true)
            .await;
        assert_eq!(status, StatusCode::CREATED);
        ids.push((
            name,
            b["user"]["id"].as_str().unwrap().to_owned(),
            b["operation_id"].as_str().unwrap().to_owned(),
        ));
    }
    finish(&pool, &ids[0].2, true).await;

    let names = |body: &Value| -> Vec<String> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["username"].as_str().unwrap().to_owned())
            .collect()
    };
    let (_, all) = e.call(Some(&root), "GET", USERS, None, false).await;
    assert_eq!(names(&all), ["heidi", "grace", "frank"], "newest first");
    assert_eq!(all["total"], 3);
    // What each account uses of its plan: nothing measured yet means no numbers, and a reading
    // of half the plan's limits shows as about half.
    let frank = &all["items"][2]["tenant"];
    for field in [
        "memory_percent",
        "cpu_percent",
        "disk_percent",
        "inodes_percent",
    ] {
        assert!(
            frank["usage"][field].is_null(),
            "{field}: nothing is measured yet"
        );
    }
    let (tenant_id, mem, cpu, disk, inodes): (
        Uuid,
        Option<i64>,
        Option<i32>,
        Option<i64>,
        Option<i64>,
    ) = sqlx::query_as(
        "SELECT t.id, p.memory_bytes, p.cpu_millis, p.disk_bytes, p.inodes
               FROM tenants t JOIN plans p ON p.id = t.plan_id WHERE t.username = 'frank'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO usage_minute (tenant_id, at, cpu_millis, memory_bytes, pids, net_rx_bytes, net_tx_bytes, io_read_bytes, io_write_bytes)
         VALUES ($1, now(), $2, $3, 1, 0, 0, 0, 0)",
    )
    .bind(tenant_id)
    .bind(cpu.map_or(0, |c| c / 2))
    .bind(mem.map_or(0, |m| m / 2))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tenant_disk_usage (tenant_id, total_bytes, inodes) VALUES ($1, $2, $3)",
    )
    .bind(tenant_id)
    .bind(disk.map_or(0, |d| d / 2))
    .bind(inodes.map_or(0, |i| i / 2))
    .execute(&pool)
    .await
    .unwrap();
    let (_, all2) = e.call(Some(&root), "GET", USERS, None, false).await;
    let usage = &all2["items"][2]["tenant"]["usage"];
    for (field, limit) in [
        ("memory_percent", mem.is_some()),
        ("cpu_percent", cpu.is_some()),
        ("disk_percent", disk.is_some()),
        ("inodes_percent", inodes.is_some()),
    ] {
        if limit {
            let n = usage[field].as_i64().unwrap_or(-1);
            assert!((45..=50).contains(&n), "{field} is about half, not {n}");
        } else {
            assert!(usage[field].is_null(), "{field}: the plan has no limit");
        }
    }
    let (_, page) = e
        .call(
            Some(&root),
            "GET",
            &format!("{USERS}?limit=1&offset=1"),
            None,
            false,
        )
        .await;
    assert_eq!(
        (names(&page), page["total"].clone()),
        (vec!["grace".to_owned()], json!(3))
    );
    let (_, active) = e
        .call(
            Some(&root),
            "GET",
            &format!("{USERS}?status=active"),
            None,
            false,
        )
        .await;
    assert_eq!(names(&active), ["frank"]);
    let (_, found) = e
        .call(Some(&root), "GET", &format!("{USERS}?q=RAC"), None, false)
        .await;
    assert_eq!(names(&found), ["grace"], "case-insensitive substring");
    let (_, none) = e
        .call(Some(&root), "GET", &format!("{USERS}?q=%25"), None, false)
        .await;
    assert_eq!(none["total"], 0, "a percent sign is just a character");
    for bad in ["limit=0", "limit=x", "offset=-1", "status=nope"] {
        let (status, _) = e
            .call(Some(&root), "GET", &format!("{USERS}?{bad}"), None, false)
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let (status, _) = e
        .call(
            Some(&root),
            "GET",
            &format!("{USERS}/{}", Uuid::now_v7()),
            None,
            false,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // An administrator's id is not a user's.
    let (status, _) = e
        .call(
            Some(&root),
            "GET",
            &format!("{USERS}/{}", account(&pool, "other", &["admin"]).await),
            None,
            false,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Update.
    let frank = format!("{USERS}/{}", ids[0].1);
    let (status, u) = e
        .call(
            Some(&root),
            "PATCH",
            &frank,
            Some(json!({"email": "Frank.New@Example.com", "locale": "bn"})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{u}");
    assert_eq!(
        (u["email"].clone(), u["locale"].clone()),
        (json!("frank.new@example.com"), json!("bn"))
    );
    let (status, b) = e
        .call(
            Some(&root),
            "PATCH",
            &frank,
            Some(json!({"email": "grace@example.com"})),
            true,
        )
        .await;
    assert_eq!((status, code(&b)), (StatusCode::CONFLICT, "email_taken"));
    let (status, _) = e
        .call(Some(&root), "PATCH", &frank, Some(json!({})), true)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Password reset: policy applies, old sessions end, the hash changes.
    let session = e.sign_in(ids[0].1.parse().unwrap()).await;
    let before: String = sqlx::query_scalar("SELECT password_hash FROM accounts WHERE id = $1")
        .bind(ids[0].1.parse::<Uuid>().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    let pw = format!("{frank}/password");
    let (status, b) = e
        .call(
            Some(&root),
            "POST",
            &pw,
            Some(json!({"password": "short"})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    let (status, _) = e
        .call(
            Some(&root),
            "POST",
            &pw,
            Some(json!({"password": "another long passphrase 77"})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let after: String = sqlx::query_scalar("SELECT password_hash FROM accounts WHERE id = $1")
        .bind(ids[0].1.parse::<Uuid>().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(before, after);
    assert!(
        after.starts_with("$argon2id$"),
        "a PHC string, never the password"
    );
    let (status, _) = e.call(Some(&session), "GET", USERS, None, false).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Everything it did is in the audit log.
    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_logs WHERE action LIKE 'admin.user_%' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(actions.contains(&"admin.user_created".to_owned()));
    assert!(actions.contains(&"admin.user_updated".to_owned()));
    assert!(actions.contains(&"admin.user_password_reset".to_owned()));
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn only_those_allowed_get_a_token_and_only_for_an_active_user(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    add_node(&pool, "node1", true).await;
    let (_, body) = e
        .call(Some(&root), "POST", USERS, Some(new_user("ivan")), true)
        .await;
    let id = body["user"]["id"].as_str().unwrap().to_owned();
    let path = format!("{USERS}/{id}/impersonate");

    let (status, b) = e.call(Some(&plain), "POST", &path, None, true).await;
    assert_eq!(
        (status, code(&b)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );
    let (status, b) = e.call(Some(&root), "POST", &path, None, true).await;
    assert_eq!(status, StatusCode::OK, "{b}");
    assert_eq!(b["expires_in"], 60);
    assert_eq!(b["redeem_path"], "/api/v1/public/auth/impersonate");
    assert_eq!(b["token"].as_str().unwrap().len(), 43);
    // The token itself is never written to the log.
    let logged: Vec<Value> = sqlx::query_scalar(
        "SELECT metadata FROM audit_logs WHERE action = 'admin.impersonation_issued'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(logged.len(), 1);
    assert!(!logged[0].to_string().contains(b["token"].as_str().unwrap()));

    // Suspended and deleted users cannot be signed in as.
    finish(&pool, body["operation_id"].as_str().unwrap(), true).await;
    e.call(
        Some(&root),
        "POST",
        &format!("{USERS}/{id}/suspend"),
        None,
        true,
    )
    .await;
    let (status, b) = e.call(Some(&root), "POST", &path, None, true).await;
    assert_eq!(
        (status, code(&b)),
        (StatusCode::CONFLICT, "user_unavailable")
    );
    let (status, _) = e
        .call(
            Some(&root),
            "POST",
            &format!("{USERS}/{}/impersonate", Uuid::now_v7()),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn new_users_get_the_default_files_and_administrators_can_edit_them(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    add_node(&pool, "node1", true).await;
    const FILES: &str = "/api/v1/admin/skeleton-files";

    let (status, b) = e.call(Some(&plain), "GET", FILES, None, false).await;
    assert_eq!(
        (status, code(&b)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );
    // The installation ships one default page.
    let (status, list) = e.call(Some(&root), "GET", FILES, None, false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["items"][0]["path"], "index.html");
    assert!(
        list["items"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Your site is ready")
    );

    // Add and change files.
    let put = |path: &str, content: &str| json!({"path": path, "content": content});
    let (status, list) = e
        .call(
            Some(&root),
            "PUT",
            FILES,
            Some(put("css/site.css", "body{}")),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let (_, list) = e
        .call(
            Some(&root),
            "PUT",
            FILES,
            Some(put("index.html", "<h1>Mine</h1>")),
            true,
        )
        .await;
    let paths: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["css/site.css", "index.html"]);
    assert_eq!(list["total_bytes"], 6 + 13);

    // Bad paths, clashes and sizes are refused; nothing is saved.
    for (path, content) in [
        ("../etc/passwd", "x"),
        ("/abs", "x"),
        ("a//b", "x"),
        ("index.html/x", "x"),
        ("css", "x"),
        ("big.txt", &"x".repeat(16 * 1024 + 1)),
    ] {
        let (status, b) = e
            .call(Some(&root), "PUT", FILES, Some(put(path, content)), true)
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path}: {b}");
    }
    let (_, list) = e.call(Some(&root), "GET", FILES, None, false).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 2);

    // A new user's set-up carries exactly these files.
    let (status, body) = e
        .call(Some(&root), "POST", USERS, Some(new_user("judy")), true)
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, payload, _) = operation(&pool, body["operation_id"].as_str().unwrap()).await;
    assert_eq!(
        payload["skeleton"],
        json!([
            {"path": "css/site.css", "content": "body{}"},
            {"path": "index.html", "content": "<h1>Mine</h1>"}
        ])
    );
    assert!(payload.to_string().len() < 64 * 1024);

    // Removing a file changes later users only.
    let (status, _) = e
        .call(
            Some(&root),
            "DELETE",
            &format!("{FILES}?path=css/site.css"),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = e
        .call(
            Some(&root),
            "DELETE",
            &format!("{FILES}?path=css/site.css"),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, body) = e
        .call(Some(&root), "POST", USERS, Some(new_user("kate")), true)
        .await;
    let (_, payload, _) = operation(&pool, body["operation_id"].as_str().unwrap()).await;
    assert_eq!(payload["skeleton"].as_array().unwrap().len(), 1);
    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_logs WHERE action LIKE 'admin.skeleton_%' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        actions,
        [
            "admin.skeleton_file_saved",
            "admin.skeleton_file_saved",
            "admin.skeleton_file_deleted"
        ]
    );
}
