//! Payments, subscriptions and renewal notices against a real PostgreSQL (T17.18-T17.20).
//! Ignored by default; run with DATABASE_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_auth_core::password::{Argon2Params, PasswordHasher};
use builtpanel_license::FileSigningKey;
use builtpanel_license_authority::{
    LaState, Settings,
    billing::{self, Timing},
    config::LaConfig,
    register_key, router,
};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const SECRET: &str = "a-shared-webhook-secret";
const DAY: i64 = 86_400;

struct Env {
    app: Router,
    state: LaState,
}

async fn env(pool: &PgPool, secret: Option<&str>) -> Env {
    let key_dir = std::env::temp_dir().join(format!("bp-la-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&key_dir).unwrap();
    let key =
        FileSigningKey::generate_to_file(&key_dir.join("license-1.key"), "license-1").unwrap();
    register_key(pool, &key).await.unwrap();
    let config = LaConfig {
        signing_key_file: key_dir.join("license-1.key"),
        payment_webhook_secret: secret.map(|s| SecretString::from(s.to_owned())),
        ..LaConfig::default()
    };
    let state = LaState {
        db: pool.clone(),
        signer: Arc::new(std::sync::RwLock::new(Arc::new(key))),
        settings: Settings::from_config(&config),
        prover: builtpanel_license_authority::domain_proof::default_prover(),
        hasher: Arc::new(
            PasswordHasher::new(Argon2Params {
                memory_kib: 19 * 1024,
                iterations: 2,
                parallelism: 1,
            })
            .unwrap(),
        ),
    };
    Env {
        app: router(state.clone()),
        state,
    }
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    call_with(app, method, path, token, body, &[]).await
}

async fn call_with(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = match body {
        Some(v) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}

async fn admin(pool: &PgPool, env: &Env, email: &str, role: &str) -> String {
    let hasher = PasswordHasher::new(Argon2Params {
        memory_kib: 19 * 1024,
        iterations: 2,
        parallelism: 1,
    })
    .unwrap();
    let hash = hasher
        .hash(&SecretString::from("a-long-admin-password".to_owned()))
        .unwrap();
    sqlx::query("INSERT INTO admin_users (email, password_hash, role) VALUES ($1, $2, $3)")
        .bind(email)
        .bind(hash)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    let (status, session) = call(
        &env.app,
        "POST",
        "/api/v1/admin/login",
        None,
        Some(json!({"email": email, "password": "a-long-admin-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    session["token"].as_str().unwrap().to_owned()
}

/// A license of a plan allowing `max` installations.
/// A license of a plan allowing `max` installations.
async fn license(env: &Env, token: &str, max: i32, features: &[&str]) -> (String, String) {
    let (_, customer) = call(
        &env.app,
        "POST",
        "/api/v1/admin/customers",
        Some(token),
        Some(json!({"name": "Acme", "email": format!("{}@acme.example", Uuid::new_v4().simple())})),
    )
    .await;
    let (_, product) = call(&env.app, "POST", "/api/v1/admin/products", Some(token), Some(json!({"slug": format!("p{}", &Uuid::new_v4().simple().to_string()[..8]), "name": "Built Panel"}))).await;
    let (status, plan) = call(
        &env.app,
        "POST",
        "/api/v1/admin/plans",
        Some(token),
        Some(json!({"product_id": product["id"], "name": "Pro", "features": features, "limits": {"max_nodes": 3, "max_tenants": -1}, "max_activations": max, "valid_days": 365})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plan}");
    let (status, made) = call(
        &env.app,
        "POST",
        "/api/v1/admin/licenses",
        Some(token),
        Some(json!({"customer_id": customer["id"], "plan_id": plan["id"], "domains": ["panel.example"], "hostnames": ["web1"], "ips": ["203.0.113.7"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    (
        made["license"]["id"].as_str().unwrap().to_owned(),
        made["key"].as_str().unwrap().to_owned(),
    )
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .try_into()
        .unwrap()
}

/// What the provider sends: a signed event.
async fn hook(env: &Env, body: &Value, signed_at: i64, secret: &str) -> (StatusCode, Value) {
    let text = body.to_string();
    let header = billing::sign(secret.as_bytes(), signed_at, text.as_bytes());
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/payments")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-builtpanel-signature", header)
        .body(Body::from(text))
        .unwrap();
    let res = env.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn paid(env: &Env, id: &str, reference: &str, end: i64) -> Value {
    let (status, body) = hook(
        env,
        &json!({"id": id, "type": "payment.succeeded", "reference": reference, "period_end": end}),
        now(),
        SECRET,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// `(status, note, expires_at)` of a license.
async fn license_row(pool: &PgPool, id: &str) -> (String, Option<String>, Option<i64>) {
    sqlx::query_as(
        "SELECT status, status_note, extract(epoch FROM expires_at)::bigint FROM licenses WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn state_of(env: &Env, token: &str, sub: &str) -> String {
    let (status, v) = call(
        &env.app,
        "GET",
        &format!("/api/v1/admin/subscriptions/{sub}"),
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v["state"].as_str().unwrap().to_owned()
}

async fn subscribe(env: &Env, token: &str, license: &str, reference: &str) -> String {
    let (status, v) = call(
        &env.app,
        "POST",
        "/api/v1/admin/subscriptions",
        Some(token),
        Some(json!({"license_id": license, "provider": "acme-pay", "reference": reference})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["state"], json!("pending"));
    v["id"].as_str().unwrap().to_owned()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_webhook_is_off_without_a_secret_and_refuses_what_is_not_signed_in_time(pool: PgPool) {
    let env = env(&pool, Some(SECRET)).await;
    // The same authority with no secret set.
    let mut state = env.state.clone();
    state.settings.payment_webhook_secret = None;
    let off = Env {
        app: router(state.clone()),
        state,
    };
    let (status, body) = hook(&off, &json!({"id": "e1"}), now(), SECRET).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");

    let event = json!({"id": "e1", "type": "payment.succeeded", "reference": "r"});
    assert_eq!(
        hook(&env, &event, now(), "another-secret-entirely").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        hook(&env, &event, now() - 3_600, SECRET).await.0,
        StatusCode::UNAUTHORIZED,
        "a recorded request cannot be played again an hour later"
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/payments")
        .body(Body::from(event.to_string()))
        .unwrap();
    assert_eq!(
        env.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    // Nothing was kept of what was refused.
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM payment_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, 0);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_subscription_lives_from_payment_through_dunning_to_suspension_and_back(pool: PgPool) {
    let env = env(&pool, Some(SECRET)).await;
    let token = admin(&pool, &env, "root@la.example", "admin").await;
    let viewer = admin(&pool, &env, "look@la.example", "viewer").await;
    let (license_id, _) = license(&env, &token, 1, &["domains"]).await;
    let t = Timing::default();
    let sub = subscribe(&env, &token, &license_id, "sub_100").await;
    // One live subscription per license, and a reference names one subscription.
    let (status, v) = call(
        &env.app,
        "POST",
        "/api/v1/admin/subscriptions",
        Some(&token),
        Some(json!({"license_id": license_id, "provider": "acme-pay", "reference": "sub_101"})),
    )
    .await;
    assert_eq!(
        (status, v["errors"].is_null()),
        (StatusCode::CONFLICT, true),
        "{v}"
    );
    assert_eq!(
        call(
            &env.app,
            "POST",
            "/api/v1/admin/subscriptions",
            Some(&viewer),
            Some(json!({"license_id": license_id, "provider": "x", "reference": "z"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // First payment: active, and the license runs until the period and the unpaid stages end.
    let end1 = now() + 30 * DAY;
    let v = paid(&env, "evt_1", "sub_100", end1).await;
    assert_eq!(
        (v["outcome"].clone(), v["state"].clone()),
        (json!("applied"), json!("active"))
    );
    let (status, note, expires) = license_row(&pool, &license_id).await;
    assert_eq!(
        (status.as_str(), note, expires),
        ("active", None, Some(end1 + 10 * DAY))
    );
    // The same event again changes nothing; one for a subscription nobody knows is kept and told.
    assert_eq!(
        paid(&env, "evt_1", "sub_100", end1 + 99 * DAY).await["outcome"],
        json!("duplicate")
    );
    assert_eq!(
        license_row(&pool, &license_id).await.2,
        Some(end1 + 10 * DAY)
    );
    assert_eq!(
        paid(&env, "evt_2", "nobody", end1).await["outcome"],
        json!("unknown_subscription")
    );
    // An older payment arriving late is stale; an event of a kind we do not use is ignored.
    assert_eq!(
        paid(&env, "evt_3", "sub_100", end1 - DAY).await["outcome"],
        json!("stale")
    );
    let (_, v) = hook(
        &env,
        &json!({"id": "evt_4", "type": "invoice.created", "reference": "sub_100"}),
        now(),
        SECRET,
    )
    .await;
    assert_eq!(v["outcome"], json!("ignored_type"));
    // A period that ends at an impossible time is refused and not remembered.
    let (status, _) = hook(&env, &json!({"id": "evt_5", "type": "payment.succeeded", "reference": "sub_100", "period_end": 5}), now(), SECRET).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payment_events WHERE event_id = 'evt_5'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kept, 0);

    // Renewal: the next payment extends the period and tells the customer.
    let end2 = end1 + 30 * DAY;
    paid(&env, "evt_6", "sub_100", end2).await;
    assert_eq!(
        license_row(&pool, &license_id).await.2,
        Some(end2 + 10 * DAY)
    );

    // Nothing arrives: the clocks run, a day over each stage's end at a time.
    assert_eq!(billing::sweep(&pool, &t, end2 - 1).await.unwrap().moved, 0);
    assert_eq!(billing::sweep(&pool, &t, end2 + 1).await.unwrap().moved, 1);
    assert_eq!(state_of(&env, &token, &sub).await, "past_due");
    assert_eq!(
        license_row(&pool, &license_id).await.0,
        "active",
        "it works while past due"
    );
    billing::sweep(&pool, &t, end2 + 3 * DAY + 1).await.unwrap();
    assert_eq!(state_of(&env, &token, &sub).await, "grace");
    assert_eq!(
        license_row(&pool, &license_id).await.0,
        "active",
        "and in grace"
    );
    billing::sweep(&pool, &t, end2 + 10 * DAY + 1)
        .await
        .unwrap();
    assert_eq!(state_of(&env, &token, &sub).await, "suspended");
    let (status, note, _) = license_row(&pool, &license_id).await;
    assert_eq!(
        (status.as_str(), note.as_deref()),
        ("suspended", Some("subscription: suspended"))
    );
    // The same sweep again does nothing.
    assert_eq!(
        billing::sweep(&pool, &t, end2 + 10 * DAY + 2)
            .await
            .unwrap()
            .moved,
        0
    );

    // A payment brings it back, and the license with it.
    let end3 = end2 + 60 * DAY;
    paid(&env, "evt_7", "sub_100", end3).await;
    assert_eq!(state_of(&env, &token, &sub).await, "active");
    assert_eq!(
        license_row(&pool, &license_id).await,
        ("active".to_owned(), None, Some(end3 + 10 * DAY))
    );

    // A license an administrator suspended is not lifted by a payment.
    let (status, _) = call(
        &env.app,
        "POST",
        &format!("/api/v1/admin/licenses/{license_id}/suspend"),
        Some(&token),
        Some(json!({"note": "chargeback dispute"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    paid(&env, "evt_8", "sub_100", end3 + 30 * DAY).await;
    let (status, note, _) = license_row(&pool, &license_id).await;
    assert_eq!(
        (status.as_str(), note.as_deref()),
        ("suspended", Some("chargeback dispute"))
    );
    call(
        &env.app,
        "POST",
        &format!("/api/v1/admin/licenses/{license_id}/resume"),
        Some(&token),
        None,
    )
    .await;

    // A sweep that comes very late catches up through every stage, then nothing revives it.
    billing::sweep(&pool, &t, end3 + 30 * DAY + 100 * DAY)
        .await
        .unwrap();
    assert_eq!(state_of(&env, &token, &sub).await, "expired");
    assert_eq!(license_row(&pool, &license_id).await.0, "suspended");
    assert_eq!(
        paid(&env, "evt_9", "sub_100", now() + 30 * DAY).await["outcome"],
        json!("ignored_over")
    );
    assert_eq!(state_of(&env, &token, &sub).await, "expired");

    // What the customer is to be told, in order, once each; delivery is marked.
    let (_, notices) = call(
        &env.app,
        "GET",
        "/api/v1/admin/billing/notices",
        Some(&token),
        None,
    )
    .await;
    let kinds: Vec<&str> = notices
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "activated",
            "renewed",
            "renewal_upcoming",
            "payment_failed",
            "grace_started",
            "suspended",
            "renewed",
            "renewed",
            "payment_failed",
            "grace_started",
            "suspended",
            "expired"
        ],
        "{notices}"
    );
    let first = notices[0]["id"].as_str().unwrap();
    assert_eq!(
        call(
            &env.app,
            "POST",
            &format!("/api/v1/admin/billing/notices/{first}/delivered"),
            Some(&token),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &env.app,
            "POST",
            &format!("/api/v1/admin/billing/notices/{}/delivered", Uuid::new_v4()),
            Some(&token),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, pending) = call(
        &env.app,
        "GET",
        "/api/v1/admin/billing/notices",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(pending.as_array().unwrap().len(), kinds.len() - 1);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_renewal_is_announced_once_a_failed_payment_starts_the_clock_and_a_cancelled_license_runs_to_its_end(
    pool: PgPool,
) {
    let env = env(&pool, Some(SECRET)).await;
    let token = admin(&pool, &env, "root@la.example", "admin").await;
    let viewer = admin(&pool, &env, "look@la.example", "viewer").await;
    let (license_id, _) = license(&env, &token, 1, &["domains"]).await;
    let t = Timing::default();
    let sub = subscribe(&env, &token, &license_id, "sub_200").await;
    // A payment without an end: the period is what the authority is set to give.
    let (_, v) = hook(
        &env,
        &json!({"id": "p1", "type": "payment.succeeded", "reference": "sub_200"}),
        now(),
        SECRET,
    )
    .await;
    assert_eq!(v["state"], json!("active"));
    let (_, got) = call(
        &env.app,
        "GET",
        &format!("/api/v1/admin/subscriptions/{sub}"),
        Some(&token),
        None,
    )
    .await;
    let end = got["current_period_end"].as_i64().unwrap();
    assert!((end - (now() + 30 * DAY)).abs() < 60, "{end}");

    // Seven days before the end: one reminder, however often it is swept.
    assert_eq!(
        billing::sweep(&pool, &t, end - 8 * DAY)
            .await
            .unwrap()
            .reminded,
        0
    );
    assert_eq!(
        billing::sweep(&pool, &t, end - 6 * DAY)
            .await
            .unwrap()
            .reminded,
        1
    );
    assert_eq!(
        billing::sweep(&pool, &t, end - 5 * DAY)
            .await
            .unwrap()
            .reminded,
        0
    );
    let (_, notices) = call(
        &env.app,
        "GET",
        "/api/v1/admin/billing/notices",
        Some(&token),
        None,
    )
    .await;
    let kinds: Vec<&str> = notices
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["activated", "renewal_upcoming"]);
    // After the renewal the next period gets its own reminder.
    let (_, _) = hook(&env, &json!({"id": "p2", "type": "payment.succeeded", "reference": "sub_200", "period_end": end + 30 * DAY}), now(), SECRET).await;
    assert_eq!(
        billing::sweep(&pool, &t, end + 24 * DAY)
            .await
            .unwrap()
            .reminded,
        1
    );

    // A failed payment: past due at once, the license still works.
    let (_, v) = hook(
        &env,
        &json!({"id": "f1", "type": "payment.failed", "reference": "sub_200"}),
        now(),
        SECRET,
    )
    .await;
    assert_eq!(v["state"], json!("past_due"));
    assert_eq!(license_row(&pool, &license_id).await.0, "active");
    // (A second failure changes nothing.)
    let (_, v) = hook(
        &env,
        &json!({"id": "f2", "type": "payment.failed", "reference": "sub_200"}),
        now(),
        SECRET,
    )
    .await;
    assert_eq!(v["outcome"], json!("stale"));

    // Cancelling: the license runs to the end of what was paid, then the subscription is over.
    assert_eq!(
        call(
            &env.app,
            "POST",
            &format!("/api/v1/admin/subscriptions/{sub}/cancel"),
            Some(&viewer),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, v) = call(
        &env.app,
        "POST",
        &format!("/api/v1/admin/subscriptions/{sub}/cancel"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(
        (status, v["state"].clone()),
        (StatusCode::OK, json!("cancelled")),
        "{v}"
    );
    assert_eq!(
        license_row(&pool, &license_id).await.2,
        Some(end + 30 * DAY)
    );
    assert_eq!(
        call(
            &env.app,
            "POST",
            &format!("/api/v1/admin/subscriptions/{sub}/cancel"),
            Some(&token),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        billing::sweep(&pool, &t, end + 29 * DAY)
            .await
            .unwrap()
            .moved,
        0
    );
    assert_eq!(
        billing::sweep(&pool, &t, end + 31 * DAY)
            .await
            .unwrap()
            .moved,
        1
    );
    assert_eq!(state_of(&env, &token, &sub).await, "expired");
    // A new subscription for the same license can be made once the old one is over.
    subscribe(&env, &token, &license_id, "sub_201").await;
    // A revoked license cannot have one.
    let (revoked_id, _) = license(&env, &token, 1, &["domains"]).await;
    call(
        &env.app,
        "POST",
        &format!("/api/v1/admin/licenses/{revoked_id}/revoke"),
        Some(&token),
        Some(json!({"note": "fraud"})),
    )
    .await;
    let (status, _) = call(
        &env.app,
        "POST",
        "/api/v1/admin/subscriptions",
        Some(&token),
        Some(json!({"license_id": revoked_id, "provider": "x", "reference": "sub_300"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Everything that changed a subscription is in the audit log.
    let changes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'subscription.changed'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(changes, 5, "paid, renewed, failed, cancelled, expired");
}
