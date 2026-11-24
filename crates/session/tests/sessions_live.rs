//! Session store against real PostgreSQL and Redis.
//! Ignored by default. Run with DATABASE_URL (superuser, CREATEDB) and REDIS_URL:
//! `eval "$(deploy/dev/devenv.sh test-env)" && cargo test -p builtpanel-session -- --include-ignored`
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::Duration;

use axum::{
    Router,
    body::Body,
    extract::FromRef,
    http::{Request, StatusCode, header},
    routing::{get, post},
};
use builtpanel_cache::RedisHandle;
use builtpanel_session::{
    Authenticated, CSRF_HEADER, MAX_USER_AGENT, NewSession, RecentAuth, RevokeReason,
    SessionConfig, SessionStore, set_cookie,
};
use secrecy::SecretString;
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

fn redis() -> RedisHandle {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL")
}

async fn account(pool: &PgPool) -> Uuid {
    let n = Uuid::now_v7().simple().to_string();
    sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash) VALUES ('client', $1, $2, $3) RETURNING id",
    )
    .bind(format!("u{}", &n[..20]))
    .bind(format!("u{}@example.com", &n[..20]))
    .bind(HASH)
    .fetch_one(pool)
    .await
    .expect("account")
}

fn store(pool: &PgPool, idle: u64, absolute: u64, touch: u64) -> SessionStore {
    SessionStore::new(
        redis(),
        pool.clone(),
        SessionConfig {
            idle_timeout: Duration::from_secs(idle),
            absolute_timeout: Duration::from_secs(absolute),
            touch_interval: Duration::from_secs(touch),
            ..SessionConfig::default()
        },
    )
}

fn new_session(account_id: Uuid) -> NewSession {
    NewSession {
        account_id,
        ip: Some("203.0.113.5".parse().unwrap()),
        user_agent: Some("Mozilla/5.0 test".to_owned()),
    }
}

#[derive(Clone)]
struct State {
    sessions: SessionStore,
}
impl FromRef<State> for SessionStore {
    fn from_ref(s: &State) -> Self {
        s.sessions.clone()
    }
}

async fn whoami(Authenticated(s): Authenticated) -> String {
    s.account_id.to_string()
}

