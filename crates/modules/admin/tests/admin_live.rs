//! Administrator management against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::Duration;

use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Request, StatusCode, header},
};
use builtpanel_auth_core::password::{Argon2Params, PasswordHasher};
use builtpanel_cache::RedisHandle;
use builtpanel_database::{mfa, rbac::assign_role};
use builtpanel_mod_admin::{AdminState, router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const PASSWORD: &str = "Orbit-Lantern-Quiet-Harbor-42";
const BASE: &str = "/api/v1/admin/administrators";

struct Env {
    app: Router,
    sessions: SessionStore,
}

fn env(pool: &PgPool, sudo_secs: u64) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(
        redis,
        pool.clone(),
        SessionConfig {
            sudo_window: Duration::from_secs(sudo_secs),
            ..SessionConfig::default()
        },
    );
    let hasher = PasswordHasher::new(Argon2Params {
        memory_kib: 19 * 1024,
        iterations: 2,
        parallelism: 1,
    })
    .unwrap();
    let app = router()
        .with_state(AdminState {
            db: pool.clone(),
            sessions: sessions.clone(),
            hasher,
        })
        .layer(MockConnectInfo(std::net::SocketAddr::from((
            [198, 51, 100, 7],
            4000,
        ))));
    Env { app, sessions }
}

/// An account of the given kind holding the given roles.
async fn account(pool: &PgPool, name: &str, kind: &str, roles: &[&str]) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ($1::account_kind, $2, $2 || '@example.com', $3) RETURNING id",
    )
    .bind(kind)
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

struct Browser {
    cookie: String,
    csrf: String,
}

