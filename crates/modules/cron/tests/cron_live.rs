//! Cron jobs over HTTP, against real PostgreSQL and Redis (T10.01).
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
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
use builtpanel_mod_cron::{CronState, SETTINGS_SCHEMA, client_router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use builtpanel_settings::{Scope, SettingsRegistry};
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
    settings: Arc<SettingsRegistry>,
}

struct Browser {
    cookie: String,
    csrf: String,
}

async fn env(pool: &PgPool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let settings = Arc::new(
        SettingsRegistry::build(vec![(
            "cron".to_owned(),
            serde_json::from_str(SETTINGS_SCHEMA).unwrap(),
        )])
        .unwrap(),
    );
    let manifest = Manifest::parse(
        "cron",
        "[module]\nname = \"backups\"\nversion = \"1.0.0\"\ntitle = { en = \"Cron\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions, settings) = (pool.clone(), sessions.clone(), settings.clone());
            move || ModuleRoutes {
                client: Some(client_router().with_state(CronState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    settings: settings.clone(),
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
        settings,
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

    async fn call(
        &self,
        who: &Browser,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, &who.cookie)
            .header(CSRF_HEADER, &who.csrf);
        let req = match body {
            Some(v) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                req.body(Body::from(v.to_string())).unwrap()
            }
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

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
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

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn jobs_are_checked_against_the_rules_and_kept_per_account(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, alice_tenant) = client(&pool, "alice", 10_000).await;
    let (bob_account, _) = client(&pool, "bob", 10_001).await;
    let alice = e.sign_in(alice_account).await;
    let bob = e.sign_in(bob_account).await;
    let root = e
        .sign_in(admin(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let path = "/api/v1/client/cron-jobs";
    let job = |name: &str, schedule: &str, command: &str| json!({"name": name, "schedule": schedule, "command": command});
    let (status, _) = e.call(&root, "GET", path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, made) = e
        .call(
            &alice,
            "POST",
            path,
            Some(job("Cache", "0  *  * * *", "php artisan cache:clear")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    // Spaces between fields are tidied.
    assert_eq!(made["schedule"], "0 * * * *");
    let id = made["id"].as_str().unwrap().to_owned();

    // The shortest interval is 5 minutes: every minute, and uneven schedules too.
    for schedule in ["* * * * *", "*/4 * * * *", "0,1 * * * *", "0-3 5 * * *"] {
        let (status, got) = e
            .call(&alice, "POST", path, Some(job("Fast", schedule, "true")))
            .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{schedule}: {got}"
        );
        assert!(got.to_string().contains("too_often"), "{schedule}: {got}");
    }
    let (status, _) = e
        .call(
            &alice,
            "POST",
            path,
            Some(job("Five", "*/5 * * * *", "true")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // Not a schedule at all, or one that never comes.
    for (schedule, want) in [
        ("nonsense", "invalid"),
        ("0 0 31 2 *", "never_runs"),
        ("@hourly", "invalid"),
    ] {
        let (status, got) = e
            .call(&alice, "POST", path, Some(job("Bad", schedule, "true")))
            .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{schedule}: {got}"
        );
        assert!(got.to_string().contains(want), "{schedule}: {got}");
    }
    // The command and the timeout.
    for (body, want) in [
        (
            json!({"name": "A", "schedule": "0 * * * *", "command": "   "}),
            "required",
        ),
        (
            json!({"name": "A", "schedule": "0 * * * *", "command": "x".repeat(1001)}),
            "too_long",
        ),
        (
            json!({"name": "A", "schedule": "0 * * * *", "command": "true", "timeout_seconds": 99999}),
            "out_of_range",
        ),
        (
            json!({"name": "", "schedule": "0 * * * *", "command": "true"}),
            "invalid",
        ),
    ] {
        let (status, got) = e.call(&alice, "POST", path, Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {got}");
        assert!(got.to_string().contains(want), "{body}: {got}");
    }
    // A name is used once in an account, but two accounts may use it.
    let (status, got) = e
        .call(
            &alice,
            "POST",
            path,
            Some(job("Cache", "0 3 * * *", "true")),
        )
        .await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "name_taken"));
    let (status, _) = e
        .call(&bob, "POST", path, Some(job("Cache", "0 3 * * *", "true")))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // Bob cannot see or touch Alice's job.
    let one = format!("{path}/{id}");
    for method in ["GET", "DELETE"] {
        let (status, _) = e.call(&bob, method, &one, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
    }
    let (status, _) = e
        .call(&bob, "PUT", &one, Some(job("Cache", "0 3 * * *", "true")))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = e.call(&alice, "GET", path, None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 2);

    // The setting changes the rule: with one minute allowed, every minute is fine.
    let mut tx = pool.begin().await.unwrap();
    builtpanel_settings::set(
        &e.settings,
        &mut tx,
        "cron",
        Scope::Global,
        json!({"min_interval_minutes": 1}).as_object().unwrap(),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let (status, _) = e
        .call(
            &alice,
            "POST",
            path,
            Some(job("Every", "* * * * *", "true")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // Change and switch off; the version moves so that the node sees it.
    let (status, changed) = e.call(&alice, "PUT", &one, Some(json!({"name": "Cache", "schedule": "30 4 * * *", "command": "true", "enabled": false}))).await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(
        (changed["enabled"].clone(), changed["schedule"].clone()),
        (json!(false), json!("30 4 * * *"))
    );
    let version: i64 = sqlx::query_scalar("SELECT version FROM cron_jobs WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(version, 2);

    // The node was told: the whole list, each time, and a job can be run at once.
    let syncs: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'sync_cron_jobs' ORDER BY created_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(syncs.len() >= 5, "{}", syncs.len());
    let last = syncs.last().unwrap();
    assert_eq!(last["jobs"].as_array().unwrap().len(), 3, "{last}");
    assert_eq!(last["username"], "alice");
    let (status, _) = e.call(&alice, "POST", &format!("{one}/run"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let run: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'run_cron_job'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(run["job"]["id"].as_str().unwrap(), id);
    let (status, _) = e.call(&bob, "POST", &format!("{one}/run"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // What the node reports becomes the job's last result (and only if it is newer).
    let node: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE id = $1")
        .bind(alice_tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    let report = |at: i64, status: &str, code: i32, output: &str| {
        json!({"type": "cron_runs", "runs": [
            {"job_id": id, "tenant_id": alice_tenant, "started_at": at, "status": status, "exit_code": code, "output": output},
            {"job_id": id, "tenant_id": alice_tenant, "started_at": at + 1, "status": "skipped", "exit_code": null, "output": "still going"}
        ]})
    };
    for (at, status, code, output) in [
        (1_800_000_100i64, "failed", 2, "boom"),
        (1_800_000_000, "ok", 0, "older, must not win"),
    ] {
        let op: Uuid = sqlx::query_scalar(
            "INSERT INTO operations (kind, node_id, payload, idempotency_key, status, output, expires_at)
             VALUES ('report_cron_runs', $1, '{}', $2, 'dispatched', NULL, now() + interval '1 hour') RETURNING id",
        )
        .bind(node)
        .bind(format!("cronruns:{at}"))
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE operations SET status = 'succeeded', output = $2, finished_at = now() WHERE id = $1")
            .bind(op)
            .bind(report(at, status, code, output))
            .execute(&pool)
            .await
            .unwrap();
    }
    let (st, ex, out): (Option<String>, Option<i32>, Option<String>) = sqlx::query_as(
        "SELECT last_status, last_exit_code, last_output FROM cron_jobs WHERE id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (st.as_deref(), ex, out.as_deref()),
        (Some("failed"), Some(2), Some("boom"))
    );
    let (_, shown) = e.call(&alice, "GET", &one, None).await;
    assert_eq!(shown["last_status"], "failed");

    // The plan's limit.
    sqlx::query("UPDATE plans SET limits = jsonb_set(limits, '{max_cron_jobs}', '3') WHERE id = (SELECT plan_id FROM tenants WHERE id = $1)")
        .bind(alice_tenant)
        .execute(&pool)
        .await
        .unwrap();
    let (status, got) = e
        .call(
            &alice,
            "POST",
            path,
            Some(job("Fourth", "0 5 * * *", "true")),
        )
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "limit_reached"),
        "{got}"
    );
    let (_, list) = e.call(&alice, "GET", path, None).await;
    assert_eq!(list["limit"], 3);

    let (status, _) = e.call(&alice, "DELETE", &one, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = e.call(&alice, "GET", &one, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A plan without the feature has none.
    sqlx::query("UPDATE feature_sets SET all_features = false, features = ARRAY['files']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)").execute(&pool).await.unwrap();
    let (status, got) = e.call(&alice, "GET", path, None).await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::FORBIDDEN, "feature_disabled")
    );
}
