//! Audit log against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use axum::{
    Router,
    body::Body,
    extract::{FromRef, State, connect_info::MockConnectInfo},
    http::{Request, StatusCode, header},
    routing::post,
};
use builtpanel_audit::{AuditEntry, AuditWriter, Outcome, insert};
use builtpanel_cache::RedisHandle;
use builtpanel_database::rbac::assign_role;
use builtpanel_http_kit::{client_ip::client_ip_layer, request_id::correlation_id_layer};
use builtpanel_session::{NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

#[derive(Clone)]
struct AppState {
    sessions: SessionStore,
    db: PgPool,
}
impl FromRef<AppState> for SessionStore {
    fn from_ref(s: &AppState) -> Self {
        s.sessions.clone()
    }
}
impl FromRef<AppState> for PgPool {
    fn from_ref(s: &AppState) -> Self {
        s.db.clone()
    }
}

/// A handler that audits something, to see what the request scope fills in.
async fn act(State(db): State<PgPool>) -> StatusCode {
    insert(&db, &AuditEntry::new("test.acted", Outcome::Success))
        .await
        .unwrap();
    StatusCode::NO_CONTENT
}

fn build(pool: &PgPool, peer: &str, trusted: &[IpAddr]) -> (Router, SessionStore) {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let trusted: Arc<[IpAddr]> = trusted.to_vec().into();
    let router = builtpanel_audit::router::<AppState>()
        .route("/act", post(act))
        .with_state(AppState {
            sessions: sessions.clone(),
            db: pool.clone(),
        })
        .layer(axum::middleware::from_fn_with_state(
            trusted,
            client_ip_layer,
        ))
        .layer(axum::middleware::from_fn(correlation_id_layer))
        .layer(MockConnectInfo(SocketAddr::new(
            peer.parse().unwrap(),
            5000,
        )));
    (router, sessions)
}

async fn account(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', $1, $1 || '@example.com', $2) RETURNING id",
    )
    .bind(name)
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn cookie(sessions: &SessionStore, account_id: Uuid) -> String {
    let (token, _) = sessions
        .create(NewSession {
            account_id,
            ip: None,
            user_agent: None,
        })
        .await
        .unwrap();
    set_cookie(&token).split(';').next().unwrap().to_owned()
}

async fn get(app: &Router, path: &str, cookie: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().uri(path);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn sqlstate(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_log_cannot_be_updated_deleted_or_truncated(pool: PgPool) {
    let writer = AuditWriter::new(pool.clone());
    let actor = Uuid::now_v7();
    writer
        .record(
            &AuditEntry::new("admin.role_assigned", Outcome::Success)
                .actor(actor)
                .target("role", "admin"),
        )
        .await
        .unwrap();

    // Even this connection (a superuser, and owner of the table) is refused.
    for statement in [
        "UPDATE audit_logs SET outcome = 'failure'",
        "UPDATE audit_logs SET actor_id = NULL",
        "DELETE FROM audit_logs",
        "TRUNCATE audit_logs",
    ] {
        let err = sqlx::query(statement)
            .execute(&pool)
            .await
            .expect_err(statement);
        assert!(
            err.to_string().contains("append-only"),
            "{statement}: {err}"
        );
        assert_eq!(sqlstate(&err).as_deref(), Some("42501"), "{statement}");
    }
    // Also inside a transaction, and with a WHERE clause that matches nothing.
    let mut tx = pool.begin().await.unwrap();
    assert!(
        sqlx::query("DELETE FROM audit_logs WHERE false")
            .execute(&mut *tx)
            .await
            .is_ok()
    );
    assert!(
        sqlx::query("DELETE FROM audit_logs")
            .execute(&mut *tx)
            .await
            .is_err()
    );
    drop(tx);

    let (action, outcome, actor_id): (String, String, Option<Uuid>) =
        sqlx::query_as("SELECT action, outcome, actor_id FROM audit_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (action.as_str(), outcome.as_str(), actor_id),
        ("admin.role_assigned", "success", Some(actor))
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_database_rejects_malformed_rows(pool: PgPool) {
    let bad = [
        (
            "INSERT INTO audit_logs (action, outcome) VALUES ('login', 'success')",
            "single-segment action",
        ),
        (
            "INSERT INTO audit_logs (action, outcome) VALUES ('Auth.Login', 'success')",
            "upper case action",
        ),
        (
            "INSERT INTO audit_logs (action, outcome) VALUES ('auth.login', 'maybe')",
            "unknown outcome",
        ),
        (
            "INSERT INTO audit_logs (action, outcome, metadata) VALUES ('auth.login', 'success', '[1]')",
            "array metadata",
        ),
        (
            "INSERT INTO audit_logs (action, outcome, metadata) VALUES ('auth.login', 'success', '\"s\"')",
            "string metadata",
        ),
    ];
    for (sql, why) in bad {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{why}");
    }
    let huge = json!({ "big": "x".repeat(9000) });
    let r = sqlx::query(
        "INSERT INTO audit_logs (action, outcome, metadata) VALUES ('auth.login', 'success', $1)",
    )
    .bind(huge)
    .execute(&pool)
    .await;
    assert!(r.is_err(), "oversized metadata");

    // The writer never trips over that limit: it stores a marker instead.
    let writer = AuditWriter::new(pool.clone());
    writer
        .record(
            &AuditEntry::new("auth.login", Outcome::Success)
                .metadata(json!({ "big": "x".repeat(9000) })),
        )
        .await
        .unwrap();
    writer
        .record(
            &AuditEntry::new("auth.login", Outcome::Success)
                .metadata(json!(["not", "an", "object"])),
        )
        .await
        .unwrap();
    let stored: Vec<Value> = sqlx::query_scalar("SELECT metadata FROM audit_logs ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        stored,
        [
            json!({"truncated": true}),
            json!({"value": ["not", "an", "object"]})
        ]
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn entries_pick_up_correlation_id_and_client_ip_from_the_request(pool: PgPool) {
    let proxy: IpAddr = "127.0.0.1".parse().unwrap();
    let (app, _) = build(&pool, "127.0.0.1", &[proxy]);
    let req = Request::builder()
        .method("POST")
        .uri("/act")
        .header("x-request-id", "corr-test-7")
        .header("x-forwarded-for", "6.6.6.6, 203.0.113.44")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );

    // An untrusted peer cannot choose its recorded address.
    let (direct, _) = build(&pool, "198.51.100.9", &[proxy]);
    let req = Request::builder()
        .method("POST")
        .uri("/act")
        .header("x-forwarded-for", "6.6.6.6")
        .body(Body::empty())
        .unwrap();
    direct.oneshot(req).await.unwrap();

    // Outside any request there is nothing to fill in, and an explicit IP wins.
    insert(&pool, &AuditEntry::new("test.outside", Outcome::Success))
        .await
        .unwrap();
    insert(
        &pool,
        &AuditEntry::new("test.explicit", Outcome::Success).actor_ip("192.0.2.1".parse().unwrap()),
    )
    .await
    .unwrap();

    let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT action, host(actor_ip), correlation_id FROM audit_logs ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows[0],
        (
            "test.acted".into(),
            Some("203.0.113.44".into()),
            Some("corr-test-7".into())
        )
    );
    assert_eq!(rows[1].0, "test.acted");
    assert_eq!(rows[1].1.as_deref(), Some("198.51.100.9"));
    assert!(rows[1].2.is_some(), "generated correlation id");
    assert_eq!(rows[2], ("test.outside".into(), None, None));
    assert_eq!(
        rows[3],
        ("test.explicit".into(), Some("192.0.2.1".into()), None)
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_entry_written_in_a_transaction_commits_or_rolls_back_with_it(pool: PgPool) {
    let entry = AuditEntry::new("admin.thing_changed", Outcome::Success);
    let mut tx = pool.begin().await.unwrap();
    insert(&mut *tx, &entry).await.unwrap();
    tx.rollback().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);

    let mut tx = pool.begin().await.unwrap();
    insert(&mut *tx, &entry).await.unwrap();
    tx.commit().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn only_holders_of_audit_read_can_read_the_log(pool: PgPool) {
    let (app, sessions) = build(&pool, "198.51.100.1", &[]);
    let nobody = account(&pool, "nobody").await;
    let support = account(&pool, "support1").await;
    let admin = account(&pool, "admin1").await;
    assign_role(&pool, support, "support", None).await.unwrap();
    assign_role(&pool, admin, "admin", None).await.unwrap();

    let path = "/api/v1/admin/audit-logs";
    assert_eq!(get(&app, path, None).await.0, StatusCode::UNAUTHORIZED);
    let (status, body) = get(&app, path, Some(&cookie(&sessions, nobody).await)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "permission_denied");
    for who in [support, admin] {
        let (status, body) = get(&app, path, Some(&cookie(&sessions, who).await)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["items"], json!([]));
        assert_eq!(body["next_cursor"], Value::Null);
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn listing_filters_and_pages_without_gaps_or_duplicates(pool: PgPool) {
    let (app, sessions) = build(&pool, "198.51.100.1", &[]);
    let reader = account(&pool, "reader").await;
    assign_role(&pool, reader, "support", None).await.unwrap();
    let auth = cookie(&sessions, reader).await;
    let alice = Uuid::now_v7();
    let bob = Uuid::now_v7();

    let writer = AuditWriter::new(pool.clone());
    let plan: [(&'static str, Outcome, Uuid, Option<&str>); 7] = [
        ("auth.login", Outcome::Success, alice, None),
        ("auth.login_failed", Outcome::Failure, bob, None),
        ("auth.logout", Outcome::Success, alice, Some("session-1")),
        (
            "admin.role_assigned",
            Outcome::Success,
            alice,
            Some("admin"),
        ),
        ("auth.login_failed", Outcome::Denied, bob, None),
        ("admin.role_revoked", Outcome::Success, alice, Some("admin")),
        ("auth.login", Outcome::Success, bob, None),
    ];
    for (action, outcome, actor, target) in plan {
        let mut e = AuditEntry::new(action, outcome).actor(actor);
        if let Some(t) = target {
            e = e.target("thing", t);
        }
        writer.record(&e).await.unwrap();
    }

    let ids = |body: &Value| -> Vec<String> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let actions = |body: &Value| -> Vec<String> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["action"].as_str().unwrap().to_owned())
            .collect()
    };
    let list = |query: String| {
        let app = app.clone();
        let auth = auth.clone();
        async move {
            get(
                &app,
                &format!("/api/v1/admin/audit-logs?{query}"),
                Some(&auth),
            )
            .await
        }
    };

    // Everything, newest first.
    let (_, all) = list(String::new()).await;
    assert_eq!(
        actions(&all),
        [
            "auth.login",
            "admin.role_revoked",
            "auth.login_failed",
            "admin.role_assigned",
            "auth.logout",
            "auth.login_failed",
            "auth.login"
        ]
    );
    let all_ids = ids(&all);

    // Walk it two at a time.
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let query = match &cursor {
            Some(next) => format!("limit=2&cursor={next}"),
            None => "limit=2".to_owned(),
        };
        let (status, page) = list(query).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        seen.extend(ids(&page));
        pages += 1;
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
        assert!(pages < 10, "paging did not terminate");
    }
    assert_eq!(pages, 4);
    assert_eq!(
        seen, all_ids,
        "pages cover every entry exactly once, in order"
    );

    // Filters.
    let (_, b) = list("action=auth.login_failed".into()).await;
    assert_eq!(actions(&b), ["auth.login_failed", "auth.login_failed"]);
    let (_, b) = list("action=admin.".into()).await;
    assert_eq!(actions(&b), ["admin.role_revoked", "admin.role_assigned"]);
    let (_, b) = list(format!("actor_id={bob}")).await;
    assert_eq!(b["items"].as_array().unwrap().len(), 3);
    let (_, b) = list("outcome=denied".into()).await;
    assert_eq!(b["items"].as_array().unwrap().len(), 1);
    let (_, b) = list("target_type=thing&target_id=admin".into()).await;
    assert_eq!(actions(&b), ["admin.role_revoked", "admin.role_assigned"]);
    let (_, b) = list("from=2000-01-01T00:00:00Z&to=2000-01-02T00:00:00Z".into()).await;
    assert_eq!(b["items"], json!([]));
    let (_, b) = list("from=2000-01-01T00:00:00Z&to=2999-01-01T00:00:00Z&limit=1".into()).await;
    assert_eq!(b["items"].as_array().unwrap().len(), 1);
    // `%` and `_` are not wildcards: such input is simply not a valid action prefix.
    let (status, _) = list("action=auth%25".into()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Bad input is a 422 that names the fields.
    let (status, body) = list("limit=0&outcome=maybe&cursor=oops&actor_id=x".into()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let fields: Vec<&str> = body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["actor_id", "outcome", "limit", "cursor"]);
}
