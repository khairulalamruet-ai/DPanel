//! The log API over HTTP against real PostgreSQL and Redis (T10.04). The node is played by
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
use builtpanel_mod_logs::{LogsState, client_router};
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
        "logs",
        "[module]\nname = \"logs\"\nversion = \"1.0.0\"\ntitle = { en = \"Logs\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions) = (pool.clone(), sessions.clone());
            move || ModuleRoutes {
                client: Some(client_router().with_state(LogsState {
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

    async fn get(&self, who: &Browser, path: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("GET")
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

/// Plays the node: answers every `read_log` with two lines (and fails ones for "broken").
fn node(pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let rows: Vec<(Uuid, Value)> = sqlx::query_as(
                "SELECT id, payload FROM operations WHERE kind IN ('read_log', 'read_traffic', 'read_waf_events') AND status = 'pending'",
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
                if payload["username"] == "broken" {
                    sqlx::query("UPDATE operations SET status = 'failed', error = '{\"code\":\"not_found\",\"message\":\"x\",\"retryable\":false}', finished_at = now() WHERE id = $1")
                        .bind(id)
                        .execute(&pool)
                        .await
                        .unwrap();
                    continue;
                }
                if payload["type"] == "read_waf_events" {
                    let out = json!({"type": "waf_events", "available": true, "partial": false, "events": [
                        {"at_ms": 5, "method": "GET", "host": "a.example",
                         "uri": format!("/?limit={} hosts={}", payload["limit"], payload["hosts"]),
                         "rule_id": 942100, "message": "SQL Injection", "blocked": true}]});
                    sqlx::query("UPDATE operations SET status = 'succeeded', output = $2, finished_at = now() WHERE id = $1")
                        .bind(id)
                        .bind(out)
                        .execute(&pool)
                        .await
                        .unwrap();
                    continue;
                }
                if payload["type"] == "read_traffic" || payload.get("top").is_some() {
                    let out = json!({"type": "traffic", "requests": 7, "bytes": 700, "status_2xx": 5,
                        "status_3xx": 1, "status_4xx": 1, "status_5xx": 0, "bots": 2,
                        "hours": [{"at_ms": 3_600_000, "requests": 7, "bytes": 700}],
                        "top_paths": [{"path": format!("/top{}", payload["top"]), "requests": 4}],
                        "partial": false});
                    sqlx::query("UPDATE operations SET status = 'succeeded', output = $2, finished_at = now() WHERE id = $1")
                        .bind(id)
                        .bind(out)
                        .execute(&pool)
                        .await
                        .unwrap();
                    continue;
                }
                let out = json!({"type": "log_lines", "truncated": false, "lines": [
                    {"at_ms": payload["since_ms"].as_i64().unwrap() + 1, "text": format!("{} {} {}", payload["source"], payload["tail"], payload["contains"])},
                ]});
                sqlx::query("UPDATE operations SET status = 'succeeded', output = $2, finished_at = now() WHERE id = $1")
                    .bind(id)
                    .bind(out)
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
async fn logs_are_read_through_the_node_for_the_owner_only(pool: PgPool) {
    let e = env(&pool, Duration::from_secs(5)).await;
    let (alice_account, _) = client(&pool, "alice", 10_000).await;
    let (broken_account, _) = client(&pool, "broken", 10_001).await;
    let alice = e.sign_in(alice_account).await;
    let broken = e.sign_in(broken_account).await;
    let nobody = {
        let admin: Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (kind, username, email, password_hash)
             VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
        )
        .bind(HASH)
        .fetch_one(&pool)
        .await
        .unwrap();
        e.sign_in(admin).await
    };
    let played = node(pool.clone());

    let (status, got) = e
        .get(&nobody, "/api/v1/client/logs?source=web_access")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{got}");
    let (status, got) = e
        .get(
            &alice,
            "/api/v1/client/logs?source=web_error&lines=50&search=oops",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got["lines"][0]["text"], "\"web_error\" 50 \"oops\"");
    // The cursor of a live view is exclusive: the node is asked from the next millisecond.
    let (_, got) = e
        .get(&alice, "/api/v1/client/logs?source=php_error&since_ms=1000")
        .await;
    assert_eq!(got["lines"][0]["at_ms"], 1002);
    // What is not a log, or not a sensible count, is refused before the node is asked.
    for q in [
        "source=shadow",
        "source=web_access&lines=0",
        "source=web_access&lines=9999",
    ] {
        let (status, got) = e.get(&alice, &format!("/api/v1/client/logs?{q}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{q}: {got}");
    }
    let asked: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'read_log'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(asked, 2);
    // A node that cannot read it.
    let (status, got) = e
        .get(&broken, "/api/v1/client/logs?source=web_access")
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{got}");
    // A plan without the feature has none; a suspended account has nothing to read.
    sqlx::query("UPDATE tenants SET status = 'suspended' WHERE username = 'alice'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, got) = e.get(&alice, "/api/v1/client/logs?source=web_access").await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    sqlx::query("UPDATE feature_sets SET all_features = false, features = ARRAY['files']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)").execute(&pool).await.unwrap();
    let (status, got) = e.get(&alice, "/api/v1/client/logs?source=web_access").await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("feature_disabled"))
    );
    played.abort();
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_activity_is_the_accounts_own_and_pages(pool: PgPool) {
    let e = env(&pool, Duration::from_secs(5)).await;
    let (alice_account, alice_tenant) = client(&pool, "alice", 10_000).await;
    let (bob_account, bob_tenant) = client(&pool, "bob", 10_001).await;
    let alice = e.sign_in(alice_account).await;
    let bob = e.sign_in(bob_account).await;
    let staff = Uuid::now_v7();
    for (n, tenant, actor) in [
        (1, alice_tenant, alice_account),
        (2, alice_tenant, alice_account),
        (3, alice_tenant, staff),
        (4, bob_tenant, bob_account),
    ] {
        sqlx::query(
            "INSERT INTO audit_logs (at, actor_id, actor_ip, action, target_type, target_id, tenant_id, outcome, metadata)
             VALUES (now() - make_interval(secs => 10 - $1), $2, '203.0.113.9', $3, 'database', 'd1', $4, 'success', '{\"secret\":\"x\"}')",
        )
        .bind(f64::from(n))
        .bind(actor)
        .bind(if n == 3 { "database.drop" } else { "database.create" })
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    }
    let (status, got) = e.get(&alice, "/api/v1/client/activity").await;
    assert_eq!(status, StatusCode::OK, "{got}");
    let items = got["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "only Alice's own account: {got}");
    // Newest first: the staff entry, then her two.
    assert_eq!(items[0]["action"], "database.drop");
    assert_eq!(items[0]["by_you"], false);
    assert!(items[0]["ip"].is_null(), "staff's address stays hidden");
    assert_eq!(items[1]["by_you"], true);
    assert_eq!(items[1]["ip"], "203.0.113.9");
    assert!(items[1].get("metadata").is_none());
    // Pages.
    let (_, first) = e.get(&alice, "/api/v1/client/activity?limit=2").await;
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    let cursor = first["next_cursor"].as_str().unwrap();
    let (_, rest) = e
        .get(
            &alice,
            &format!("/api/v1/client/activity?limit=2&cursor={cursor}"),
        )
        .await;
    assert_eq!(rest["items"].as_array().unwrap().len(), 1);
    assert!(rest["next_cursor"].is_null());
    // A filter, and what is refused.
    let (_, only) = e
        .get(&alice, "/api/v1/client/activity?action=database.drop")
        .await;
    assert_eq!(only["items"].as_array().unwrap().len(), 1);
    for q in ["limit=0", "limit=500", "cursor=x", "action=A%20B"] {
        let (status, got) = e.get(&alice, &format!("/api/v1/client/activity?{q}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{q}: {got}");
    }
    // Bob sees only his.
    let (_, got) = e.get(&bob, "/api/v1/client/activity").await;
    assert_eq!(got["items"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn traffic_is_counted_by_the_node_for_the_owner_only(pool: PgPool) {
    let e = env(&pool, Duration::from_secs(5)).await;
    let (alice_account, _) = client(&pool, "alice", 10_000).await;
    let alice = e.sign_in(alice_account).await;
    let staff = {
        let admin: Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (kind, username, email, password_hash)
             VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
        )
        .bind(HASH)
        .fetch_one(&pool)
        .await
        .unwrap();
        e.sign_in(admin).await
    };
    let played = node(pool.clone());
    let (status, got) = e.get(&staff, "/api/v1/client/traffic").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{got}");
    let (status, got) = e.get(&alice, "/api/v1/client/traffic?range=7d&top=3").await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(
        (got["requests"].as_u64(), got["bots"].as_u64()),
        (Some(7), Some(2))
    );
    assert_eq!(got["range"], "7d");
    assert_eq!(got["hours"][0]["bytes"], 700);
    assert_eq!(
        got["top_paths"][0]["path"], "/top3",
        "the count asked for is sent on"
    );
    // The asked time is a week back from now.
    let since: i64 = sqlx::query_scalar(
        "SELECT (payload->>'since_ms')::bigint FROM operations WHERE kind = 'read_traffic' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    assert!(
        (now_ms - since - 7 * 24 * 3_600_000).abs() < 60_000,
        "{since}"
    );
    for q in ["range=1y", "top=0", "top=51"] {
        let (status, got) = e.get(&alice, &format!("/api/v1/client/traffic?{q}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{q}: {got}");
    }
    // What the firewall stopped: the account's own names and the limit are sent on.
    let (status, got) = e.get(&staff, "/api/v1/client/waf/events").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{got}");
    let (status, got) = e.get(&alice, "/api/v1/client/waf/events?limit=7").await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got["available"], true);
    assert_eq!(got["items"][0]["rule_id"], 942_100);
    assert_eq!(got["items"][0]["blocked"], true);
    assert!(
        got["items"][0]["uri"]
            .as_str()
            .unwrap()
            .starts_with("/?limit=7 hosts="),
        "{got}"
    );
    for q in ["range=1y", "limit=0", "limit=201"] {
        let (status, got) = e
            .get(&alice, &format!("/api/v1/client/waf/events?{q}"))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{q}: {got}");
    }
    // A suspended account has no running web server to count.
    sqlx::query("UPDATE tenants SET status = 'suspended' WHERE username = 'alice'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = e.get(&alice, "/api/v1/client/traffic").await;
    assert_eq!(status, StatusCode::CONFLICT);
    played.abort();
}
