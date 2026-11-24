//! The real agent code enrolling against the real enrollment endpoint, over a
//! local TCP port, with real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{Router, http::StatusCode, routing::post};
use builtpanel_cache::{
    RedisHandle,
    lockout::{FailureLimiter, LockoutPolicy},
};
use builtpanel_database::node_enrollments::insert_token;
use builtpanel_mod_nodes::{ENROLL_MAX_FAILURES, NodesState, router, token};
use builtpanel_node_agent::{
    enroll::{EnrollError, Options, enroll},
    identity::{CA_FILE, CERT_FILE, IDENTITY_FILE, KEY_FILE, StateDir},
    verify::VerifyError,
};
use builtpanel_pki::CertificateAuthority;
use builtpanel_session::{SessionConfig, SessionStore};
use rcgen::KeyPair;
use secrecy::{ExposeSecret as _, SecretString};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;
use x509_parser::prelude::FromDer as _;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

struct Control {
    base: String,
    ca: Arc<CertificateAuthority>,
    pool: PgPool,
}

/// Starts the real nodes module on a free local port.
async fn control_plane(pool: &PgPool) -> Control {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let ca = Arc::new(
        CertificateAuthority::create(
            "Test CA",
            OffsetDateTime::now_utc(),
            time::Duration::days(3650),
        )
        .unwrap()
        .0,
    );
    let state = NodesState {
        db: pool.clone(),
        sessions: SessionStore::new(redis.clone(), pool.clone(), SessionConfig::default()),
        public_url: None,
        stream_url: None,
        token_ttl: Duration::from_secs(900),
        ca: Some(ca.clone()),
        limiter: FailureLimiter::new(
            redis,
            format!("test:{}:enroll", Uuid::now_v7().simple()),
            LockoutPolicy {
                threshold: ENROLL_MAX_FAILURES,
                window: Duration::from_secs(600),
                lockout: Duration::from_secs(900),
            },
        ),
        gateway: None,
        operations: builtpanel_operations::Operations::new(pool.clone()),
        audit: builtpanel_audit::AuditWriter::new(pool.clone()),
    };
    let app = router()
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            Arc::<[std::net::IpAddr]>::from(Vec::new()),
            builtpanel_http_kit::client_ip::client_ip_layer,
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Control {
        base: format!("http://127.0.0.1:{port}"),
        ca,
        pool: pool.clone(),
    }
}

impl Control {
    async fn token(&self, name: &str) -> SecretString {
        let (secret, hash) = token::generate().unwrap();
        let admin: Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (kind, username, email, password_hash)
             VALUES ('admin', $1, $1 || '@example.com', $2) RETURNING id",
        )
        .bind(format!("by-{name}"))
        .bind(HASH)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        insert_token(
            &mut self.pool.acquire().await.unwrap(),
            name,
            &hash,
            admin,
            900,
        )
        .await
        .unwrap();
        secret
    }

    fn options(&self, token: SecretString, state_dir: &Path) -> Options {
        Options {
            server: self.base.clone(),
            token,
            ca_fingerprint: format!("sha256:{}", self.ca.fingerprint_sha256()),
            state_dir: state_dir.to_owned(),
            hostname: Some("agent-box.example.com".to_owned()),
            replace: false,
            tls_ca_pem: None,
        }
    }
}

/// The agent's `enroll` blocks, so it runs off the async runtime like in `main`.
async fn run_enroll(
    options: Options,
) -> Result<builtpanel_node_agent::enroll::Enrolled, EnrollError> {
    tokio::task::spawn_blocking(move || enroll(&options))
        .await
        .unwrap()
}

fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "builtpanel-agent-live-{tag}-{}",
        Uuid::now_v7().simple()
    ))
}

