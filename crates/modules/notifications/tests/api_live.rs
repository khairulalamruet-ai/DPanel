//! The notifications API over HTTP, with real sessions (T18.03). Ignored by default; run with
//! DATABASE_URL, REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, OnceLock};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_cache::RedisHandle;
use builtpanel_kernel::Event;
use builtpanel_mod_notifications::{NotificationsState, admin_router, client_router, handle_event};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

struct Env {
    app: Router,
    state: NotificationsState,
}

struct Browser {
    cookie: String,
    csrf: String,
}

fn env(pool: &PgPool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let state = NotificationsState {
        db: pool.clone(),
        sessions: SessionStore::new(redis, pool.clone(), SessionConfig::default()),
        settings: Arc::new(OnceLock::new()),
        master_key: MasterKey::from_bytes(&[3u8; 32]),
    };
    let app = Router::new()
        .merge(client_router().with_state(state.clone()))
        .merge(admin_router().with_state(state.clone()));
    Env { app, state }
}

impl Env {
    async fn sign_in(&self, account_id: Uuid) -> Browser {
        let (t, data) = self
            .state
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
}

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
        builtpanel_database::rbac::assign_role(pool, id, role, None)
            .await
            .unwrap();
    }
    id
}

async fn tenant(pool: &PgPool, owner: Uuid) -> Uuid {
    let node: Uuid = sqlx::query_scalar("INSERT INTO nodes (name) VALUES ('node1') RETURNING id")
        .fetch_one(pool)
        .await
        .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'alice', 10001, 'active') RETURNING id",
    )
    .bind(owner)
    .bind(node)
    .bind(plan)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL and Redis"]
