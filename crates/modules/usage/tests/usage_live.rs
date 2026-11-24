//! The usage API over HTTP against real PostgreSQL and Redis (T10.10's API). Ignored by default; run with DATABASE_URL, REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_cache::RedisHandle;
use builtpanel_database::modules as db;
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_usage::{UsageState, admin_router, client_router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::Value;
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

async fn env(pool: &PgPool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let manifest = Manifest::parse(
        "usage",
        "[module]\nname = \"logs\"\nversion = \"1.0.0\"\ntitle = { en = \"Usage\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions) = (pool.clone(), sessions.clone());
            move || ModuleRoutes {
                admin: Some(admin_router().with_state(UsageState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                })),
                client: Some(client_router().with_state(UsageState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
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

async fn admin(pool: &PgPool, name: &str, roles: &[&str]) -> Uuid {
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
        builtpanel_database::rbac::assign_role(pool, id, role, None)
            .await
            .unwrap();
    }
    id
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn accounts_see_their_own_usage_and_administrators_the_ranking_and_the_nodes(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, alice) = client(&pool, "alice", 10_000).await;
    let (bob_account, bob) = client(&pool, "bob", 10_001).await;
    let node: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE id = $1")
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
    let a = e.sign_in(alice_account).await;
    let b = e.sign_in(bob_account).await;
    let root = e
        .sign_in(admin(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e.sign_in(admin(&pool, "plain", &[]).await).await;
    // Twenty minutes of readings: alice uses 0.5 CPU and bob 0.1.
    for (tenant, cpu, mem) in [(alice, 500, 100_000_000i64), (bob, 100, 300_000_000)] {
        sqlx::query(
            "INSERT INTO usage_minute (tenant_id, at, cpu_millis, memory_bytes, pids, net_rx_bytes, net_tx_bytes, io_read_bytes, io_write_bytes)
             SELECT $1, date_trunc('minute', now()) - make_interval(mins => m), $2, $3, 4, 1000, 500, 0, 100
             FROM generate_series(1, 20) m",
        )
        .bind(tenant)
        .bind(cpu)
        .bind(mem)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO node_usage_minute (node_id, at, cpus, cpu_busy_millis, memory_used_bytes, memory_total_bytes, swap_used_bytes, load1_x100, disk_used_bytes, disk_total_bytes, containers, cpu_busy_jiffies, cpu_total_jiffies)
         SELECT $1, date_trunc('minute', now()) - make_interval(mins => m), 2, 250, 1000, 2000, 0, 80, 5000, 9000, 7, 1, 2
         FROM generate_series(1, 20) m",
    )
    .bind(node)
    .execute(&pool)
    .await
    .unwrap();

    // The account's own usage, with the plan's limits.
    let (status, mine) = e.get(&a, "/api/v1/client/usage?range=1h").await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    let points = mine["points"].as_array().unwrap();
    assert!((19..=21).contains(&points.len()), "{}", points.len());
    assert!(
        points
            .iter()
            .all(|p| p["cpu_millis"] == 500 && p["memory_bytes"] == 100_000_000i64)
    );
    assert_eq!(mine["step_seconds"], 60);
    assert!(mine["limits"].get("cpu_millis").is_some());
    let (_, theirs) = e.get(&b, "/api/v1/client/usage").await;
    assert!(
        theirs["points"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["cpu_millis"] == 100)
    );
    // Five-minute points over a day: the twenty minutes fall into a few.
    let (_, day) = e.get(&a, "/api/v1/client/usage?range=24h").await;
    assert!(day["points"].as_array().unwrap().len() <= 6);
    for q in ["range=1y", "range="] {
        let (status, _) = e.get(&a, &format!("/api/v1/client/usage?{q}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{q}");
    }
    // Administrators are not accounts.
    let (status, _) = e.get(&root, "/api/v1/client/usage").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The ranking, by what is asked.
    let (status, by_cpu) = e
        .get(&root, "/api/v1/admin/usage/top?by=cpu&range=1h")
        .await;
    assert_eq!(status, StatusCode::OK, "{by_cpu}");
    assert_eq!(by_cpu["items"][0]["username"], "alice");
    assert_eq!(by_cpu["items"][0]["cpu_millis"], 500);
    let (_, by_mem) = e.get(&root, "/api/v1/admin/usage/top?by=memory").await;
    assert_eq!(by_mem["items"][0]["username"], "bob");
    let (_, one) = e
        .get(&root, "/api/v1/admin/usage/top?by=network&limit=1")
        .await;
    assert_eq!(one["items"].as_array().unwrap().len(), 1);
    for q in ["by=heat", "limit=0", "limit=1000", "range=2y"] {
        let (status, _) = e.get(&root, &format!("/api/v1/admin/usage/top?{q}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{q}");
    }
    for path in [
        "/api/v1/admin/usage/top".to_owned(),
        format!("/api/v1/admin/usage/nodes/{node}"),
    ] {
        assert_eq!(
            e.get(&plain, &path).await.0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
        assert_eq!(e.get(&a, &path).await.0, StatusCode::FORBIDDEN, "{path}");
    }
    // A node.
    let (status, n) = e
        .get(&root, &format!("/api/v1/admin/usage/nodes/{node}?range=1h"))
        .await;
    assert_eq!(status, StatusCode::OK, "{n}");
    let pts = n["points"].as_array().unwrap();
    assert!(
        !pts.is_empty()
            && pts
                .iter()
                .all(|p| p["cpu_busy_millis"] == 250 && p["containers"] == 7)
    );
}
