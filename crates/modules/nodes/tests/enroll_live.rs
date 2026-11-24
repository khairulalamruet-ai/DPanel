//! Node enrollment over HTTP, against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Request, StatusCode, header},
};
use builtpanel_cache::{
    RedisHandle,
    lockout::{FailureLimiter, LockoutPolicy},
};
use builtpanel_database::{node_enrollments::insert_token, rbac::assign_role};
use builtpanel_mod_nodes::{ENROLL_MAX_FAILURES, NodesState, router, token};
use builtpanel_pki::CertificateAuthority;
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tower::ServiceExt as _;
use uuid::Uuid;
use x509_parser::prelude::{FromDer as _, GeneralName, X509Certificate};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const ENROLL: &str = "/api/v1/public/nodes/enroll";

struct Env {
    app: Router,
    sessions: SessionStore,
    pool: PgPool,
    ca: Option<Arc<CertificateAuthority>>,
}

fn env(pool: &PgPool, with_ca: bool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis.clone(), pool.clone(), SessionConfig::default());
    // A private counter namespace per test, so tests never share lockouts.
    let limiter = FailureLimiter::new(
        redis,
        format!("test:{}:enroll", Uuid::now_v7().simple()),
        LockoutPolicy {
            threshold: ENROLL_MAX_FAILURES,
            window: Duration::from_secs(600),
            lockout: Duration::from_secs(900),
        },
    );
    let ca = with_ca.then(|| {
        Arc::new(
            CertificateAuthority::create(
                "Test CA",
                OffsetDateTime::now_utc(),
                time::Duration::days(3650),
            )
            .unwrap()
            .0,
        )
    });
    let app = router()
        .with_state(NodesState {
            db: pool.clone(),
            sessions: sessions.clone(),
            public_url: Some(Arc::from("https://panel.example.com:8443")),
            stream_url: None,
            token_ttl: Duration::from_secs(900),
            ca: ca.clone(),
            limiter,
            gateway: None,
            operations: builtpanel_operations::Operations::new(pool.clone()),
            audit: builtpanel_audit::AuditWriter::new(pool.clone()),
        })
        .layer(axum::middleware::from_fn_with_state(
            Arc::<[std::net::IpAddr]>::from(Vec::new()),
            builtpanel_http_kit::client_ip::client_ip_layer,
        ))
        .layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 9], 5000))));
    Env {
        app,
        sessions,
        pool: pool.clone(),
        ca,
    }
}

impl Env {
    async fn post(
        &self,
        path: &str,
        body: Value,
        cookie: Option<(&str, &str)>,
    ) -> (StatusCode, Value, Option<String>, Option<String>) {
        let mut req = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some((c, csrf)) = cookie {
            req = req.header(header::COOKIE, c).header(CSRF_HEADER, csrf);
        }
        let res = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let cache = res
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap().to_owned());
        let retry = res
            .headers()
            .get(header::RETRY_AFTER)
            .map(|v| v.to_str().unwrap().to_owned());
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            cache,
            retry,
        )
    }

    async fn enrol(&self, token: &str, csr: &str) -> (StatusCode, Value) {
        let (s, b, _, _) = self.post(ENROLL, json!({"token": token, "csr_pem": csr, "hostname": "node1.example.com", "agent_version": "0.1.0"}), None).await;
        (s, b)
    }

    /// A pending token for `name`, created straight in the database.
    async fn token(&self, name: &str) -> String {
        let (secret, hash) = token::generate().unwrap();
        let admin = admin(&self.pool, &format!("by-{name}"), &[]).await;
        insert_token(
            &mut self.pool.acquire().await.unwrap(),
            name,
            &hash,
            admin,
            900,
        )
        .await
        .unwrap();
        secret.expose_secret().to_owned()
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
        assign_role(pool, id, role, None).await.unwrap();
    }
    id
}

/// The agent side: a fresh P-256 key and its CSR. The CSR is greedy on purpose.
fn greedy_csr() -> (KeyPair, String) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["evil.example.com".to_owned()]).unwrap();
    params.distinguished_name.push(DnType::CommonName, "root");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let csr = params.serialize_request(&key).unwrap().pem().unwrap();
    (key, csr)
}

