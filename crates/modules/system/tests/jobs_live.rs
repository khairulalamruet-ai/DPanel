//! The panel's background jobs (Server: Scheduled actions and Tasks), against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_cache::RedisHandle;
use builtpanel_database::rbac::assign_role;
use builtpanel_kernel::{ModuleMount, Registry};
use builtpanel_mod_system::{
    SystemState,
    jobs::{Spec, spawn},
    router,
};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use builtpanel_settings::SettingsRegistry;
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

struct Browser {
    cookie: String,
    csrf: String,
}

struct Env {
    app: Router,
    sessions: SessionStore,
}

fn env(pool: &PgPool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let base = Router::new().fallback(|| async { StatusCode::NOT_FOUND });
    let mount = ModuleMount::new(Registry::build(vec![]).unwrap(), base);
    let state = SystemState {
        db: pool.clone(),
        sessions: sessions.clone(),
        mount,
        settings: Arc::new(SettingsRegistry::build(vec![]).unwrap()),
        branding: Arc::default(),
    };
    Env {
        app: router().with_state(state),
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

    async fn send(
        &self,
        who: &Browser,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, &who.cookie)
            .header(CSRF_HEADER, &who.csrf);
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

    async fn job(&self, who: &Browser, key: &str) -> Value {
        let (_, list) = self
            .send(who, "GET", "/api/v1/admin/system/jobs", None)
            .await;
        list.as_array()
            .unwrap()
            .iter()
            .find(|j| j["key"] == key)
            .cloned()
            .unwrap_or_else(|| panic!("no job {key} in {list}"))
    }
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

fn spec(key: &'static str) -> Spec {
    Spec {
        key,
        title: "A test job",
        description: "Counts its runs.",
        default_interval: Duration::from_secs(3600),
        min_interval: Duration::from_secs(60),
    }
}

async fn until(what: &str, mut ok: impl AsyncFnMut() -> bool) {
    for _ in 0..60 {
        if ok().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for {what}");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_job_runs_when_due_can_be_changed_run_at_once_and_reports_failures(pool: PgPool) {
    let e = env(&pool);
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;

    let runs = Arc::new(AtomicU32::new(0));
    let fail = Arc::new(AtomicU32::new(0));
    spawn(pool.clone(), spec("test_counter"), {
        let (runs, fail) = (runs.clone(), fail.clone());
        move || {
            let (runs, fail) = (runs.clone(), fail.clone());
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                if fail.load(Ordering::SeqCst) == 1 {
                    return Err("the thing broke".to_owned());
                }
                if fail.load(Ordering::SeqCst) == 2 {
                    panic!("boom");
                }
                Ok(())
            }
        }
    });

    // It registers itself and, never having run, is due at once.
    until("the first run", async || runs.load(Ordering::SeqCst) == 1).await;
    until("the first run to be recorded", async || {
        e.job(&root, "test_counter").await["run_count"] == 1
    })
    .await;
    let j = e.job(&root, "test_counter").await;
    assert_eq!(
        (
            j["interval_seconds"].as_i64(),
            j["min_interval_seconds"].as_i64(),
            j["enabled"].as_bool(),
            j["running"].as_bool(),
            j["last_status"].as_str()
        ),
        (Some(3600), Some(60), Some(true), Some(false), Some("ok")),
        "{j}"
    );
    assert!(j["next_run_at"].is_string(), "{j}");
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1, "not due for an hour");

    // Only a manager changes it, within what the job allows.
    let path = "/api/v1/admin/system/jobs/test_counter";
    let (status, body) = e
        .send(&plain, "PUT", path, Some(json!({"enabled": false})))
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );
    for bad in [5, 59, 30 * 24 * 3600 + 1] {
        let (status, body) = e
            .send(&root, "PUT", path, Some(json!({"interval_seconds": bad})))
            .await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
            "{bad}"
        );
    }
    let (status, body) = e
        .send(
            &root,
            "PUT",
            "/api/v1/admin/system/jobs/nope",
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!((status, code(&body)), (StatusCode::NOT_FOUND, "not_found"));
    let (status, body) = e
        .send(&root, "PUT", path, Some(json!({"interval_seconds": 600})))
        .await;
    assert_eq!(
        (status, body["interval_seconds"].as_i64()),
        (StatusCode::OK, Some(600)),
        "{body}"
    );
    let (_, body) = e
        .send(&root, "PUT", path, Some(json!({"reset_interval": true})))
        .await;
    assert_eq!(body["interval_seconds"], 3600);

    // A run at once, also while the job is switched off.
    let (status, _) = e
        .send(&root, "PUT", path, Some(json!({"enabled": false})))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = e.send(&root, "POST", &format!("{path}/run"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    until("the manual run", async || runs.load(Ordering::SeqCst) == 2).await;

    // A failure and a panic are told, not hidden, and the job goes on afterwards.
    fail.store(1, Ordering::SeqCst);
    e.send(&root, "POST", &format!("{path}/run"), None).await;
    until("the failed run", async || {
        e.job(&root, "test_counter").await["last_status"] == "error"
    })
    .await;
    let j = e.job(&root, "test_counter").await;
    assert_eq!(j["last_error"], "the thing broke", "{j}");
    fail.store(2, Ordering::SeqCst);
    e.send(&root, "POST", &format!("{path}/run"), None).await;
    until("the panicked run", async || {
        runs.load(Ordering::SeqCst) == 4
    })
    .await;
    until("the panic to be recorded", async || {
        e.job(&root, "test_counter").await["last_error"] == "the job stopped unexpectedly"
    })
    .await;

    let (status, history) = e.send(&root, "GET", &format!("{path}/runs"), None).await;
    assert_eq!(status, StatusCode::OK);
    let history = history.as_array().unwrap();
    assert_eq!(history.len(), 4, "{history:?}");
    assert_eq!(history[0]["ok"], false);
    assert_eq!(history[1]["message"], "the thing broke");
    assert_eq!(history[2]["manual"], true);
    assert_eq!(history[3]["manual"], false, "the first run was by schedule");

    // The task list: a job that is on is scheduled, one that is off is not listed, and the operations
    // waiting for a node show.
    let (_, tasks) = e
        .send(&root, "GET", "/api/v1/admin/system/tasks", None)
        .await;
    assert!(
        tasks
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"] != "A test job"),
        "switched off: {tasks}"
    );
    e.send(&root, "PUT", path, Some(json!({"enabled": true})))
        .await;
    let (_, tasks) = e
        .send(&root, "GET", "/api/v1/admin/system/tasks", None)
        .await;
    let mine = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "A test job")
        .unwrap_or_else(|| panic!("{tasks}"));
    assert_eq!(
        (mine["kind"].as_str(), mine["status"].as_str()),
        (Some("job"), Some("scheduled"))
    );
    let node: Uuid =
        sqlx::query_scalar("INSERT INTO nodes (name, status) VALUES ('n1', 'online') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO operations (kind, node_id, payload, idempotency_key, expires_at)
         VALUES ('create_backup', $1, '{}', 'k1', now() + interval '1 hour')",
    )
    .bind(node)
    .execute(&pool)
    .await
    .unwrap();
    let (_, tasks) = e
        .send(&root, "GET", "/api/v1/admin/system/tasks", None)
        .await;
    let op = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "operation")
        .unwrap_or_else(|| panic!("{tasks}"));
    assert_eq!(
        (
            op["name"].as_str(),
            op["status"].as_str(),
            op["node"].as_str()
        ),
        (Some("create_backup"), Some("waiting"), Some("n1"))
    );
    // Reading needs only the read permission.
    let (status, _) = e
        .send(&plain, "GET", "/api/v1/admin/system/tasks", None)
        .await;
    assert_ne!(status, StatusCode::UNAUTHORIZED);
}