async fn token_state(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "SELECT CASE WHEN consumed_at IS NOT NULL THEN 'consumed' WHEN revoked_at IS NOT NULL THEN 'revoked' ELSE 'pending' END
         FROM node_enrollment_tokens ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_agent_enrols_for_real_and_keeps_its_key_to_itself(pool: PgPool) {
    let cp = control_plane(&pool).await;
    let secret = cp.token("agent-1").await;
    let dir = temp_dir("ok");
    let enrolled =
        run_enroll(cp.options(SecretString::from(secret.expose_secret().to_owned()), &dir))
            .await
            .expect("enrolment succeeds");
    let id = &enrolled.identity;
    assert_eq!(id.name, "agent-1");
    assert_eq!(id.ca_fingerprint_sha256, cp.ca.fingerprint_sha256());

    // On disk: private key private, everything else readable, nothing secret in the identity.
    use std::os::unix::fs::PermissionsExt as _;
    let mode = |name: &str| {
        std::fs::metadata(dir.join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(
        (
            mode(KEY_FILE),
            mode(CERT_FILE),
            mode(CA_FILE),
            mode(IDENTITY_FILE)
        ),
        (0o600, 0o644, 0o644, 0o644)
    );
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let loaded = StateDir::new(&dir).load().unwrap();
    assert_eq!(&loaded.identity, id);
    let identity_text = std::fs::read_to_string(dir.join(IDENTITY_FILE)).unwrap();
    assert!(!identity_text.contains(secret.expose_secret()) && !identity_text.contains("PRIVATE"));

    // The stored key belongs to the stored certificate, and the control plane knows that certificate.
    let key = KeyPair::from_pem(&std::fs::read_to_string(dir.join(KEY_FILE)).unwrap()).unwrap();
    let cert_pem = std::fs::read_to_string(dir.join(CERT_FILE)).unwrap();
    let der = x509_der(&cert_pem);
    let (_, cert) = x509_parser::prelude::X509Certificate::from_der(&der).unwrap();
    assert_eq!(
        cert.public_key().subject_public_key.data.as_ref(),
        key.public_key_raw()
    );
    let (node_id, fingerprint, hostname, version): (Uuid, String, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT n.id, c.fingerprint_sha256, n.hostname, n.agent_version
         FROM nodes n JOIN node_certificates c ON c.node_id = n.id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(node_id, id.node_id);
    assert_eq!(
        fingerprint,
        builtpanel_node_agent::verify::fingerprint(&der)
    );
    assert_eq!(hostname.as_deref(), Some("agent-box.example.com"));
    assert_eq!(
        version.as_deref(),
        Some(builtpanel_node_agent::AGENT_VERSION)
    );
    // The private key exists nowhere on the control plane side.
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM nodes n WHERE row_to_json(n)::text LIKE '%PRIVATE%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

fn x509_der(pem: &str) -> Vec<u8> {
    x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .unwrap()
        .1
        .contents
}
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_used_token_is_refused_and_the_existing_identity_is_left_alone(pool: PgPool) {
    let cp = control_plane(&pool).await;
    let secret = cp.token("agent-1").await;
    let dir = temp_dir("twice");
    run_enroll(cp.options(SecretString::from(secret.expose_secret().to_owned()), &dir))
        .await
        .unwrap();
    let key_before = std::fs::read(dir.join(KEY_FILE)).unwrap();

    // Without --force the agent does not even call the network.
    let mut closed_port = cp.options(SecretString::from(secret.expose_secret().to_owned()), &dir);
    closed_port.server = "http://127.0.0.1:1".to_owned();
    assert!(matches!(
        run_enroll(closed_port).await,
        Err(EnrollError::State(_))
    ));

    // With --force the call is made, the token no longer works, and nothing is replaced.
    let mut forced = cp.options(SecretString::from(secret.expose_secret().to_owned()), &dir);
    forced.replace = true;
    let err = run_enroll(forced).await.unwrap_err();
    assert!(
        matches!(&err, EnrollError::Rejected { status: 401, code: Some(c), .. } if c == "invalid_enrollment_token"),
        "{err}"
    );
    assert_eq!(
        std::fs::read(dir.join(KEY_FILE)).unwrap(),
        key_before,
        "the old key survives a failed re-enrolment"
    );
    assert!(StateDir::new(&dir).load().is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn an_agent_that_expects_another_ca_refuses_and_stores_nothing(pool: PgPool) {
    let cp = control_plane(&pool).await;
    let secret = cp.token("agent-1").await;
    let dir = temp_dir("pin");
    let (other, _) = CertificateAuthority::create(
        "Other CA",
        OffsetDateTime::now_utc(),
        time::Duration::days(30),
    )
    .unwrap();
    let mut options = cp.options(secret, &dir);
    options.ca_fingerprint = other.fingerprint_sha256().to_owned();
    let err = run_enroll(options).await.unwrap_err();
    assert!(
        matches!(err, EnrollError::Verify(VerifyError::CaMismatch)),
        "{err}"
    );
    assert!(
        !dir.join(KEY_FILE).exists() && !dir.join(IDENTITY_FILE).exists(),
        "nothing was stored"
    );
    assert!(!StateDir::new(&dir).is_enrolled());
    let _ = std::fs::remove_dir_all(&dir);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn mistakes_the_agent_can_catch_itself_do_not_burn_the_token(pool: PgPool) {
    let cp = control_plane(&pool).await;
    let secret = cp.token("agent-1").await;
    let dir = temp_dir("early");

    // A malformed pin, a bad server address and an unwritable directory all fail before any request.
    let mut bad_pin = cp.options(SecretString::from(secret.expose_secret().to_owned()), &dir);
    bad_pin.ca_fingerprint = "not-a-fingerprint".into();
    assert!(matches!(
        run_enroll(bad_pin).await,
        Err(EnrollError::Verify(VerifyError::BadPin))
    ));
    let mut bad_server = cp.options(SecretString::from(secret.expose_secret().to_owned()), &dir);
    bad_server.server = "http://panel.example.com".into();
    assert!(matches!(
        run_enroll(bad_server).await,
        Err(EnrollError::InvalidServer(_))
    ));
    let blocker = temp_dir("blocker");
    std::fs::write(&blocker, "a file where a directory should be").unwrap();
    let unwritable = cp.options(
        SecretString::from(secret.expose_secret().to_owned()),
        &blocker.join("state"),
    );
    assert!(matches!(
        run_enroll(unwritable).await,
        Err(EnrollError::State(_))
    ));
    assert_eq!(
        token_state(&pool).await,
        "pending",
        "none of these used the token"
    );

    // And the token still works afterwards.
    assert!(run_enroll(cp.options(secret, &dir)).await.is_ok());
    assert_eq!(token_state(&pool).await, "consumed");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&blocker);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn refusals_reach_the_user_with_the_reason_and_the_retry_time(pool: PgPool) {
    let cp = control_plane(&pool).await;
    let dir = temp_dir("refused");
    // A name clash: the control plane says so, the token is kept, nothing is stored.
    let secret = cp.token("agent-1").await;
    sqlx::query("INSERT INTO nodes (name) VALUES ('agent-1')")
        .execute(&pool)
        .await
        .unwrap();
    let err = run_enroll(cp.options(SecretString::from(secret.expose_secret().to_owned()), &dir))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, EnrollError::Rejected { status: 409, code: Some(c), .. } if c == "node_name_taken"),
        "{err}"
    );
    assert!(!StateDir::new(&dir).is_enrolled());
    assert_eq!(token_state(&pool).await, "pending");

    // Repeated wrong tokens lock the address out, and the agent reports when to retry.
    let (junk, _) = token::generate().unwrap();
    for _ in 0..ENROLL_MAX_FAILURES {
        let err = run_enroll(cp.options(SecretString::from(junk.expose_secret().to_owned()), &dir))
            .await
            .unwrap_err();
        assert!(matches!(err, EnrollError::Rejected { status: 401, .. }));
    }
    let err = run_enroll(cp.options(SecretString::from(junk.expose_secret().to_owned()), &dir))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, EnrollError::Rejected { status: 429, retry_after: Some(s), .. } if (1..=900).contains(s)),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_redirect_is_never_followed_so_the_token_cannot_be_forwarded() {
    // "Attacker" server: counts every request that reaches it.
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let sink = Router::new().fallback(post(move || {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            StatusCode::OK
        }
    }));
    let sink_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sink_port = sink_listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(sink_listener, sink).await.unwrap() });
    // The "control plane" answers every POST with a redirect to the attacker.
    let target = format!("http://127.0.0.1:{sink_port}/steal");
    let redirector = Router::new().fallback(post(move || {
        let target = target.clone();
        async move { (StatusCode::TEMPORARY_REDIRECT, [("location", target)]) }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, redirector).await.unwrap() });

    let dir = temp_dir("redirect");
    let options = Options {
        server: format!("http://127.0.0.1:{port}"),
        token: SecretString::from("dhn_secret".to_owned()),
        ca_fingerprint: "ab".repeat(32),
        state_dir: dir.clone(),
        hostname: None,
        replace: false,
        tls_ca_pem: None,
    };
    let err = run_enroll(options).await.unwrap_err();
    assert!(
        matches!(err, EnrollError::Rejected { status: 307, .. }),
        "{err}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "the redirect target received nothing"
    );
    assert!(!StateDir::new(&dir).is_enrolled());
    let _ = std::fs::remove_dir_all(&dir);
}
