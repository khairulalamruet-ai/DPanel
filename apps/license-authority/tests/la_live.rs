//! The license authority over HTTP against a real PostgreSQL (T17.01-T17.05, T17.08).
//! Ignored by default; run with DATABASE_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use builtpanel_auth_core::password::{Argon2Params, PasswordHasher};
use builtpanel_license::{Document, Expect, FileSigningKey, PublicKeys, SigningKey as _};
use builtpanel_license_authority::{
    LaState, Settings, config::LaConfig, public::proof_message, register_key, router,
};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

struct Env {
    app: Router,
    key_dir: std::path::PathBuf,
}

async fn env(pool: &PgPool) -> Env {
    let key_dir = std::env::temp_dir().join(format!("bp-la-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&key_dir).unwrap();
    let key =
        FileSigningKey::generate_to_file(&key_dir.join("license-1.key"), "license-1").unwrap();
    register_key(pool, &key).await.unwrap();
    let config = LaConfig {
        signing_key_file: key_dir.join("license-1.key"),
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
        app: router(state),
        key_dir,
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

/// An installation: an identity key pair that signs what the authority asks for.
struct Installation {
    id: Uuid,
    key: FileSigningKey,
}

impl Installation {
    fn new(dir: &std::path::Path) -> Self {
        let id = Uuid::new_v4();
        Self {
            id,
            key: FileSigningKey::generate_to_file(&dir.join(format!("{id}.key")), "installation")
                .unwrap(),
        }
    }

    async fn nonce(&self, env: &Env) -> String {
        let (status, c) = call(
            &env.app,
            "POST",
            "/api/v1/licenses/challenge",
            None,
            Some(json!({"installation_id": self.id})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{c}");
        c["nonce"].as_str().unwrap().to_owned()
    }

    fn sign(&self, purpose: &str, nonce: &str, extra: Option<&str>) -> String {
        URL_SAFE_NO_PAD.encode(
            self.key
                .sign(&proof_message(purpose, nonce, self.id, extra))
                .unwrap(),
        )
    }

    async fn activate(&self, env: &Env, key: &str) -> (StatusCode, Value) {
        self.activate_as(env, key, "web1", "panel.example", "203.0.113.7")
            .await
    }

    /// Activates saying it is `hostname` at `domain`, seen from `ip` (what the proxy writes).
    async fn activate_as(
        &self,
        env: &Env,
        key: &str,
        hostname: &str,
        domain: &str,
        ip: &str,
    ) -> (StatusCode, Value) {
        let nonce = self.nonce(env).await;
        let hash: String = builtpanel_license_authority::sha256(key.trim())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        call_with(
            &env.app,
            "POST",
            "/api/v1/licenses/activate",
            None,
            Some(json!({"license_key": key, "installation_id": self.id, "public_key": STANDARD.encode(self.key.public_key()),
                        "nonce": nonce, "signature": self.sign("activate", &nonce, Some(&format!("{hash}\n{hostname}\n{domain}"))),
                        "hostname": hostname, "domain": domain})),
            &[("x-forwarded-for", ip)],
        )
        .await
    }

    async fn ask(&self, env: &Env, what: &str) -> (StatusCode, Value) {
        self.ask_as(env, what, "web1", "panel.example", "203.0.113.7")
            .await
    }

    async fn ask_as(
        &self,
        env: &Env,
        what: &str,
        hostname: &str,
        domain: &str,
        ip: &str,
    ) -> (StatusCode, Value) {
        let nonce = self.nonce(env).await;
        call_with(
            &env.app,
            "POST",
            &format!("/api/v1/licenses/{what}"),
            None,
            Some(json!({"installation_id": self.id, "nonce": nonce,
                        "signature": self.sign(what, &nonce, Some(&format!("{hostname}\n{domain}"))),
                        "hostname": hostname, "domain": domain})),
            &[("x-forwarded-for", ip)],
        )
        .await
    }
}

async fn bundle(env: &Env) -> PublicKeys {
    let (status, text) = call(&env.app, "GET", "/api/v1/public-keys", None, None).await;
    assert_eq!(status, StatusCode::OK);
    PublicKeys::parse(text.as_str().unwrap()).unwrap()
}

fn expect(inst: Uuid) -> Expect<'static> {
    Expect {
        issuer: "builtpanel-la",
        audience: "builtpanel",
        installation_id: inst,
        clock_skew: 120,
        host: builtpanel_license::HostFacts {
            hostname: "web1",
            domain: "panel.example",
        },
    }
}

fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn administrators_sign_in_and_manage_customers_plans_and_licenses(pool: PgPool) {
    let e = env(&pool).await;
    let (status, _) = call(
        &e.app,
        "POST",
        "/api/v1/admin/login",
        None,
        Some(json!({"email": "nobody@example.com", "password": "whatever-whatever"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        call(&e.app, "GET", "/api/v1/admin/customers", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &e.app,
            "GET",
            "/api/v1/admin/customers",
            Some("not-a-token"),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let root = admin(&pool, &e, "root@example.com", "admin").await;
    let viewer = admin(&pool, &e, "look@example.com", "viewer").await;
    let (id, key) = license(&e, &root, 2, &["resellers"]).await;
    // The key is shown once, in its form; only a hash is kept.
    assert!(
        key.starts_with("DHP-") && key.len() == 4 + 5 * 4 + 3,
        "{key}"
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM licenses WHERE key_hint = $1")
        .bind(key.rsplit('-').next().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 1);
    let leaked: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT row_to_json(l)::text t FROM licenses l) x WHERE t LIKE '%' || $1 || '%'").bind(&key).fetch_one(&pool).await.unwrap();
    assert_eq!(leaked, 0, "the key itself is not stored");
    let (_, detail) = call(
        &e.app,
        "GET",
        &format!("/api/v1/admin/licenses/{id}"),
        Some(&root),
        None,
    )
    .await;
    assert_eq!(detail["license"]["status"], "active");
    assert!(detail["license"]["expires_at"].as_i64().unwrap() > now() + 300 * 86_400);
    // A viewer reads but changes nothing.
    assert_eq!(
        call(&e.app, "GET", "/api/v1/admin/licenses", Some(&viewer), None)
            .await
            .0,
        StatusCode::OK
    );
    let (status, got) = call(
        &e.app,
        "POST",
        &format!("/api/v1/admin/licenses/{id}/suspend"),
        Some(&viewer),
        None,
    )
    .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("viewer_cannot_change"))
    );
    // suspend, resume, revoke, renew.
    let (status, s) = call(
        &e.app,
        "POST",
        &format!("/api/v1/admin/licenses/{id}/suspend"),
        Some(&root),
        Some(json!({"note": "unpaid"})),
    )
    .await;
    assert_eq!(
        (status, s["status"].as_str(), s["status_note"].as_str()),
        (StatusCode::OK, Some("suspended"), Some("unpaid")),
        "{s}"
    );
    assert_eq!(
        call(
            &e.app,
            "POST",
            &format!("/api/v1/admin/licenses/{id}/suspend"),
            Some(&root),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &e.app,
            "POST",
            &format!("/api/v1/admin/licenses/{id}/resume"),
            Some(&root),
            None
        )
        .await
        .1["status"],
        "active"
    );
    let (_, before) = call(
        &e.app,
        "GET",
        &format!("/api/v1/admin/licenses/{id}"),
        Some(&root),
        None,
    )
    .await;
    let (_, renewed) = call(
        &e.app,
        "POST",
        &format!("/api/v1/admin/licenses/{id}/renew"),
        Some(&root),
        Some(json!({"days": 30})),
    )
    .await;
    assert_eq!(
        renewed["expires_at"].as_i64().unwrap() - before["license"]["expires_at"].as_i64().unwrap(),
        30 * 86_400
    );
    assert_eq!(
        call(
            &e.app,
            "POST",
            &format!("/api/v1/admin/licenses/{id}/renew"),
            Some(&root),
            Some(json!({"days": 30, "expires_at": 5}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, r) = call(
        &e.app,
        "POST",
        &format!("/api/v1/admin/licenses/{id}/revoke"),
        Some(&root),
        Some(json!({"note": "chargeback"})),
    )
    .await;
    assert_eq!(
        (status, r["status"].as_str()),
        (StatusCode::OK, Some("revoked"))
    );
    assert_eq!(
        call(
            &e.app,
            "POST",
            &format!("/api/v1/admin/licenses/{id}/renew"),
            Some(&root),
            Some(json!({"days": 30}))
        )
        .await
        .0,
        StatusCode::CONFLICT,
        "a revoked license stays revoked"
    );
    let revocations: i64 = sqlx::query_scalar("SELECT count(*) FROM revocations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(revocations, 1);
    // Bad input, a duplicate, the audit trail.
    assert_eq!(
        call(
            &e.app,
            "POST",
            "/api/v1/admin/customers",
            Some(&root),
            Some(json!({"name": "", "email": "x@y.z"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (_, c) = call(
        &e.app,
        "POST",
        "/api/v1/admin/customers",
        Some(&root),
        Some(json!({"name": "A", "email": "dup@acme.example"})),
    )
    .await;
    assert!(c["id"].is_string());
    assert_eq!(
        call(
            &e.app,
            "POST",
            "/api/v1/admin/customers",
            Some(&root),
            Some(json!({"name": "B", "email": "DUP@acme.example"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, audit) = call(&e.app, "GET", "/api/v1/admin/audit", Some(&root), None).await;
    let actions: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    for wanted in [
        "license.issued",
        "license.suspended",
        "license.revoked",
        "license.renewed",
        "admin.login",
    ] {
        assert!(actions.contains(&wanted), "{wanted} in {actions:?}");
    }
    // Signed out, the token stops working.
    assert_eq!(
        call(&e.app, "POST", "/api/v1/admin/logout", Some(&viewer), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &e.app,
            "GET",
            "/api/v1/admin/customers",
            Some(&viewer),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_installation_that_proves_its_key_gets_a_signed_entitlement_within_its_limit(
    pool: PgPool,
) {
    let e = env(&pool).await;
    let root = admin(&pool, &e, "root@example.com", "admin").await;
    let (license_id, key) = license(&e, &root, 1, &["resellers", "mail"]).await;
    let first = Installation::new(&e.key_dir);
    let (status, granted) = first.activate(&e, &key).await;
    assert_eq!(status, StatusCode::OK, "{granted}");
    let doc: Document = serde_json::from_value(granted["entitlement"].clone()).unwrap();
    let claims = doc
        .verify(&bundle(&e).await, &expect(first.id), now())
        .unwrap();
    assert_eq!(
        (
            claims.features.as_slice(),
            claims.limits["max_nodes"],
            claims.limits["max_tenants"]
        ),
        (
            ["resellers".to_owned(), "mail".to_owned()].as_slice(),
            3,
            -1
        )
    );
    assert_eq!(claims.license_id.to_string(), license_id);
    // For another installation it does not verify.
    assert!(
        doc.verify(&bundle(&e).await, &expect(Uuid::new_v4()), now())
            .is_err()
    );
    // The limit: a second installation is refused, the first may ask again (nothing changes).
    let second = Installation::new(&e.key_dir);
    let (status, refused) = second.activate(&e, &key).await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (StatusCode::CONFLICT, Some("activation_limit")),
        "{refused}"
    );
    assert_eq!(first.activate(&e, &key).await.0, StatusCode::OK);
    let holders: i64 =
        sqlx::query_scalar("SELECT count(*) FROM activations WHERE deactivated_at IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(holders, 1);
    // A wrong key, a wrong proof, a used or invented nonce.
    let (status, bad) = first.activate(&e, "DHP-AAAAA-BBBBB-CCCCC-DDDDD").await;
    assert_eq!(
        (status, bad["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("invalid_license_key"))
    );
    let nonce = second.nonce(&e).await;
    let (status, bad) = call(&e.app, "POST", "/api/v1/licenses/activate", None, Some(json!({"license_key": key, "installation_id": second.id,
        "public_key": STANDARD.encode(second.key.public_key()), "nonce": nonce, "signature": first.sign("activate", &nonce, None)}))).await;
    assert_eq!(
        (status, bad["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("bad_signature")),
        "{bad}"
    );
    let (status, bad) = call(&e.app, "POST", "/api/v1/licenses/activate", None, Some(json!({"license_key": key, "installation_id": second.id,
        "public_key": STANDARD.encode(second.key.public_key()), "nonce": nonce, "signature": second.sign("activate", &nonce, None)}))).await;
    assert_eq!(
        (status, bad["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("bad_nonce")),
        "the nonce was spent: {bad}"
    );
    // Somebody else's id with their own key is not allowed to take over an installation.
    let thief = Installation {
        id: first.id,
        key: FileSigningKey::generate_to_file(&e.key_dir.join("thief.key"), "x").unwrap(),
    };
    let (status, bad) = thief.activate(&e, &key).await;
    assert_eq!(
        (status, bad["code"].as_str()),
        (StatusCode::CONFLICT, Some("installation_key_mismatch")),
        "{bad}"
    );
    // Refresh, heartbeat and status work for the holder and not for a stranger.
    let (status, again) = first.ask(&e, "refresh").await;
    assert_eq!(status, StatusCode::OK, "{again}");
    let newer: Document = serde_json::from_value(again["entitlement"].clone()).unwrap();
    assert!(
        newer
            .verify(&bundle(&e).await, &expect(first.id), now())
            .is_ok()
    );
    assert_ne!(newer.payload, doc.payload, "a new jti every time");
    let (status, beat) = first.ask(&e, "heartbeat").await;
    assert_eq!(
        (status, beat["license_status"].as_str()),
        (StatusCode::OK, Some("active")),
        "{beat}"
    );
    let (status, state) = first.ask(&e, "status").await;
    assert_eq!(
        (status, state["activated"].as_bool()),
        (StatusCode::OK, Some(true))
    );
    let stranger = Installation::new(&e.key_dir);
    assert_eq!(
        stranger.ask(&e, "refresh").await.0,
        StatusCode::UNAUTHORIZED
    );
    // A proof made for one purpose is not good for another.
    let nonce = first.nonce(&e).await;
    let (status, bad) = call(&e.app, "POST", "/api/v1/licenses/refresh", None, Some(json!({"installation_id": first.id, "nonce": nonce, "signature": first.sign("heartbeat", &nonce, None)}))).await;
    assert_eq!(
        (status, bad["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("bad_signature"))
    );
    // Letting go frees the place for the other installation.
    assert_eq!(first.ask(&e, "deactivate").await.0, StatusCode::NO_CONTENT);
    assert_eq!(first.ask(&e, "refresh").await.1["code"], "not_activated");
    assert_eq!(second.activate(&e, &key).await.0, StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_revoked_suspended_or_expired_license_is_refused_and_two_racing_installations_cannot_both_win(
    pool: PgPool,
) {
    let e = env(&pool).await;
    let root = admin(&pool, &e, "root@example.com", "admin").await;
    let (id, key) = license(&e, &root, 1, &["x"]).await;
    let holder = Installation::new(&e.key_dir);
    assert_eq!(holder.activate(&e, &key).await.0, StatusCode::OK);
    call(
        &e.app,
        "POST",
        &format!("/api/v1/admin/licenses/{id}/suspend"),
        Some(&root),
        None,
    )
    .await;
    let (status, got) = holder.ask(&e, "refresh").await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("license_suspended"))
    );
    let (_, beat) = holder.ask(&e, "heartbeat").await;
    assert_eq!(beat["license_status"], "suspended");
    call(
        &e.app,
        "POST",
        &format!("/api/v1/admin/licenses/{id}/resume"),
        Some(&root),
        None,
    )
    .await;
    assert_eq!(holder.ask(&e, "refresh").await.0, StatusCode::OK);
    call(
        &e.app,
        "POST",
        &format!("/api/v1/admin/licenses/{id}/revoke"),
        Some(&root),
        None,
    )
    .await;
    assert_eq!(holder.ask(&e, "refresh").await.1["code"], "license_revoked");
    // Expired: set the end in the past.
    let (id2, key2) = license(&e, &root, 1, &["x"]).await;
    sqlx::query("UPDATE licenses SET expires_at = now() - interval '1 day' WHERE id = $1::uuid")
        .bind(&id2)
        .execute(&pool)
        .await
        .unwrap();
    let other = Installation::new(&e.key_dir);
    let (status, got) = other.activate(&e, &key2).await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("license_expired"))
    );
    // An entitlement never outlives its license: with two hours left it lasts two hours.
    sqlx::query("UPDATE licenses SET expires_at = now() + interval '2 hours' WHERE id = $1::uuid")
        .bind(&id2)
        .execute(&pool)
        .await
        .unwrap();
    let (status, granted) = other.activate(&e, &key2).await;
    assert_eq!(status, StatusCode::OK, "{granted}");
    let doc: Document = serde_json::from_value(granted["entitlement"].clone()).unwrap();
    let claims = doc
        .verify(&bundle(&e).await, &expect(other.id), now())
        .unwrap();
    assert!(claims.exp - now() <= 2 * 3600 + 5, "{}", claims.exp - now());
    // Two installations race for the one place: exactly one gets it.
    let (_, key3) = license(&e, &root, 1, &["x"]).await;
    let (a, b) = (Installation::new(&e.key_dir), Installation::new(&e.key_dir));
    let (ra, rb) = tokio::join!(a.activate(&e, &key3), b.activate(&e, &key3));
    let mut statuses = [ra.0, rb.0];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "{ra:?} {rb:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_private_key_is_never_in_the_database_and_a_replayed_activation_gets_the_same_answer(
    pool: PgPool,
) {
    let e = env(&pool).await;
    let root = admin(&pool, &e, "root@example.com", "admin").await;
    // A rotation makes a new key file; the database learns the public half only.
    let (status, info) = call(
        &e.app,
        "POST",
        "/api/v1/admin/keys/rotate",
        Some(&root),
        Some(json!({"kid": "license-2"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{info}");
    assert_eq!(
        call(
            &e.app,
            "POST",
            "/api/v1/admin/keys/rotate",
            Some(&root),
            Some(json!({"kid": "license-2"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, keys) = call(&e.app, "GET", "/api/v1/admin/keys", Some(&root), None).await;
    let active: Vec<&str> = keys
        .as_array()
        .unwrap()
        .iter()
        .filter(|k| k["status"] == "active")
        .map(|k| k["kid"].as_str().unwrap())
        .collect();
    assert_eq!(active, ["license-2"], "one active key, the new one");
    for file in ["license-1.key", "license-2.key"] {
        let private = std::fs::read_to_string(e.key_dir.join(file)).unwrap();
        let private = private.trim();
        let der = STANDARD.decode(private).unwrap();
        // Look in every table, as text, for the private key in any form it might be written.
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT tablename::text FROM pg_tables WHERE schemaname = 'public'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(tables.len() > 20, "{tables:?}");
        let seed = &der[16..48];
        let forms = [
            private.to_owned(),
            STANDARD.encode(seed),
            URL_SAFE_NO_PAD.encode(seed),
            seed.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        ];
        for table in &tables {
            let dump: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT COALESCE(string_agg(row_to_json(t)::text, ' '), '') FROM \"{table}\" t"
            )))
            .fetch_one(&pool)
            .await
            .unwrap();
            for form in &forms {
                assert!(
                    !dump.contains(form.as_str()),
                    "the private key of {file} is in {table}"
                );
            }
        }
    }
    // A documents signed after the rotation verify with the bundle (which has both keys).
    let (_, key) = license(&e, &root, 1, &["x"]).await;
    let inst = Installation::new(&e.key_dir);
    let (status, granted) = inst.activate(&e, &key).await;
    assert_eq!(status, StatusCode::OK, "{granted}");
    let doc: Document = serde_json::from_value(granted["entitlement"].clone()).unwrap();
    assert_eq!(doc.kid, "license-2");
    assert!(
        doc.verify(&bundle(&e).await, &expect(inst.id), now())
            .is_ok()
    );
    // The same activation sent twice with one Idempotency-Key is the same answer, the same document.
    let (_, key2) = license(&e, &root, 1, &["x"]).await;
    let other = Installation::new(&e.key_dir);
    let send = |nonce: String| {
        let hash: String = builtpanel_license_authority::sha256(&key2)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let body = json!({"license_key": key2, "installation_id": other.id, "public_key": STANDARD.encode(other.key.public_key()),
                          "nonce": nonce, "signature": other.sign("activate", &nonce, Some(&format!("{hash}\nweb1\npanel.example"))),
                          "hostname": "web1", "domain": "panel.example"});
        let app = e.app.clone();
        async move {
            call_with(
                &app,
                "POST",
                "/api/v1/licenses/activate",
                None,
                Some(body),
                &[
                    ("idempotency-key", "retry-key-0001"),
                    ("x-forwarded-for", "203.0.113.7"),
                ],
            )
            .await
        }
    };
    let (s1, r1) = send(other.nonce(&e).await).await;
    let (s2, r2) = send(other.nonce(&e).await).await;
    assert_eq!((s1, s2), (StatusCode::OK, StatusCode::OK));
    assert_eq!(r1, r2, "the replay is answered from the first");
    // The same key for a different request is refused.
    let third = Installation::new(&e.key_dir);
    let nonce = third.nonce(&e).await;
    let body = json!({"license_key": key2, "installation_id": third.id, "public_key": STANDARD.encode(third.key.public_key()), "nonce": nonce, "signature": third.sign("activate", &nonce, None)});
    let (status, got) = call_with(
        &e.app,
        "POST",
        "/api/v1/licenses/activate",
        None,
        Some(body),
        &[("idempotency-key", "retry-key-0001")],
    )
    .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("idempotency_key_reused"))
    );
}

/// A license works only where it is bound (the user's rule: domain, host name and address are
/// all assigned in the license). The address is read from the connection, so a request that
/// merely claims another one gets nowhere.
#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_license_is_bound_to_its_domain_host_name_and_address(pool: PgPool) {
    let e = env(&pool).await;
    let token = admin(&pool, &e, "root@la.example", "admin").await;
    let (id, key) = license(&e, &token, 2, &["mail"]).await;
    let inst = Installation::new(&e.key_dir);

    // Without all three lists a license cannot even be made.
    let (_, customer) = call(&e.app, "GET", "/api/v1/admin/customers", Some(&token), None).await;
    let _ = customer;
    for body in [
        json!({"domains": [], "hostnames": ["web1"], "ips": ["203.0.113.7"]}),
        json!({"domains": ["panel.example"], "hostnames": [], "ips": ["203.0.113.7"]}),
        json!({"domains": ["panel.example"], "hostnames": ["web1"], "ips": ["203.0.113.0/24"]}),
    ] {
        let (status, got) = call(
            &e.app,
            "PUT",
            &format!("/api/v1/admin/licenses/{id}/binding"),
            Some(&token),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{got}");
    }

    // Another address, host name or domain: refused, and each refusal is written down.
    for (host, domain, ip, why) in [
        ("web1", "panel.example", "198.51.100.9", "ip"),
        ("web2", "panel.example", "203.0.113.7", "hostname"),
        ("web1", "other.example", "203.0.113.7", "domain"),
    ] {
        let (status, got) = inst.activate_as(&e, &key, host, domain, ip).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{why}: {got}");
        assert_eq!(got["code"], "binding_mismatch", "{why}: {got}");
    }
    let refused: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'license.binding_refused'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(refused, 3);
    // Nothing was activated by those.
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM activations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(held, 0);

    // Where it is bound, it works, and the entitlement says where.
    let (status, got) = inst.activate(&e, &key).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    let doc: Document = serde_json::from_value(got["entitlement"].clone()).unwrap();
    let claims = doc
        .verify(&bundle(&e).await, &expect(inst.id), now())
        .unwrap();
    assert_eq!(claims.bind.domains, vec!["panel.example"]);
    assert_eq!(claims.bind.hostnames, vec!["web1"]);
    assert_eq!(claims.bind.ips, vec!["203.0.113.7"]);
    // An installation that is elsewhere cannot verify it either.
    let elsewhere = Expect {
        host: builtpanel_license::HostFacts {
            hostname: "web9",
            domain: "panel.example",
        },
        ..expect(inst.id)
    };
    assert!(doc.verify(&bundle(&e).await, &elsewhere, now()).is_err());

    // Refresh and heartbeat are checked every time: moved to another address, no entitlement
    // and the installation is told why; an event is kept against the installation.
    for what in ["refresh", "heartbeat"] {
        let (status, got) = inst.ask(&e, what).await;
        assert_eq!(status, StatusCode::OK, "{what}: {got}");
        let (status, got) = inst
            .ask_as(&e, what, "web1", "panel.example", "198.51.100.9")
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{what}: {got}");
        assert_eq!(got["code"], "binding_mismatch");
    }
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM integrity_events WHERE kind = 'binding' AND installation_id = $1",
    )
    .bind(inst.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(events, 2);

    // A signature does not cover a host name that was changed on the way.
    let nonce = inst.nonce(&e).await;
    let (status, _) = call_with(
        &e.app,
        "POST",
        "/api/v1/licenses/refresh",
        None,
        Some(json!({"installation_id": inst.id, "nonce": nonce,
                    "signature": inst.sign("refresh", &nonce, Some("web1\npanel.example")),
                    "hostname": "web2", "domain": "panel.example"})),
        &[("x-forwarded-for", "203.0.113.7")],
    )
    .await;
    assert_ne!(status, StatusCode::OK);

    // The administrator can move the license; the old place stops working at the next refresh.
    let (status, got) = call(
        &e.app,
        "PUT",
        &format!("/api/v1/admin/licenses/{id}/binding"),
        Some(&token),
        Some(json!({"domains": ["new.example"], "hostnames": ["web2"], "ips": ["198.51.100.9", "2001:db8::9"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got["bound_ips"], json!(["198.51.100.9", "2001:db8::9"]));
    let (status, _) = inst.ask(&e, "refresh").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, got) = inst
        .ask_as(&e, "refresh", "web2", "new.example", "2001:db8::9")
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");

    // A header from a stranger's connection is not believed: the connection decides.
    let nonce = inst.nonce(&e).await;
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/licenses/refresh")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", "198.51.100.9")
        .body(Body::from(
            json!({"installation_id": inst.id, "nonce": nonce,
                   "signature": inst.sign("refresh", &nonce, Some("web2\nnew.example")),
                   "hostname": "web2", "domain": "new.example"})
            .to_string(),
        ))
        .unwrap();
    req.extensions_mut().insert(axum::extract::ConnectInfo(
        "192.0.2.50:4444".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let res = e.app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "the claimed address is not the connection's"
    );
}

/// Releases and the daily scan (T17.14, T17.16): a manifest signed with the release key is
/// published, an installation that reports the hashes of what it runs passes, and one binary
/// that is not the release's makes the scan fail and leaves an event.
#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_changed_binary_fails_the_next_scan_and_leaves_an_event(pool: PgPool) {
    use builtpanel_license::{ReleaseManifest, SignedManifest, integrity_report_text, sha256_hex};
    let e = env(&pool).await;
    let token = admin(&pool, &e, "root@la.example", "admin").await;
    let (_, key) = license(&e, &token, 1, &["mail"]).await;
    let inst = Installation::new(&e.key_dir);
    let (status, got) = inst.activate(&e, &key).await;
    assert_eq!(status, StatusCode::OK, "{got}");

    // The release: two programs, signed by a release key the authority knows.
    let release_key =
        FileSigningKey::generate_to_file(&e.key_dir.join("release-1.key"), "release-1").unwrap();
    let files = vec![
        (
            "builtpanel-control-api".to_owned(),
            b"control api 1.0".to_vec(),
        ),
        ("builtpanel-worker".to_owned(), b"worker 1.0".to_vec()),
    ];
    let manifest = ReleaseManifest::build("1.0.0", "abcdef0", 1_700_000_000, &files).unwrap();
    let signed = SignedManifest::sign(&manifest, &release_key).unwrap();
    // Nobody trusts the key yet: not published.
    let publish = |signed: &SignedManifest| {
        call(
            &e.app,
            "POST",
            "/api/v1/admin/releases",
            Some(&token),
            Some(json!({"manifest": signed})),
        )
    };
    let (status, _) = publish(&signed).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, got) = call(
        &e.app,
        "POST",
        "/api/v1/admin/release-keys",
        Some(&token),
        Some(json!({"kid": "release-1", "public_key": STANDARD.encode(release_key.public_key())})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{got}");
    // A changed manifest is not published; the real one is, once.
    let mut changed = signed.clone();
    changed.payload.push('A');
    assert_eq!(publish(&changed).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, got) = publish(&signed).await;
    assert_eq!(status, StatusCode::CREATED, "{got}");
    assert_eq!(publish(&signed).await.0, StatusCode::CONFLICT);
    let (status, doc) = call(&e.app, "GET", "/api/v1/releases/latest", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["kid"], "release-1");

    // One scan: ask, hash, report.
    let scan = |version: &'static str,
                files: std::collections::BTreeMap<String, String>,
                sign_for: Option<Uuid>| {
        let (e, inst, token) = (&e, &inst, &token);
        async move {
            let nonce = inst.nonce(e).await;
            let (status, c) = call_with(
                &e.app,
                "POST",
                "/api/v1/integrity/challenge",
                None,
                Some(json!({"installation_id": inst.id, "nonce": nonce, "version": version,
                            "signature": inst.sign("integrity", &nonce, Some("web1\npanel.example")),
                            "hostname": "web1", "domain": "panel.example"})),
                &[("x-forwarded-for", "203.0.113.7")],
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{c}");
            let scan_id: Uuid = c["scan_id"].as_str().unwrap().parse().unwrap();
            let scan_nonce = c["nonce"].as_str().unwrap().to_owned();
            let text = integrity_report_text(&sign_for.unwrap_or(scan_id).to_string(), &files);
            let signature = inst.sign("integrity-report", &scan_nonce, Some(&text));
            let (status, r) = call(
                &e.app,
                "POST",
                "/api/v1/integrity/report",
                None,
                Some(json!({"installation_id": inst.id, "scan_id": scan_id, "files": files, "signature": signature})),
            )
            .await;
            let _ = token;
            (c, scan_id, status, r)
        }
    };
    let honest = |files: &[(String, Vec<u8>)]| -> std::collections::BTreeMap<String, String> {
        files
            .iter()
            .map(|(n, b)| (n.clone(), sha256_hex(b)))
            .collect()
    };
    let (c, _, status, r) = scan("1.0.0", honest(&files), None).await;
    assert_eq!(
        c["components"],
        json!(["builtpanel-control-api", "builtpanel-worker"])
    );
    assert_eq!(
        (status, r["result"].clone()),
        (StatusCode::OK, json!("passed")),
        "{r}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM integrity_events WHERE kind <> 'binding'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );

    // One byte of one program is not the release's: failed, with an event naming the file.
    let mut patched = files.clone();
    patched[0].1[3] ^= 1;
    let (_, scan_id, status, r) = scan("1.0.0", honest(&patched), None).await;
    assert_eq!(
        (status, r["result"].clone()),
        (StatusCode::OK, json!("failed")),
        "{r}"
    );
    assert_eq!(r["mismatched"], json!(["builtpanel-control-api"]));
    let (kind, detail): (String, Value) =
        sqlx::query_as("SELECT kind, detail FROM integrity_events WHERE scan_id = $1")
            .bind(scan_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kind, "mismatch");
    assert_eq!(detail["mismatched"], json!(["builtpanel-control-api"]));
    let (status, listed) = call(
        &e.app,
        "GET",
        &format!("/api/v1/admin/integrity/events?installation_id={}", inst.id),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");

    // A program that is not there: partial. A version nobody published: unverifiable.
    let mut fewer = honest(&files);
    fewer.remove("builtpanel-worker");
    let (_, _, _, r) = scan("1.0.0", fewer, None).await;
    assert_eq!(r["result"], "partial");
    assert_eq!(r["missing"], json!(["builtpanel-worker"]));
    let (c, _, _, r) = scan("9.9.9", honest(&files), None).await;
    assert_eq!(c["components"], json!([]));
    assert_eq!(r["result"], "unverifiable");

    // A report signed for another scan is not accepted, and a scan is answered once.
    let (_, _, status, _) = scan("1.0.0", honest(&files), Some(Uuid::new_v4())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let nonce = inst.nonce(&e).await;
    let (_, c) = call_with(
        &e.app,
        "POST",
        "/api/v1/integrity/challenge",
        None,
        Some(
            json!({"installation_id": inst.id, "nonce": nonce, "version": "1.0.0",
                    "signature": inst.sign("integrity", &nonce, Some("web1\npanel.example")),
                    "hostname": "web1", "domain": "panel.example"}),
        ),
        &[("x-forwarded-for", "203.0.113.7")],
    )
    .await;
    let scan_id: Uuid = c["scan_id"].as_str().unwrap().parse().unwrap();
    let files_ok = honest(&files);
    let signature = inst.sign(
        "integrity-report",
        c["nonce"].as_str().unwrap(),
        Some(&integrity_report_text(&scan_id.to_string(), &files_ok)),
    );
    let body = json!({"installation_id": inst.id, "scan_id": scan_id, "files": files_ok, "signature": signature});
    assert_eq!(
        call(
            &e.app,
            "POST",
            "/api/v1/integrity/report",
            None,
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&e.app, "POST", "/api/v1/integrity/report", None, Some(body))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

/// Integrity events and recovery (T17.17): a failed scan takes the license away until an
/// administrator has looked; an approved recovery ends with a clean scan; every step is
/// written to the audit log; a viewer cannot decide.
#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_failed_scan_takes_the_license_away_until_a_recovery_is_approved_and_the_scan_is_clean(
    pool: PgPool,
) {
    use builtpanel_license::{ReleaseManifest, SignedManifest, integrity_report_text, sha256_hex};
    let e = env(&pool).await;
    let token = admin(&pool, &e, "root@la.example", "admin").await;
    let viewer = admin(&pool, &e, "viewer@la.example", "viewer").await;
    let (_, key) = license(&e, &token, 1, &["mail"]).await;
    let inst = Installation::new(&e.key_dir);
    assert_eq!(inst.activate(&e, &key).await.0, StatusCode::OK);

    // A published release.
    let release_key =
        FileSigningKey::generate_to_file(&e.key_dir.join("release-1.key"), "release-1").unwrap();
    let files = vec![("builtpanel-control-api".to_owned(), b"control api".to_vec())];
    let signed = SignedManifest::sign(
        &ReleaseManifest::build("1.0.0", "abcdef0", 1, &files).unwrap(),
        &release_key,
    )
    .unwrap();
    call(
        &e.app,
        "POST",
        "/api/v1/admin/release-keys",
        Some(&token),
        Some(json!({"kid": "release-1", "public_key": STANDARD.encode(release_key.public_key())})),
    )
    .await;
    assert_eq!(
        call(
            &e.app,
            "POST",
            "/api/v1/admin/releases",
            Some(&token),
            Some(json!({"manifest": signed}))
        )
        .await
        .0,
        StatusCode::CREATED
    );

    // One scan with the given file contents; returns the answer.
    let scan = |content: &'static [u8]| {
        let (e, inst) = (&e, &inst);
        async move {
            let nonce = inst.nonce(e).await;
            let (_, c) = call_with(
                &e.app,
                "POST",
                "/api/v1/integrity/challenge",
                None,
                Some(json!({"installation_id": inst.id, "nonce": nonce, "version": "1.0.0",
                            "signature": inst.sign("integrity", &nonce, Some("web1\npanel.example")),
                            "hostname": "web1", "domain": "panel.example"})),
                &[("x-forwarded-for", "203.0.113.7")],
            )
            .await;
            let scan_id: Uuid = c["scan_id"].as_str().unwrap().parse().unwrap();
            let found: std::collections::BTreeMap<String, String> =
                [("builtpanel-control-api".to_owned(), sha256_hex(content))].into();
            let signature = inst.sign(
                "integrity-report",
                c["nonce"].as_str().unwrap(),
                Some(&integrity_report_text(&scan_id.to_string(), &found)),
            );
            call(&e.app, "POST", "/api/v1/integrity/report", None, Some(json!({"installation_id": inst.id, "scan_id": scan_id, "files": found, "signature": signature}))).await
        }
    };
    let audited = |action: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_logs WHERE action = $1")
                .bind(action)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };

    // All is well; the license refreshes.
    assert_eq!(scan(b"control api").await.1["integrity"], "ok");
    assert_eq!(inst.ask(&e, "refresh").await.0, StatusCode::OK);

    // The program is changed: the scan fails, the license is refused, and it cannot be given up
    // to start over under another identity.
    let (_, r) = scan(b"control api, patched").await;
    assert_eq!(
        (r["result"].as_str(), r["integrity"].as_str()),
        (Some("failed"), Some("failed")),
        "{r}"
    );
    let (status, got) = inst.ask(&e, "refresh").await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("integrity_failed")),
        "{got}"
    );
    assert_eq!(inst.ask(&e, "deactivate").await.0, StatusCode::FORBIDDEN);
    let (_, beat) = inst.ask(&e, "heartbeat").await;
    assert_eq!(
        beat["integrity"], "failed",
        "a heartbeat still gets through and says so"
    );
    assert_eq!(audited("integrity.failed").await, 1);
    // A clean scan alone does not clear it: somebody has to look.
    let (_, r) = scan(b"control api").await;
    assert_eq!(
        (r["result"].as_str(), r["integrity"].as_str()),
        (Some("passed"), Some("failed")),
        "{r}"
    );
    assert_eq!(inst.ask(&e, "refresh").await.0, StatusCode::FORBIDDEN);

    // The installation asks for help; a viewer cannot decide, an administrator can.
    let nonce = inst.nonce(&e).await;
    let (status, _) = call_with(
        &e.app,
        "POST",
        "/api/v1/licenses/recovery-request",
        None,
        Some(json!({"installation_id": inst.id, "nonce": nonce,
                    "signature": inst.sign("recovery", &nonce, Some("web1\npanel.example")),
                    "hostname": "web1", "domain": "panel.example", "note": "reinstalled from the release"})),
        &[("x-forwarded-for", "203.0.113.7")],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (_, requests) = call(
        &e.app,
        "GET",
        "/api/v1/admin/recovery-requests?state=requested",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(requests.as_array().unwrap().len(), 1, "{requests}");
    assert_eq!(requests[0]["integrity_state"], "failed");
    let request_id = requests[0]["id"].as_str().unwrap().to_owned();
    let approve = format!("/api/v1/admin/recovery-requests/{request_id}/approve");
    assert_eq!(
        call(&e.app, "POST", &approve, Some(&viewer), Some(json!({})))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &e.app,
            "POST",
            &approve,
            Some(&token),
            Some(json!({"note": "checked with the customer"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&e.app, "POST", &approve, Some(&token), Some(json!({})))
            .await
            .0,
        StatusCode::CONFLICT,
        "decided once"
    );
    assert_eq!(audited("recovery.approved").await, 1);
    // Approved is not yet cleared: the next scan decides. Still patched: failed again.
    assert_eq!(inst.ask(&e, "refresh").await.0, StatusCode::FORBIDDEN);
    let (_, r) = scan(b"control api, patched again").await;
    assert_eq!(r["integrity"], "failed");

    // Recovery again, this time a clean scan after the approval puts everything right.
    let (_, listed) = call(
        &e.app,
        "GET",
        "/api/v1/admin/recovery-requests",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(listed[0]["state"], "approved");
    let nonce = inst.nonce(&e).await;
    call_with(&e.app, "POST", "/api/v1/licenses/recovery-request", None, Some(json!({"installation_id": inst.id, "nonce": nonce,
        "signature": inst.sign("recovery", &nonce, Some("web1\npanel.example")), "hostname": "web1", "domain": "panel.example", "note": "again"})), &[("x-forwarded-for", "203.0.113.7")]).await;
    let (_, requests) = call(
        &e.app,
        "GET",
        "/api/v1/admin/recovery-requests?state=requested",
        Some(&token),
        None,
    )
    .await;
    let second = requests[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        call(
            &e.app,
            "POST",
            &format!("/api/v1/admin/recovery-requests/{second}/approve"),
            Some(&token),
            Some(json!({}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (_, r) = scan(b"control api").await;
    assert_eq!(
        (r["result"].as_str(), r["integrity"].as_str()),
        (Some("passed"), Some("ok")),
        "{r}"
    );
    assert_eq!(
        inst.ask(&e, "refresh").await.0,
        StatusCode::OK,
        "the license is back"
    );
    assert_eq!(audited("integrity.recovered").await, 1);
    let (_, listed) = call(
        &e.app,
        "GET",
        "/api/v1/admin/recovery-requests?state=completed",
        Some(&token),
        None,
    )
    .await;
    // Both approved requests end with the clean scan.
    assert_eq!(listed.as_array().unwrap().len(), 2, "{listed}");
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM integrity_events ORDER BY created_at")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        kinds.contains(&"recovered".to_owned()) && kinds.contains(&"mismatch".to_owned()),
        "{kinds:?}"
    );

    // A false alarm is dismissed at once, and rejected requests stay rejected.
    let (_, r) = scan(b"control api, oops").await;
    assert_eq!(r["integrity"], "failed");
    assert_eq!(inst.ask(&e, "refresh").await.0, StatusCode::FORBIDDEN);
    let dismiss = format!("/api/v1/admin/installations/{}/integrity/dismiss", inst.id);
    assert_eq!(
        call(&e.app, "POST", &dismiss, Some(&viewer), Some(json!({})))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &e.app,
            "POST",
            &dismiss,
            Some(&token),
            Some(json!({"note": "false alarm, a backup was restored"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&e.app, "POST", &dismiss, Some(&token), Some(json!({})))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(inst.ask(&e, "refresh").await.0, StatusCode::OK);
    assert_eq!(audited("integrity.dismissed").await, 1);
}
