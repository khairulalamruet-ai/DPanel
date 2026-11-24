//! `Require<P>` against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use axum::{
    Router,
    body::Body,
    extract::FromRef,
    http::{Request, StatusCode, header},
    routing::get,
};
use builtpanel_cache::RedisHandle;
use builtpanel_database::rbac::{AssignRole, assign_role, revoke_role};
use builtpanel_rbac::{Principal, Require, permission, permissions};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

permission!(
    /// A key no built-in role names explicitly.
    DnsZonesWrite = "dns.zones.write"
);

#[derive(Clone)]
struct State {
    sessions: SessionStore,
    db: PgPool,
}
impl FromRef<State> for SessionStore {
    fn from_ref(s: &State) -> Self {
        s.sessions.clone()
    }
}
impl FromRef<State> for PgPool {
    fn from_ref(s: &State) -> Self {
        s.db.clone()
    }
}

async fn read_roles(Require(who, ..): Require<permissions::RolesRead>) -> String {
    who.roles.join(",")
}
async fn manage_roles(Require(who, ..): Require<permissions::RolesManage>) -> String {
    who.account_id().to_string()
}
async fn write_dns(_: Require<DnsZonesWrite>) -> &'static str {
    "ok"
}
async fn whoami(who: Principal) -> String {
    who.permissions.grants().join(",")
}

fn app(pool: &PgPool) -> (Router, SessionStore) {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let router = Router::new()
        .route("/roles", get(read_roles).post(manage_roles))
        .route("/dns", get(write_dns))
        .route("/who", get(whoami))
        .with_state(State {
            sessions: sessions.clone(),
            db: pool.clone(),
        });
    (router, sessions)
}