async fn an_account_reads_its_inbox_sets_channels_and_pauses(pool: PgPool) {
    let e = env(&pool);
    let owner = account(&pool, "alice", "client", &[]).await;
    let other = account(&pool, "bob", "client", &[]).await;
    let tenant_id = tenant(&pool, owner).await;
    let (alice, bob) = (e.sign_in(owner).await, e.sign_in(other).await);
    handle_event(
        &e.state,
        &Event::BackupFailed {
            backup_id: Uuid::new_v4(),
            tenant_id: Some(tenant_id),
            error: "full".into(),
        },
    )
    .await
    .unwrap();

    // The inbox: one unread, only for the owner.
    let (status, got) = e
        .call(&alice, "GET", "/api/v1/client/notifications", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got["unread"], 1);
    assert_eq!(got["items"][0]["event_type"], "backup_failed");
    assert_eq!(got["items"][0]["read"], false);
    assert_eq!(
        e.call(&bob, "GET", "/api/v1/client/notifications", None)
            .await
            .1["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let (status, _) = e
        .call(&alice, "GET", "/api/v1/client/notifications?limit=0", None)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let id = got["items"][0]["id"].clone();
    // Someone else cannot read it away; the owner can.
    assert_eq!(
        e.call(
            &bob,
            "POST",
            "/api/v1/client/notifications/read",
            Some(json!({"ids": [id]}))
        )
        .await
        .1["updated"],
        0
    );
    assert_eq!(
        e.call(
            &alice,
            "POST",
            "/api/v1/client/notifications/read",
            Some(json!({"ids": [id]}))
        )
        .await
        .1["updated"],
        1
    );
    assert_eq!(
        e.call(
            &alice,
            "GET",
            "/api/v1/client/notifications?unread=true",
            None
        )
        .await
        .1["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // Channels: the events a client may ask for are its own.
    let (_, events) = e
        .call(&alice, "GET", "/api/v1/client/notifications/events", None)
        .await;
    assert_eq!(
        events["events"],
        json!(["certificate_expiring", "backup_failed"])
    );
    let (status, made) = e
        .call(&alice, "POST", "/api/v1/client/notifications/channels", Some(json!({"kind": "email", "name": "me", "target": "alice@example.org", "events": ["backup_failed"]})))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert!(made["secret"].is_null());
    assert_eq!(made["channel"]["events"], json!(["backup_failed"]));
    let channel = made["channel"]["id"].as_str().unwrap().to_owned();
    // A name is used once; a node's event is not a client's to ask for; a webhook must be a public https URL.
    let (status, got) = e
        .call(
            &alice,
            "POST",
            "/api/v1/client/notifications/channels",
            Some(json!({"kind": "email", "name": "me", "target": "a@example.org"})),
        )
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("channel_exists")),
        "{got}"
    );
    for bad in [
        json!({"kind": "email", "name": "x", "target": "not an address"}),
        json!({"kind": "email", "name": "x", "target": "a@example.org", "events": ["node_offline"]}),
        json!({"kind": "webhook", "name": "x", "target": "http://hooks.example.com/in"}),
        json!({"kind": "webhook", "name": "x", "target": "https://127.0.0.1/in"}),
        json!({"kind": "sms", "name": "x", "target": "123456"}),
    ] {
        let (status, got) = e
            .call(
                &alice,
                "POST",
                "/api/v1/client/notifications/channels",
                Some(bad.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {got}");
    }
    // A webhook is made with a secret, shown once, and stored sealed.
    let (status, hook) = e
        .call(&alice, "POST", "/api/v1/client/notifications/channels", Some(json!({"kind": "webhook", "name": "hook", "target": "https://hooks.example.com/in", "events": []})))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    assert!(hook["secret"].as_str().unwrap().starts_with("whsec_"));
    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT sealed_secret FROM notification_channels WHERE name = 'hook'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !String::from_utf8_lossy(&sealed).contains("whsec_"),
        "the secret is not stored as it is"
    );
    let listed = e
        .call(&alice, "GET", "/api/v1/client/notifications/channels", None)
        .await
        .1;
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert!(
        !listed.to_string().contains("whsec_"),
        "the list never carries a secret"
    );
    // Somebody else's channel is not found, not forbidden (it does not exist for them).
    let path = format!("/api/v1/client/notifications/channels/{channel}");
    assert_eq!(
        e.call(&bob, "PATCH", &path, Some(json!({"enabled": false})))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(&bob, "DELETE", &path, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(
            &alice,
            "PATCH",
            &path,
            Some(json!({"enabled": false, "events": ["certificate_expiring"]}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let listed = e
        .call(&alice, "GET", "/api/v1/client/notifications/channels", None)
        .await
        .1;
    assert_eq!(
        (listed[0]["enabled"].clone(), listed[0]["events"].clone()),
        (json!(false), json!(["certificate_expiring"]))
    );
    assert_eq!(
        e.call(&alice, "DELETE", &path, None).await.0,
        StatusCode::NO_CONTENT
    );

    // Pause and snooze.
    let (_, quiet) = e
        .call(
            &alice,
            "PUT",
            "/api/v1/client/notifications/quiet",
            Some(json!({"paused": false, "snooze_minutes": 60})),
        )
        .await;
    assert_eq!(quiet["paused"], false);
    assert!(quiet["snoozed_until"].as_i64().unwrap() > 0);
    let (status, _) = e
        .call(
            &alice,
            "PUT",
            "/api/v1/client/notifications/quiet",
            Some(json!({"paused": false, "snooze_minutes": 99999})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, quiet) = e
        .call(
            &alice,
            "PUT",
            "/api/v1/client/notifications/quiet",
            Some(json!({"paused": true})),
        )
        .await;
    assert_eq!(
        (quiet["paused"].clone(), quiet["snoozed_until"].clone()),
        (json!(true), Value::Null)
    );
    assert_eq!(
        e.call(&alice, "GET", "/api/v1/client/notifications/quiet", None)
            .await
            .1["paused"],
        true
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL and Redis"]
async fn only_an_administrator_with_the_right_sets_the_mail_server_and_the_password_is_never_shown(
    pool: PgPool,
) {
    let e = env(&pool);
    let root = e
        .sign_in(account(&pool, "root1", "admin", &["super_admin"]).await)
        .await;
    let client = e
        .sign_in(account(&pool, "carol", "client", &[]).await)
        .await;
    let body = json!({"host": "mail.example.com", "port": 587, "security": "starttls", "username": "panel", "password": "s3cret-pass", "from_address": "panel@example.com", "from_name": "Acme", "enabled": true});
    // A client has no such route and no such right.
    assert_ne!(
        e.call(
            &client,
            "PUT",
            "/api/v1/admin/notifications/smtp",
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        e.call(&root, "GET", "/api/v1/admin/notifications/smtp", None)
            .await
            .1,
        Value::Null,
        "none set yet"
    );
    assert_eq!(
        e.call(&root, "PUT", "/api/v1/admin/notifications/smtp", Some(body))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (_, got) = e
        .call(&root, "GET", "/api/v1/admin/notifications/smtp", None)
        .await;
    assert_eq!(
        (got["host"].clone(), got["has_password"].clone()),
        (json!("mail.example.com"), json!(true))
    );
    assert!(!got.to_string().contains("s3cret"), "{got}");
    let sealed: Vec<u8> = sqlx::query_scalar("SELECT sealed_password FROM notification_smtp")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&sealed).contains("s3cret"),
        "sealed, not plain"
    );
    // Changing the host without the password keeps the password.
    let again = json!({"host": "mail2.example.com", "port": 465, "security": "tls", "from_address": "panel@example.com", "enabled": true});
    assert_eq!(
        e.call(
            &root,
            "PUT",
            "/api/v1/admin/notifications/smtp",
            Some(again)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        e.call(&root, "GET", "/api/v1/admin/notifications/smtp", None)
            .await
            .1["has_password"],
        true
    );
    // Bad settings are refused.
    for bad in [
        json!({"host": "", "port": 25, "security": "none", "from_address": "a@b.co", "enabled": true}),
        json!({"host": "mail.example.com", "port": 25, "security": "plain", "from_address": "a@b.co", "enabled": true}),
        json!({"host": "mail.example.com", "port": 25, "security": "none", "from_address": "nope", "enabled": true}),
        json!({"host": "bad host", "port": 25, "security": "none", "from_address": "a@b.co", "enabled": true}),
    ] {
        assert_eq!(
            e.call(
                &root,
                "PUT",
                "/api/v1/admin/notifications/smtp",
                Some(bad.clone())
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{bad}"
        );
    }
    // An administrator may ask for the events of the nodes.
    let (_, events) = e
        .call(&root, "GET", "/api/v1/admin/notifications/events", None)
        .await;
    assert!(
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "node_offline"),
        "{events}"
    );
}