async fn call(app: Router, cookie: Option<&str>) -> (StatusCode, String) {
    let mut req = Request::builder().uri("/me");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn cookie_header(set_cookie_value: &str) -> String {
    set_cookie_value.split(';').next().unwrap().to_owned()
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn create_then_load_round_trips_and_records_history(pool: PgPool) {
    let acc = account(&pool).await;
    let s = store(&pool, 60, 3600, 60);
    let (token, data) = s.create(new_session(acc)).await.expect("create");
    assert_eq!(data.account_id, acc);
    assert_eq!(token.expose().len(), 43);

    let loaded = s
        .load(token.expose())
        .await
        .expect("load")
        .expect("present");
    assert_eq!(loaded, data);

    let (ip, ua, has_hash): (String, String, bool) = sqlx::query_as(
        "SELECT host(ip), user_agent, octet_length(token_sha256) = 32 FROM sessions WHERE id = $1",
    )
    .bind(data.id)
    .fetch_one(&pool)
    .await
    .expect("history row");
    assert_eq!(
        (ip.as_str(), ua.as_str(), has_hash),
        ("203.0.113.5", "Mozilla/5.0 test", true)
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn idle_timeout_expires_session_and_extractor_returns_401(pool: PgPool) {
    let acc = account(&pool).await;
    let s = store(&pool, 1, 3600, 60);
    let (token, _) = s.create(new_session(acc)).await.expect("create");
    let app = Router::new().route("/me", get(whoami)).with_state(State {
        sessions: s.clone(),
    });
    let cookie = cookie_header(&set_cookie(&token));

    let (status, body) = call(app.clone(), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, acc.to_string());

    tokio::time::sleep(Duration::from_millis(2300)).await;
    let (status, body) = call(app, Some(&cookie)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains(r#""code":"unauthenticated""#), "{body}");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn absolute_timeout_wins_over_activity(pool: PgPool) {
    let acc = account(&pool).await;
    // Idle window is long and every load refreshes it, but the absolute limit is 8s.
    // Timing is measured, not assumed, so a heavily loaded machine cannot make
    // the "still alive" checks run after the absolute deadline.
    let s = store(&pool, 60, 8, 0);
    let before_create = std::time::Instant::now();
    let (token, _) = s.create(new_session(acc)).await.expect("create");
    let after_create = std::time::Instant::now();
    let mut alive_checks = 0;
    for pause in [0, 1000, 1000] {
        tokio::time::sleep(Duration::from_millis(pause)).await;
        let loaded = s.load(token.expose()).await.expect("load");
        // The session is at most `before_create.elapsed()` old.
        if before_create.elapsed() < Duration::from_millis(7000) {
            assert!(loaded.is_some(), "activity inside the absolute window");
            alive_checks += 1;
        }
    }
    assert!(
        alive_checks > 0,
        "machine too slow to observe the live session"
    );
    // The session is at least `after_create.elapsed()` old.
    let past_deadline = Duration::from_millis(9500);
    tokio::time::sleep(past_deadline.saturating_sub(after_create.elapsed())).await;
    assert!(s.load(token.expose()).await.expect("load").is_none());
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn activity_slides_the_idle_window(pool: PgPool) {
    let acc = account(&pool).await;
    let s = store(&pool, 2, 3600, 0);
    let (token, _) = s.create(new_session(acc)).await.expect("create");
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            s.load(token.expose()).await.expect("load").is_some(),
            "refreshed by activity"
        );
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn revoke_ends_session_and_marks_history(pool: PgPool) {
    let acc = account(&pool).await;
    let s = store(&pool, 60, 3600, 60);
    let (token, data) = s.create(new_session(acc)).await.expect("create");
    assert!(
        s.revoke(token.expose(), RevokeReason::Logout)
            .await
            .expect("revoke")
    );
    assert!(s.load(token.expose()).await.expect("load").is_none());
    assert!(
        !s.revoke(token.expose(), RevokeReason::Logout)
            .await
            .expect("second revoke")
    );

    let reason: Option<String> =
        sqlx::query_scalar("SELECT revoke_reason FROM sessions WHERE id = $1")
            .bind(data.id)
            .fetch_one(&pool)
            .await
            .expect("row");
    assert_eq!(reason.as_deref(), Some("logout"));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn missing_forged_or_unknown_cookies_are_401(pool: PgPool) {
    let s = store(&pool, 60, 3600, 60);
    let app = Router::new()
        .route("/me", get(whoami))
        .with_state(State { sessions: s });
    let forged = "__Host-bp_session=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    for cookie in [
        None,
        Some("__Host-bp_session=garbage"),
        Some(forged),
        Some("bp_session=x"),
    ] {
        let (status, _) = call(app.clone(), cookie).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{cookie:?}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn long_multibyte_user_agent_is_truncated_not_dropped(pool: PgPool) {
    let acc = account(&pool).await;
    let s = store(&pool, 60, 3600, 60);
    let mut new = new_session(acc);
    new.user_agent = Some("ব".repeat(MAX_USER_AGENT + 50));
    let (_, data) = s.create(new).await.expect("create");
    let len: i32 = sqlx::query_scalar("SELECT length(user_agent) FROM sessions WHERE id = $1")
        .bind(data.id)
        .fetch_one(&pool)
        .await
        .expect("row");
    assert_eq!(len, i32::try_from(MAX_USER_AGENT).unwrap());
}

async fn post_act(app: Router, cookie: Option<&str>, csrf: Option<&str>) -> (StatusCode, String) {
    let mut req = Request::builder().method("POST").uri("/act");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    if let Some(t) = csrf {
        req = req.header(CSRF_HEADER, t);
    }
    let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn state_changing_requests_need_the_session_csrf_token(pool: PgPool) {
    let acc = account(&pool).await;
    let s = store(&pool, 60, 3600, 60);
    let (token, data) = s.create(new_session(acc)).await.expect("create");
    let (other_token, other) = s.create(new_session(acc)).await.expect("second session");
    assert_ne!(data.csrf_token, other.csrf_token);
    let app = Router::new()
        .route("/me", get(whoami))
        .route("/act", post(whoami))
        .with_state(State { sessions: s });
    let cookie = cookie_header(&set_cookie(&token));
    let csrf = data.csrf_token.expose();

    // Safe methods need no token.
    assert_eq!(call(app.clone(), Some(&cookie)).await.0, StatusCode::OK);

    // POST without, with a wrong, or with another session's token: 403.
    for bad in [None, Some("wrong"), Some(other.csrf_token.expose())] {
        let (status, body) = post_act(app.clone(), Some(&cookie), bad).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{bad:?}");
        assert!(body.contains(r#""code":"csrf_failed""#), "{body}");
    }
    // The session's own token: allowed.
    let (status, body) = post_act(app.clone(), Some(&cookie), Some(csrf)).await;
    assert_eq!((status, body), (StatusCode::OK, acc.to_string()));

    // No session at all is still 401, not 403, even with a token.
    assert_eq!(
        post_act(app.clone(), None, Some(csrf)).await.0,
        StatusCode::UNAUTHORIZED
    );
    // The CSRF token survives a reload of the session.
    let other_cookie = cookie_header(&set_cookie(&other_token));
    let (status, _) = post_act(app, Some(&other_cookie), Some(other.csrf_token.expose())).await;
    assert_eq!(status, StatusCode::OK);
}

async fn sensitive(RecentAuth(s): RecentAuth) -> String {
    s.account_id.to_string()
}

async fn get_path(app: Router, path: &str, cookie: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .uri(path)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn sensitive_routes_need_recent_authentication(pool: PgPool) {
    let acc = account(&pool).await;
    let s = SessionStore::new(
        redis(),
        pool.clone(),
        SessionConfig {
            sudo_window: Duration::from_secs(2),
            ..SessionConfig::default()
        },
    );
    let (token, data) = s.create(new_session(acc)).await.expect("create");
    assert!(data.auth_time > 0);
    let app = Router::new()
        .route("/me", get(whoami))
        .route("/sensitive", get(sensitive))
        .with_state(State {
            sessions: s.clone(),
        });
    let cookie = cookie_header(&set_cookie(&token));

    // Fresh login: sudo mode is active.
    assert_eq!(
        get_path(app.clone(), "/sensitive", &cookie).await.0,
        StatusCode::OK
    );

    // After the window: sensitive routes ask for re-authentication, ordinary ones still work.
    tokio::time::sleep(Duration::from_millis(3100)).await;
    let (status, body) = get_path(app.clone(), "/sensitive", &cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains(r#""code":"reauth_required""#), "{body}");
    assert_eq!(
        get_path(app.clone(), "/me", &cookie).await.0,
        StatusCode::OK
    );

    // Re-authentication restarts the window without touching the lifetime.
    assert!(s.mark_reauthenticated(token.expose()).await.expect("mark"));
    let reloaded = s
        .load(token.expose())
        .await
        .expect("load")
        .expect("present");
    assert!(reloaded.auth_time > data.auth_time);
    assert_eq!(reloaded.absolute_expires_at, data.absolute_expires_at);
    assert_eq!(reloaded.csrf_token, data.csrf_token);
    assert_eq!(
        get_path(app.clone(), "/sensitive", &cookie).await.0,
        StatusCode::OK
    );

    // No session, no sudo: unauthenticated stays 401, and a revoked session cannot be revived.
    assert_eq!(
        get_path(app.clone(), "/sensitive", "__Host-bp_session=garbage")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    s.revoke(token.expose(), RevokeReason::Logout)
        .await
        .expect("revoke");
    assert!(!s.mark_reauthenticated(token.expose()).await.expect("mark"));
    assert!(s.load(token.expose()).await.expect("load").is_none());
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis: see module docs"]
async fn listing_shows_only_live_sessions_of_the_account(pool: PgPool) {
    let alice = account(&pool).await;
    let bob = account(&pool).await;
    // Idle timeout 1s so one session can lapse in Redis while its history row stays open.
    let short = store(&pool, 1, 3600, 60);
    let long = store(&pool, 600, 3600, 60);

    let (_, lapsing) = short.create(new_session(alice)).await.expect("create");
    let (t1, s1) = long.create(new_session(alice)).await.expect("create");
    let (_, s2) = long.create(new_session(alice)).await.expect("create");
    let (_, other) = long.create(new_session(bob)).await.expect("create");
    tokio::time::sleep(Duration::from_millis(2300)).await;

    let listed = long.list_active(alice).await.expect("list");
    let ids: Vec<Uuid> = listed.iter().map(|s| s.id).collect();
    assert_eq!(
        ids,
        vec![s2.id, s1.id],
        "newest first, lapsed and foreign sessions left out"
    );
    assert!(!ids.contains(&other.id));
    let first = &listed[0];
    assert_eq!(first.ip.as_deref(), Some("203.0.113.5"));
    assert_eq!(first.user_agent.as_deref(), Some("Mozilla/5.0 test"));
    assert!(first.expires_at > first.created_at && first.last_seen_at >= first.created_at);

    // The lapsed session is now closed in the history as expired.
    let reason: Option<String> =
        sqlx::query_scalar("SELECT revoke_reason FROM sessions WHERE id = $1")
            .bind(lapsing.id)
            .fetch_one(&pool)
            .await
            .expect("row");
    assert_eq!(reason.as_deref(), Some("expired"));

    // Revoking by id: only the owner can, and it kills the session immediately.
    assert!(
        !long
            .revoke_by_id(bob, s1.id, RevokeReason::Admin)
            .await
            .expect("foreign")
    );
    assert!(long.load(t1.expose()).await.expect("load").is_some());
    assert!(
        long.revoke_by_id(alice, s1.id, RevokeReason::Logout)
            .await
            .expect("own")
    );
    assert!(long.load(t1.expose()).await.expect("load").is_none());
    assert!(
        !long
            .revoke_by_id(alice, s1.id, RevokeReason::Logout)
            .await
            .expect("twice")
    );

    // Revoke all but one, then all.
    let (_, s3) = long.create(new_session(alice)).await.expect("create");
    assert_eq!(
        long.revoke_all(alice, Some(s3.id), RevokeReason::LogoutAll)
            .await
            .expect("others"),
        1
    );
    assert_eq!(long.list_active(alice).await.expect("list").len(), 1);
    assert_eq!(
        long.revoke_all(alice, None, RevokeReason::LogoutAll)
            .await
            .expect("all"),
        1
    );
    assert!(long.list_active(alice).await.expect("list").is_empty());
    assert_eq!(
        long.list_active(bob).await.expect("list").len(),
        1,
        "bob is untouched"
    );
}