async fn account(pool: &PgPool, name: &str, kind: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ($1::account_kind, $2, $2 || '@example.com', $3) RETURNING id",
    )
    .bind(kind)
    .bind(name)
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// (cookie header, csrf token)
async fn sign_in(sessions: &SessionStore, account_id: Uuid) -> (String, String) {
    let (token, data) = sessions
        .create(NewSession {
            account_id,
            ip: None,
            user_agent: None,
        })
        .await
        .unwrap();
    let cookie = set_cookie(&token).split(';').next().unwrap().to_owned();
    (cookie, data.csrf_token.expose().to_owned())
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    auth: Option<&(String, String)>,
    csrf: bool,
) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some((cookie, token)) = auth {
        req = req.header(header::COOKIE, cookie);
        if csrf {
            req = req.header(CSRF_HEADER, token);
        }
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn missing_permission_is_403_and_missing_session_is_401(pool: PgPool) {
    let (app, sessions) = app(&pool);
    assert_eq!(
        call(&app, "GET", "/roles", None, false).await.0,
        StatusCode::UNAUTHORIZED
    );

    // An account without roles is authenticated but may do nothing.
    let nobody = account(&pool, "norole", "client").await;
    let auth = sign_in(&sessions, nobody).await;
    let (status, body) = call(&app, "GET", "/roles", Some(&auth), false).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains(r#""code":"permission_denied""#), "{body}");
    assert!(
        !body.contains("admin.roles"),
        "the required key is not disclosed: {body}"
    );
    assert_eq!(
        call(&app, "GET", "/who", Some(&auth), false).await,
        (StatusCode::OK, String::new())
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn built_in_roles_grant_what_they_should(pool: PgPool) {
    let (app, sessions) = app(&pool);
    let admin = account(&pool, "anna", "admin").await;
    let root = account(&pool, "rootuser", "admin").await;
    let helper = account(&pool, "helper", "admin").await;
    assert_eq!(
        assign_role(&pool, admin, "admin", None).await.unwrap(),
        AssignRole::Assigned
    );
    assert_eq!(
        assign_role(&pool, root, "super_admin", Some(admin))
            .await
            .unwrap(),
        AssignRole::Assigned
    );
    assert_eq!(
        assign_role(&pool, helper, "support", None).await.unwrap(),
        AssignRole::Assigned
    );
    assert_eq!(
        assign_role(&pool, helper, "support", None).await.unwrap(),
        AssignRole::AlreadyAssigned
    );
    assert_eq!(
        assign_role(&pool, helper, "no_such_role", None)
            .await
            .unwrap(),
        AssignRole::UnknownRole
    );

    // admin: may read roles, may not manage them.
    let a = sign_in(&sessions, admin).await;
    assert_eq!(
        call(&app, "GET", "/roles", Some(&a), false).await,
        (StatusCode::OK, "admin".to_owned())
    );
    let (status, body) = call(&app, "POST", "/roles", Some(&a), true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("permission_denied"), "{body}");
    // CSRF is checked before permissions.
    let (_, body) = call(&app, "POST", "/roles", Some(&a), false).await;
    assert!(body.contains("csrf_failed"), "{body}");

    // super_admin: everything, including keys no role lists.
    let r = sign_in(&sessions, root).await;
    assert_eq!(
        call(&app, "POST", "/roles", Some(&r), true).await,
        (StatusCode::OK, root.to_string())
    );
    assert_eq!(
        call(&app, "GET", "/dns", Some(&r), false).await.0,
        StatusCode::OK
    );
    assert_eq!(call(&app, "GET", "/who", Some(&r), false).await.1, "*");

    // support: audit only.
    let h = sign_in(&sessions, helper).await;
    assert_eq!(
        call(&app, "GET", "/roles", Some(&h), false).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "GET", "/who", Some(&h), false).await.1,
        "admin.audit.read,admin.nodes.read"
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn role_changes_apply_to_the_next_request(pool: PgPool) {
    let (app, sessions) = app(&pool);
    let user = account(&pool, "dana", "admin").await;
    let auth = sign_in(&sessions, user).await;
    assert_eq!(
        call(&app, "GET", "/roles", Some(&auth), false).await.0,
        StatusCode::FORBIDDEN
    );

    assign_role(&pool, user, "admin", None).await.unwrap();
    assert_eq!(
        call(&app, "GET", "/roles", Some(&auth), false).await.0,
        StatusCode::OK
    );

    assert!(revoke_role(&pool, user, "admin").await.unwrap());
    assert!(!revoke_role(&pool, user, "admin").await.unwrap());
    assert_eq!(
        call(&app, "GET", "/roles", Some(&auth), false).await.0,
        StatusCode::FORBIDDEN
    );

    // A custom role with a subtree grant, and two roles adding up.
    let role: Uuid =
        sqlx::query_scalar("INSERT INTO roles (name) VALUES ('dns_operator') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO role_permissions (role_id, permission) VALUES ($1, 'dns.zones.*')")
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    assign_role(&pool, user, "dns_operator", None)
        .await
        .unwrap();
    assign_role(&pool, user, "support", None).await.unwrap();
    assert_eq!(
        call(&app, "GET", "/dns", Some(&auth), false).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "GET", "/roles", Some(&auth), false).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "GET", "/who", Some(&auth), false).await.1,
        "admin.audit.read,admin.nodes.read,dns.zones.*"
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn inactive_or_deleted_accounts_lose_access_and_their_session(pool: PgPool) {
    let (app, sessions) = app(&pool);
    let user = account(&pool, "erin", "admin").await;
    assign_role(&pool, user, "super_admin", None).await.unwrap();
    let auth = sign_in(&sessions, user).await;
    assert_eq!(
        call(&app, "GET", "/roles", Some(&auth), false).await.0,
        StatusCode::OK
    );

    sqlx::query("UPDATE accounts SET status = 'suspended' WHERE id = $1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "GET", "/roles", Some(&auth), false).await.0,
        StatusCode::UNAUTHORIZED
    );
    // The session was ended: re-activating the account does not bring it back.
    sqlx::query("UPDATE accounts SET status = 'active' WHERE id = $1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "GET", "/roles", Some(&auth), false).await.0,
        StatusCode::UNAUTHORIZED
    );

    let auth = sign_in(&sessions, user).await;
    sqlx::query("DELETE FROM accounts WHERE id = $1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "GET", "/who", Some(&auth), false).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn schema_seeds_built_in_roles_and_rejects_malformed_grants(pool: PgPool) {
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM roles WHERE built_in ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        names,
        ["admin", "client", "reseller", "super_admin", "support"]
    );
    let role: Uuid = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'client'")
        .fetch_one(&pool)
        .await
        .unwrap();
    for bad in ["admin", "Admin.x", "admin.*.read", "**", "", "admin.x "] {
        let r = sqlx::query("INSERT INTO role_permissions (role_id, permission) VALUES ($1, $2)")
            .bind(role)
            .bind(bad)
            .execute(&pool)
            .await;
        assert!(r.is_err(), "{bad:?} must be rejected");
    }
    for ok in ["*", "dns.*", "dns.zones.read"] {
        sqlx::query("INSERT INTO role_permissions (role_id, permission) VALUES ($1, $2)")
            .bind(role)
            .bind(ok)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    assert!(
        sqlx::query("INSERT INTO roles (name) VALUES ('Bad Name')")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO roles (name) VALUES ('admin')")
            .execute(&pool)
            .await
            .is_err()
    );
}