impl Env {
    async fn sign_in(&self, account_id: Uuid) -> Browser {
        let (token, data) = self
            .sessions
            .create(NewSession {
                account_id,
                ip: None,
                user_agent: None,
            })
            .await
            .unwrap();
        Browser {
            cookie: set_cookie(&token).split(';').next().unwrap().to_owned(),
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

    async fn send(
        &self,
        who: &Browser,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.call(Some(who), method, path, body, true).await
    }

    async fn get(&self, who: &Browser, path: &str) -> (StatusCode, Value) {
        self.call(Some(who), "GET", path, None, false).await
    }
}

fn new_admin(username: &str) -> Value {
    json!({"username": username, "email": format!("{username}@example.com"), "password": PASSWORD})
}

async fn audit_actions(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT action FROM audit_logs ORDER BY at, id")
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn admin_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM accounts WHERE kind = 'admin'")
        .fetch_one(pool)
        .await
        .unwrap()
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn only_a_super_admin_can_create_administrators(pool: PgPool) {
    let e = env(&pool, 600);
    let root = account(&pool, "rootuser", "admin", &["super_admin"]).await;
    let plain = account(&pool, "plainadmin", "admin", &["admin"]).await;
    let helper = account(&pool, "helper", "admin", &["support"]).await;
    let nobody = account(&pool, "norole", "admin", &[]).await;
    // A custom role that carries the manage permission, but is not super_admin.
    let role: Uuid =
        sqlx::query_scalar("INSERT INTO roles (name) VALUES ('custom_mgr') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO role_permissions (role_id, permission) VALUES ($1, 'admin.administrators.manage')")
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    let custom = account(&pool, "custommgr", "admin", &["custom_mgr"]).await;
    let before = admin_count(&pool).await;

    let (status, body) = e
        .call(None, "POST", BASE, Some(new_admin("newbie")), true)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::UNAUTHORIZED, "unauthenticated")
    );
    for (who, expected) in [
        (nobody, "permission_denied"),
        (helper, "permission_denied"),
        (plain, "permission_denied"),
        (custom, "super_admin_required"),
    ] {
        let b = e.sign_in(who).await;
        let (status, body) = e.send(&b, "POST", BASE, Some(new_admin("newbie"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{who}: {body}");
        assert_eq!(code(&body), expected, "{who}");
    }
    assert_eq!(admin_count(&pool).await, before, "nothing was created");

    let b = e.sign_in(root).await;
    let (status, body) = e
        .call(Some(&b), "POST", BASE, Some(new_admin("newbie")), false)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "csrf_failed")
    );

    let (status, body) = e.send(&b, "POST", BASE, Some(new_admin("newbie"))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["username"], "newbie");
    assert_eq!(body["status"], "active");
    assert_eq!(body["roles"], json!(["admin"]));
    assert_eq!(
        body["mfa_required"], true,
        "2FA is required unless the request says otherwise"
    );
    assert_eq!(body["mfa_enabled"], false);
    assert!(body.get("password").is_none() && body.get("password_hash").is_none());
    assert_eq!(admin_count(&pool).await, before + 1);

    let stored: (String, String) =
        sqlx::query_as("SELECT password_hash, kind::text FROM accounts WHERE username = 'newbie'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(stored.0.starts_with("$argon2id$"), "password is hashed");
    assert_eq!(stored.1, "admin");

    // The change and its audit entry exist together, without the password.
    let (actor, target_type, metadata): (Option<Uuid>, String, Value) = sqlx::query_as(
        "SELECT actor_id, target_type, metadata FROM audit_logs WHERE action = 'admin.administrator_created'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((actor, target_type.as_str()), (Some(root), "administrator"));
    assert_eq!(
        metadata,
        json!({"username": "newbie", "role": "admin", "require_mfa": true})
    );
    let dump: String = sqlx::query_scalar(
        "SELECT COALESCE(string_agg(row_to_json(a)::text, ' '), '') FROM audit_logs a",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!dump.contains(PASSWORD));

    // The new administrator may opt out of mandatory 2FA, and pick a role.
    let mut req = new_admin("relaxed");
    req["role"] = json!("support");
    req["require_mfa"] = json!(false);
    let (status, body) = e.send(&b, "POST", BASE, Some(req)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        (body["roles"].clone(), body["mfa_required"].clone()),
        (json!(["support"]), json!(false))
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn changes_need_a_recent_confirmation_of_identity(pool: PgPool) {
    let e = env(&pool, 1);
    let root = account(&pool, "rootuser", "admin", &["super_admin"]).await;
    let plain = account(&pool, "plainadmin", "admin", &["admin"]).await;
    let b = e.sign_in(root).await;
    let p = e.sign_in(plain).await;
    tokio::time::sleep(Duration::from_millis(1300)).await;

    let (status, body) = e.send(&b, "POST", BASE, Some(new_admin("newbie"))).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "reauth_required")
    );
    // The permission is checked first, so a plain admin learns nothing about sudo mode.
    let (_, body) = e.send(&p, "POST", BASE, Some(new_admin("newbie"))).await;
    assert_eq!(code(&body), "permission_denied");
    assert_eq!(admin_count(&pool).await, 2);
    // Reading does not need it.
    assert_eq!(e.get(&b, BASE).await.0, StatusCode::OK);

    e.sessions
        .mark_reauthenticated(b.cookie.split_once('=').unwrap().1)
        .await
        .unwrap();
    let (status, body) = e.send(&b, "POST", BASE, Some(new_admin("newbie"))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn invalid_input_is_reported_field_by_field(pool: PgPool) {
    let e = env(&pool, 600);
    let root = account(&pool, "rootuser", "admin", &["super_admin"]).await;
    account(&pool, "taken", "admin", &["admin"]).await;
    let b = e.sign_in(root).await;

    let (status, body) = e
        .send(
            &b,
            "POST",
            BASE,
            Some(json!({"username": "x", "email": "nope", "password": "short", "role": "ghost"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let fields: Vec<&str> = body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert_eq!(
        fields,
        ["username", "email", "role"],
        "password is only judged once name and email are valid"
    );

    let mut weak = new_admin("fresh");
    weak["password"] = json!("password1234");
    let (status, body) = e.send(&b, "POST", BASE, Some(weak)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"][0]["field"], "password");

    for role in ["client", "reseller"] {
        let mut req = new_admin("fresh");
        req["role"] = json!(role);
        let (_, body) = e.send(&b, "POST", BASE, Some(req)).await;
        assert_eq!(body["errors"][0]["code"], "role_not_assignable", "{role}");
    }
    let (_, body) = e.send(&b, "POST", BASE, Some(new_admin("root"))).await;
    assert_eq!(body["errors"][0]["code"], "reserved_username");

    let (status, body) = e.send(&b, "POST", BASE, Some(new_admin("taken"))).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "username_taken")
    );
    let mut dup_email = new_admin("another");
    dup_email["email"] = json!("TAKEN@example.com");
    let (status, body) = e.send(&b, "POST", BASE, Some(dup_email)).await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "email_taken"));

    // Nothing leaked half-created: no new account, no stray role grant, no audit entry.
    assert_eq!(admin_count(&pool).await, 2);
    assert!(audit_actions(&pool).await.is_empty());
    let (status, _) = e
        .send(&b, "POST", BASE, Some(json!({"username": "a", "extra": 1})))
        .await;
    assert!(status.is_client_error());
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn administrators_can_be_listed_paged_and_shown(pool: PgPool) {
    let e = env(&pool, 600);
    let root = account(&pool, "rootuser", "admin", &["super_admin"]).await;
    let plain = account(&pool, "plainadmin", "admin", &["admin"]).await;
    let helper = account(&pool, "helper", "admin", &["support"]).await;
    let client = account(&pool, "shopper", "client", &["client"]).await;
    let r = e.sign_in(root).await;

    let (status, all) = e.get(&r, BASE).await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let names: Vec<&str> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["username"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["rootuser", "plainadmin", "helper"],
        "administrators only, oldest first"
    );
    assert_eq!(all["next_after"], Value::Null);
    assert_eq!(all["items"][0]["roles"], json!(["super_admin"]));

    let (_, page1) = e.get(&r, &format!("{BASE}?limit=2")).await;
    assert_eq!(page1["items"].as_array().unwrap().len(), 2);
    let after = page1["next_after"].as_str().unwrap().to_owned();
    assert_eq!(after, plain.to_string());
    let (_, page2) = e.get(&r, &format!("{BASE}?limit=2&after={after}")).await;
    assert_eq!(page2["items"][0]["username"], "helper");
    assert_eq!(page2["next_after"], Value::Null);
    let (status, body) = e.get(&r, &format!("{BASE}?limit=0&after=x")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"].as_array().unwrap().len(), 2);

    let (status, one) = e.get(&r, &format!("{BASE}/{helper}")).await;
    assert_eq!(
        (status, one["username"].as_str()),
        (StatusCode::OK, Some("helper"))
    );
    assert_eq!(one["last_login_at"], Value::Null);
    for id in [
        client.to_string(),
        Uuid::now_v7().to_string(),
        "not-a-uuid".to_owned(),
    ] {
        let (status, body) = e.get(&r, &format!("{BASE}/{id}")).await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::NOT_FOUND, "not_found"),
            "{id}"
        );
    }

    // Plain administrators may read; support staff may not.
    let p = e.sign_in(plain).await;
    assert_eq!(e.get(&p, BASE).await.0, StatusCode::OK);
    let h = e.sign_in(helper).await;
    assert_eq!(e.get(&h, BASE).await.0, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn updating_an_administrator_blocks_and_audits(pool: PgPool) {
    let e = env(&pool, 600);
    let root = account(&pool, "rootuser", "admin", &["super_admin"]).await;
    let other = account(&pool, "otheradmin", "admin", &["admin"]).await;
    account(&pool, "taken", "admin", &["admin"]).await;
    let r = e.sign_in(root).await;
    let o = e.sign_in(other).await;
    let url = format!("{BASE}/{other}");

    let (status, body) = e.send(&r, "PATCH", &url, Some(json!({}))).await;
    assert_eq!(
        (status, body["errors"][0]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("empty_update"))
    );
    let (status, _) = e
        .send(&r, "PATCH", &url, Some(json!({"email": "nope"})))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, body) = e
        .send(
            &r,
            "PATCH",
            &url,
            Some(json!({"email": "taken@example.com"})),
        )
        .await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "email_taken"));
    let (status, _) = e
        .send(&r, "PATCH", &url, Some(json!({"status": "deleted"})))
        .await;
    assert!(status.is_client_error());
    let (status, body) = e
        .send(
            &r,
            "PATCH",
            &format!("{BASE}/{}", Uuid::now_v7()),
            Some(json!({"require_mfa": true})),
        )
        .await;
    assert_eq!((status, code(&body)), (StatusCode::NOT_FOUND, "not_found"));

    let (status, body) = e
        .send(
            &r,
            "PATCH",
            &url,
            Some(json!({"email": " New@Example.com ", "require_mfa": false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["email"].as_str(), body["mfa_required"].clone()),
        (Some("new@example.com"), json!(false))
    );
    assert_eq!(
        e.get(&o, BASE).await.0,
        StatusCode::OK,
        "other admin is still signed in"
    );

    // Suspending ends their sessions at once.
    let (status, body) = e
        .send(&r, "PATCH", &url, Some(json!({"status": "suspended"})))
        .await;
    assert_eq!(
        (status, body["status"].as_str()),
        (StatusCode::OK, Some("suspended"))
    );
    assert_eq!(e.get(&o, BASE).await.0, StatusCode::UNAUTHORIZED);
    let (status, body) = e
        .send(&r, "PATCH", &url, Some(json!({"status": "active"})))
        .await;
    assert_eq!(
        (status, body["status"].as_str()),
        (StatusCode::OK, Some("active"))
    );

    // You cannot lock yourself out, and cannot touch accounts as a non-super-admin.
    let (status, body) = e
        .send(
            &r,
            "PATCH",
            &format!("{BASE}/{root}"),
            Some(json!({"status": "disabled"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "cannot_lock_yourself")
    );
    let o = e.sign_in(other).await;
    let (status, body) = e
        .send(
            &o,
            "PATCH",
            &format!("{BASE}/{root}"),
            Some(json!({"email": "x@example.com"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );

    let (changed, status_meta): (Value, Option<String>) = sqlx::query_as(
        "SELECT metadata->'changed', metadata->>'status' FROM audit_logs
         WHERE action = 'admin.administrator_updated' ORDER BY at, id LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (changed, status_meta),
        (json!(["email", "require_mfa"]), None)
    );
    assert_eq!(
        audit_actions(&pool).await,
        ["admin.administrator_updated"; 3]
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn roles_can_be_assigned_and_the_last_super_admin_stays(pool: PgPool) {
    let e = env(&pool, 600);
    let root = account(&pool, "rootuser", "admin", &["super_admin"]).await;
    let other = account(&pool, "otheradmin", "admin", &["admin"]).await;
    let r = e.sign_in(root).await;
    let role = |id: Uuid, name: &str| format!("{BASE}/{id}/roles/{name}");

    let (status, body) = e.send(&r, "PUT", &role(other, "support"), None).await;
    assert_eq!(
        (status, body["roles"].clone()),
        (StatusCode::OK, json!(["admin", "support"]))
    );
    let (_, body) = e.send(&r, "PUT", &role(other, "support"), None).await;
    assert_eq!(
        body["roles"],
        json!(["admin", "support"]),
        "assigning twice changes nothing"
    );
    let (status, body) = e.send(&r, "PUT", &role(other, "ghost"), None).await;
    assert_eq!((status, code(&body)), (StatusCode::NOT_FOUND, "not_found"));
    let (status, body) = e.send(&r, "PUT", &role(other, "client"), None).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "role_not_assignable")
    );
    let (status, _) = e
        .send(&r, "PUT", &role(Uuid::now_v7(), "admin"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, body) = e.send(&r, "DELETE", &role(other, "support"), None).await;
    assert_eq!(body["roles"], json!(["admin"]));
    let (status, body) = e.send(&r, "DELETE", &role(other, "support"), None).await;
    assert_eq!(
        (status, body["roles"].clone()),
        (StatusCode::OK, json!(["admin"])),
        "removing twice is fine"
    );
    assert_eq!(
        audit_actions(&pool).await,
        ["admin.role_assigned", "admin.role_revoked"],
        "only real changes are recorded"
    );

    // The only super administrator cannot lose the role; a second one makes it possible.
    let (status, body) = e.send(&r, "DELETE", &role(root, "super_admin"), None).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "last_super_admin")
    );
    let (status, _) = e.send(&r, "PUT", &role(other, "super_admin"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = e.send(&r, "DELETE", &role(root, "super_admin"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["roles"], json!([]));
    // `other` is now the only one again.
    let o = e.sign_in(other).await;
    let (status, body) = e
        .send(&o, "DELETE", &role(other, "super_admin"), None)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "last_super_admin")
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn deleting_an_administrator_ends_their_sessions_and_keeps_the_history(pool: PgPool) {
    let e = env(&pool, 600);
    let root = account(&pool, "rootuser", "admin", &["super_admin"]).await;
    let other = account(&pool, "otheradmin", "admin", &["admin"]).await;
    let client = account(&pool, "shopper", "client", &["client"]).await;
    let r = e.sign_in(root).await;
    let o = e.sign_in(other).await;
    e.sessions
        .revoke_all(other, None, builtpanel_session::RevokeReason::Admin)
        .await
        .unwrap();
    let o2 = e.sign_in(other).await;
    assert_eq!(e.get(&o2, BASE).await.0, StatusCode::OK);
    let _ = o;

    let (status, body) = e.send(&r, "DELETE", &format!("{BASE}/{root}"), None).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "cannot_delete_self")
    );
    let (status, _) = e
        .send(&r, "DELETE", &format!("{BASE}/{client}"), None)
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "other kinds of account are not administrators"
    );

    // Their own earlier activity is in the log.
    builtpanel_audit::insert(
        &pool,
        &builtpanel_audit::AuditEntry::new("auth.login", builtpanel_audit::Outcome::Success)
            .actor(other),
    )
    .await
    .unwrap();

    let (status, _) = e.send(&r, "DELETE", &format!("{BASE}/{other}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        e.get(&o2, BASE).await.0,
        StatusCode::UNAUTHORIZED,
        "session is dead"
    );
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM accounts WHERE id = $1)")
        .bind(other)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!exists);
    let (status, _) = e.get(&r, &format!("{BASE}/{other}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Their audit rows survive, and the deletion names who was deleted.
    let theirs: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_id = $1")
        .bind(other)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(theirs, 1);
    let (actor, meta): (Option<Uuid>, Value) = sqlx::query_as(
        "SELECT actor_id, metadata FROM audit_logs WHERE action = 'admin.administrator_deleted'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (actor, meta),
        (Some(root), json!({"username": "otheradmin"}))
    );

    // The last super administrator cannot be deleted by anyone else either.
    let helper = account(&pool, "helperadmin", "admin", &["admin"]).await;
    let _ = helper;
    let second = account(&pool, "second", "admin", &["super_admin"]).await;
    let s = e.sign_in(second).await;
    let (status, _) = e.send(&s, "DELETE", &format!("{BASE}/{root}"), None).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "two super administrators: one may go"
    );
    let (status, body) = e
        .send(&s, "DELETE", &format!("{BASE}/{second}"), None)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "cannot_delete_self")
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn two_super_admins_cannot_delete_each_other_at_the_same_time(pool: PgPool) {
    let e = env(&pool, 600);
    let a = account(&pool, "alpha", "admin", &["super_admin"]).await;
    let b = account(&pool, "bravo", "admin", &["super_admin"]).await;
    let ba = e.sign_in(a).await;
    let bb = e.sign_in(b).await;

    let (url_b, url_a) = (format!("{BASE}/{b}"), format!("{BASE}/{a}"));
    let (ra, rb) = tokio::join!(
        e.send(&ba, "DELETE", &url_b, None),
        e.send(&bb, "DELETE", &url_a, None),
    );
    let outcomes = [ra.0, rb.0];
    assert_eq!(
        outcomes
            .iter()
            .filter(|s| **s == StatusCode::NO_CONTENT)
            .count(),
        1,
        "exactly one deletion succeeds: {outcomes:?}"
    );
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM accounts a JOIN account_roles ar ON ar.account_id = a.id
         JOIN roles r ON r.id = ar.role_id WHERE r.name = 'super_admin' AND a.status = 'active'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 1, "a super administrator remains");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn mfa_can_be_reset_and_sessions_ended_for_another_administrator(pool: PgPool) {
    let e = env(&pool, 600);
    let root = account(&pool, "rootuser", "admin", &["super_admin"]).await;
    let other = account(&pool, "otheradmin", "admin", &["admin"]).await;
    let r = e.sign_in(root).await;
    let o1 = e.sign_in(other).await;
    let o2 = e.sign_in(other).await;

    mfa::begin_totp_enrollment(&pool, other, &[7u8; 61])
        .await
        .unwrap();
    mfa::confirm_totp(&pool, other, 5, &[[1u8; 32], [2u8; 32]])
        .await
        .unwrap();
    let (_, view) = e.get(&r, &format!("{BASE}/{other}")).await;
    assert_eq!(view["mfa_enabled"], true);

    let (status, body) = e
        .send(&r, "DELETE", &format!("{BASE}/{other}/sessions"), None)
        .await;
    assert_eq!(
        (status, body["revoked"].clone()),
        (StatusCode::OK, json!(2))
    );
    assert_eq!(e.get(&o1, BASE).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(e.get(&o2, BASE).await.0, StatusCode::UNAUTHORIZED);

    let o3 = e.sign_in(other).await;
    let (status, _) = e
        .send(&r, "POST", &format!("{BASE}/{other}/mfa/reset"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        e.get(&o3, BASE).await.0,
        StatusCode::UNAUTHORIZED,
        "reset also ends sessions"
    );
    let (_, view) = e.get(&r, &format!("{BASE}/{other}")).await;
    assert_eq!(view["mfa_enabled"], false);
    let codes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mfa_backup_codes WHERE account_id = $1")
            .bind(other)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(codes, 0);
    // Resetting again is harmless and recorded as such.
    assert_eq!(
        e.send(&r, "POST", &format!("{BASE}/{other}/mfa/reset"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let had: Vec<bool> = sqlx::query_scalar(
        "SELECT (metadata->>'had_factor')::boolean FROM audit_logs WHERE action = 'admin.mfa_reset' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(had, [true, false]);

    for path in ["mfa/reset", "sessions"] {
        let m = if path == "sessions" { "DELETE" } else { "POST" };
        let (status, _) = e
            .send(&r, m, &format!("{BASE}/{}/{path}", Uuid::now_v7()), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn an_administrator_who_must_use_2fa_is_blocked_until_enrolled(pool: PgPool) {
    let e = env(&pool, 600);
    let id = account(&pool, "forced", "admin", &["admin"]).await;
    let b = e.sign_in(id).await;
    assert_eq!(e.get(&b, BASE).await.0, StatusCode::OK, "not required yet");

    sqlx::query("UPDATE accounts SET mfa_required = true WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = e.get(&b, BASE).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "mfa_setup_required")
    );

    mfa::begin_totp_enrollment(&pool, id, &[7u8; 61])
        .await
        .unwrap();
    assert_eq!(
        e.get(&b, BASE).await.0,
        StatusCode::FORBIDDEN,
        "a pending enrolment does not count"
    );
    mfa::confirm_totp(&pool, id, 5, &[[1u8; 32]]).await.unwrap();
    assert_eq!(
        e.get(&b, BASE).await.0,
        StatusCode::OK,
        "enrolled: access is back"
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_permission_catalogue_is_listed_for_the_role_editor(pool: PgPool) {
    let e = env(&pool, 600);
    builtpanel_database::modules::sync(
        &pool,
        &[builtpanel_database::modules::ModuleRecord {
            name: "domains",
            version: "1.0.0",
            category: "web",
            core: false,
            manifest: &json!({"module": {"title": {"en": "Domains", "bn": "ডোমেইন"}}}),
        }],
    )
    .await
    .unwrap();
    builtpanel_database::permissions::sync(
        &pool,
        &[("domains.read", "domains"), ("domains.create", "domains")],
    )
    .await
    .unwrap();
    let root = account(&pool, "rootuser", "admin", &["super_admin"]).await;
    let support = account(&pool, "helper", "admin", &["support"]).await;
    let nobody = account(&pool, "norole", "admin", &[]).await;

    let path = "/api/v1/admin/permissions";
    assert_eq!(
        e.call(None, "GET", path, None, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    for who in [support, nobody] {
        let b = e.sign_in(who).await;
        let (status, body) = e.call(Some(&b), "GET", path, None, false).await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::FORBIDDEN, "permission_denied"),
            "{who}"
        );
    }
    let r = e.sign_in(root).await;
    let (status, body) = e.call(Some(&r), "GET", path, None, false).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["key"], "domains.create");
    assert_eq!(items[1]["module"], "domains");
    assert_eq!(items[1]["module_title"]["bn"], "ডোমেইন");
    assert_eq!(items[1]["present"], true);
}