fn pem_der(pem: &str) -> Vec<u8> {
    x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .unwrap()
        .1
        .contents
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    // Literal queries only: sqlx refuses SQL assembled at run time.
    let query = match table {
        "nodes" => "SELECT count(*) FROM nodes",
        "node_certificates" => "SELECT count(*) FROM node_certificates",
        other => panic!("count: unknown table {other}"),
    };
    sqlx::query_scalar(query).fetch_one(pool).await.unwrap()
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn an_agent_enrols_and_gets_an_identity_the_ca_vouches_for(pool: PgPool) {
    let e = env(&pool, true);
    let secret = e.token("eu-west-1").await;
    let (agent_key, csr) = greedy_csr();

    let (status, body, cache, _) = e
        .post(ENROLL, json!({"token": secret, "csr_pem": csr, "hostname": "node1.example.com", "agent_version": "0.1.0"}), None)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(cache.as_deref(), Some("no-store"));
    let node_id: Uuid = body["node_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(body["name"], "eu-west-1");
    assert_eq!(body["protocol_version"], 1);

    // The certificate: ours, not what the greedy request asked for.
    let der = pem_der(body["certificate_pem"].as_str().unwrap());
    let cert = X509Certificate::from_der(&der).unwrap().1;
    assert_eq!(
        cert.subject()
            .iter_common_name()
            .next()
            .unwrap()
            .as_str()
            .unwrap(),
        format!("node-{node_id}")
    );
    assert!(!cert.is_ca());
    let eku = cert.extended_key_usage().unwrap().unwrap().value;
    assert!(eku.client_auth && !eku.server_auth);
    let sans = &cert
        .subject_alternative_name()
        .unwrap()
        .unwrap()
        .value
        .general_names;
    assert_eq!(sans.len(), 1);
    assert!(
        matches!(&sans[0], GeneralName::URI(u) if *u == format!("urn:builtpanel:node:{node_id}"))
    );
    assert_eq!(
        cert.public_key().subject_public_key.data.as_ref(),
        agent_key.public_key_raw()
    );
    // Signed by the CA that the response hands over, whose fingerprint is given too.
    let ca_der = pem_der(body["ca_certificate_pem"].as_str().unwrap());
    cert.verify_signature(Some(
        X509Certificate::from_der(&ca_der).unwrap().1.public_key(),
    ))
    .expect("signed by the CA");
    assert_eq!(
        body["ca_fingerprint_sha256"],
        e.ca.as_ref().unwrap().fingerprint_sha256()
    );

    // What was recorded.
    let (name, hostname, ip, version, status): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    ) = sqlx::query_as(
        "SELECT name, hostname, host(public_ip), agent_version, status FROM nodes WHERE id = $1",
    )
    .bind(node_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (
            name.as_str(),
            hostname.as_deref(),
            ip.as_deref(),
            version.as_deref(),
            status.as_str()
        ),
        (
            "eu-west-1",
            Some("node1.example.com"),
            Some("203.0.113.9"),
            Some("0.1.0"),
            "offline"
        )
    );
    let (serial, fingerprint): (String, String) = sqlx::query_as(
        "SELECT serial, fingerprint_sha256 FROM node_certificates WHERE node_id = $1",
    )
    .bind(node_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fingerprint.len(), 64);
    assert_eq!(
        serial,
        cert.serial
            .to_bytes_be()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let (token_node, consumed): (Option<Uuid>, bool) =
        sqlx::query_as("SELECT node_id, consumed_at IS NOT NULL FROM node_enrollment_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((token_node, consumed), (Some(node_id), true));
    // The handshake lookup (T02.08) finds the node by the certificate fingerprint.
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT n.id FROM node_certificates c JOIN nodes n ON n.id = c.node_id
         WHERE c.fingerprint_sha256 = $1 AND c.revoked_at IS NULL AND c.not_after > now() AND n.status <> 'revoked'",
    )
    .bind(&fingerprint)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(found, Some(node_id));

    // The audit entry names the node and the certificate, but not the token.
    let (action, outcome, actor_ip, meta): (String, String, Option<String>, Value) = sqlx::query_as(
        "SELECT action, outcome, host(actor_ip), metadata FROM audit_logs WHERE action = 'node.enrolled'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (action.as_str(), outcome.as_str(), actor_ip.as_deref()),
        ("node.enrolled", "success", Some("203.0.113.9"))
    );
    assert_eq!(meta["name"], "eu-west-1");
    assert_eq!(meta["certificate_fingerprint"], fingerprint.as_str());
    let everything: String = sqlx::query_scalar(
        "SELECT COALESCE(string_agg(x, ' '), '') FROM (
           SELECT row_to_json(a)::text x FROM audit_logs a UNION ALL SELECT row_to_json(t)::text FROM node_enrollment_tokens t
           UNION ALL SELECT row_to_json(n)::text FROM nodes n) q",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!everything.contains(&secret), "token text stored somewhere");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_token_cannot_be_used_to_enrol_twice(pool: PgPool) {
    let e = env(&pool, true);
    let secret = e.token("node-1").await;
    let (_, csr) = greedy_csr();
    assert_eq!(e.enrol(&secret, &csr).await.0, StatusCode::CREATED);
    for _ in 0..2 {
        let (_, other_csr) = greedy_csr();
        let (status, body) = e.enrol(&secret, &other_csr).await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::UNAUTHORIZED, "invalid_enrollment_token")
        );
    }
    assert_eq!(count(&pool, "nodes").await, 1);
    assert_eq!(count(&pool, "node_certificates").await, 1);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn every_kind_of_bad_token_gets_the_same_answer(pool: PgPool) {
    let e = env(&pool, true);
    let (_, csr) = greedy_csr();
    let (unknown, _) = token::generate().unwrap();

    let revoked = e.token("node-r").await;
    sqlx::query("UPDATE node_enrollment_tokens SET revoked_at = now() WHERE name = 'node-r'")
        .execute(&pool)
        .await
        .unwrap();
    let expired = e.token("node-e").await;
    sqlx::query("UPDATE node_enrollment_tokens SET created_at = now() - interval '2 hours', expires_at = now() - interval '1 hour' WHERE name = 'node-e'")
        .execute(&pool)
        .await
        .unwrap();
    let used = e.token("node-u").await;
    assert_eq!(e.enrol(&used, &csr).await.0, StatusCode::CREATED);

    let mut answers = Vec::new();
    for bad in [
        unknown.expose_secret().to_owned(),
        revoked,
        expired,
        used,
        "dhn_short".into(),
        String::new(),
        "x".repeat(300),
        "not a token at all".into(),
    ] {
        let (status, body) = e.enrol(&bad, &csr).await;
        answers.push((status, body.clone()));
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{bad:.20}");
    }
    let first = &answers[0].1;
    for (_, body) in &answers {
        assert_eq!(body["code"], first["code"]);
        assert_eq!(body["detail"], first["detail"]);
        assert_eq!(body["title"], first["title"]);
    }
    assert_eq!(
        count(&pool, "nodes").await,
        1,
        "only the valid enrolment created a node"
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_bad_request_does_not_use_up_the_token(pool: PgPool) {
    let e = env(&pool, true);
    let secret = e.token("node-1").await;
    let (_, good_csr) = greedy_csr();

    let (status, body) = e.enrol(&secret, "not a csr").await;
    assert_eq!(
        (status, body["errors"][0]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_csr"))
    );
    let ed = KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let ed_csr = CertificateParams::new(Vec::<String>::new())
        .unwrap()
        .serialize_request(&ed)
        .unwrap()
        .pem()
        .unwrap();
    let (status, body) = e.enrol(&secret, &ed_csr).await;
    assert_eq!(
        (status, body["errors"][0]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("unsupported_key"))
    );
    let (status, body, _, _) = e
        .post(
            ENROLL,
            json!({"token": secret, "csr_pem": good_csr, "hostname": "bad host!"}),
            None,
        )
        .await;
    assert_eq!(
        (status, body["errors"][0]["field"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("hostname"))
    );
    let (status, body) = e.enrol(&secret, &"c".repeat(9000)).await;
    assert_eq!(
        (status, body["errors"][0]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("too_long"))
    );
    assert_eq!(count(&pool, "nodes").await, 0);

    // Still pending: the same token works with a proper request.
    assert_eq!(e.enrol(&secret, &good_csr).await.0, StatusCode::CREATED);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_name_clash_leaves_the_token_usable(pool: PgPool) {
    let e = env(&pool, true);
    let secret = e.token("node-1").await;
    let (_, csr) = greedy_csr();
    // A node took the name after the token was issued.
    sqlx::query("INSERT INTO nodes (name) VALUES ('node-1')")
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = e.enrol(&secret, &csr).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "node_name_taken")
    );
    let pending: bool = sqlx::query_scalar(
        "SELECT consumed_at IS NULL AND revoked_at IS NULL FROM node_enrollment_tokens",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(pending, "the failed enrolment rolled back, token untouched");
    assert_eq!(count(&pool, "node_certificates").await, 0);

    sqlx::query("DELETE FROM nodes WHERE name = 'node-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(e.enrol(&secret, &csr).await.0, StatusCode::CREATED);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn repeated_failures_lock_the_address_out_before_any_work(pool: PgPool) {
    let e = env(&pool, true);
    let valid = e.token("node-1").await;
    let (_, csr) = greedy_csr();
    let (junk, _) = token::generate().unwrap();

    for attempt in 1..=ENROLL_MAX_FAILURES {
        let (status, _) = e.enrol(junk.expose_secret(), &csr).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "attempt {attempt}");
    }
    // The next request is refused outright, even with a valid token and a good request.
    let (status, body, _, retry) = e
        .post(ENROLL, json!({"token": valid, "csr_pem": csr}), None)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::TOO_MANY_REQUESTS, "too_many_attempts")
    );
    let retry: u64 = retry.expect("Retry-After").parse().unwrap();
    assert!((1..=900).contains(&retry), "{retry}");
    assert_eq!(count(&pool, "nodes").await, 0);
    let pending: bool =
        sqlx::query_scalar("SELECT consumed_at IS NULL FROM node_enrollment_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(pending, "a locked-out request does not touch the token");

    // Every counted failure was audited; the refusals were not (no amplification).
    let failures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'node.enrollment_failed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(failures, i64::from(ENROLL_MAX_FAILURES));
    let (outcome, reason): (String, String) =
        sqlx::query_as("SELECT outcome, metadata->>'reason' FROM audit_logs LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (outcome.as_str(), reason.as_str()),
        ("denied", "invalid_token")
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn without_a_ca_enrollment_is_switched_off_and_tokens_survive(pool: PgPool) {
    let e = env(&pool, false);
    let secret = e.token("node-1").await;
    let (_, csr) = greedy_csr();
    let (status, body) = e.enrol(&secret, &csr).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::SERVICE_UNAVAILABLE, "enrollment_unavailable")
    );
    let pending: bool =
        sqlx::query_scalar("SELECT consumed_at IS NULL FROM node_enrollment_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(pending);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_whole_journey_from_the_admin_screen_to_an_enrolled_node(pool: PgPool) {
    let e = env(&pool, true);
    let root = admin(&pool, "rootuser", &["super_admin"]).await;
    let (t, data) = e
        .sessions
        .create(NewSession {
            account_id: root,
            ip: None,
            user_agent: None,
        })
        .await
        .unwrap();
    let cookie = set_cookie(&t).split(';').next().unwrap().to_owned();
    let csrf = data.csrf_token.expose().to_owned();

    // The administrator adds a node and gets the command, which pins the CA.
    let (status, created, _, _) = e
        .post(
            "/api/v1/admin/nodes",
            json!({"name": "eu-west-1"}),
            Some((&cookie, &csrf)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let secret = created["token"].as_str().unwrap().to_owned();
    let fp = e.ca.as_ref().unwrap().fingerprint_sha256();
    assert_eq!(
        created["enroll_command"],
        format!(
            "builtpanel-agent enroll --server https://panel.example.com:8443 --ca-fingerprint sha256:{fp} --token {secret}"
        )
    );

    // The agent enrols with it.
    let (_, csr) = greedy_csr();
    let (status, enrolled) = e.enrol(&secret, &csr).await;
    assert_eq!(status, StatusCode::CREATED, "{enrolled}");
    assert_eq!(enrolled["name"], "eu-west-1");
    // The list shows the token as used, and the name can no longer be handed out.
    let req = Request::builder()
        .uri("/api/v1/admin/nodes/enrollments")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let res = e.app.clone().oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let list: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list["items"][0]["status"], "consumed");
    let (status, body, _, _) = e
        .post(
            "/api/v1/admin/nodes",
            json!({"name": "eu-west-1"}),
            Some((&cookie, &csrf)),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "node_name_taken")
    );
}
