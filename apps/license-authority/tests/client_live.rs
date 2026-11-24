//! The license client against the real authority over a real socket (T17.07, T17.09).
//! Ignored by default; run with DATABASE_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use builtpanel_auth_core::password::{Argon2Params, PasswordHasher};
use builtpanel_license::{Expect, FileSigningKey, PublicKeys, SigningKey as _};
use builtpanel_license_authority::{
    LaState, Settings,
    config::LaConfig,
    domain_proof::{Answer, BoxFuture, DomainProver, default_prover},
    register_key, router,
};
use builtpanel_license_client::{
    ClientError, Identity, LicenseClient, Mode, Policy, Stored, evaluate,
};
use secrecy::SecretString;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

struct Authority {
    url: String,
    keys: PublicKeys,
    dir: std::path::PathBuf,
}

async fn serve(pool: &PgPool, register: bool) -> Authority {
    serve_with(pool, register, default_prover(), false).await
}

async fn serve_with(
    pool: &PgPool,
    register: bool,
    prover: Arc<dyn DomainProver>,
    require_domain_proof: bool,
) -> Authority {
    let dir = std::env::temp_dir().join(format!("bp-la-client-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let key = FileSigningKey::generate_to_file(
        &dir.join("license-1.key"),
        &format!("license-{}", &Uuid::new_v4().simple().to_string()[..6]),
    )
    .unwrap();
    if register {
        register_key(pool, &key).await.unwrap();
    }
    let keys = PublicKeys::new().with(key.kid(), key.public_key()).unwrap();
    let config = LaConfig {
        signing_key_file: dir.join("license-1.key"),
        require_domain_proof,
        ..LaConfig::default()
    };
    let state = LaState {
        db: pool.clone(),
        signer: Arc::new(std::sync::RwLock::new(Arc::new(key))),
        settings: Settings::from_config(&config),
        prover,
        hasher: Arc::new(
            PasswordHasher::new(Argon2Params {
                memory_kib: 19 * 1024,
                iterations: 2,
                parallelism: 1,
            })
            .unwrap(),
        ),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Authority { url, keys, dir }
}

async fn admin_token(pool: &PgPool, a: &Authority) -> String {
    let hasher = PasswordHasher::new(Argon2Params {
        memory_kib: 19 * 1024,
        iterations: 2,
        parallelism: 1,
    })
    .unwrap();
    let hash = hasher
        .hash(&SecretString::from("a-long-admin-password".to_owned()))
        .unwrap();
    sqlx::query("INSERT INTO admin_users (email, password_hash) VALUES ('root@example.com', $1)")
        .bind(hash)
        .execute(pool)
        .await
        .unwrap();
    let r: serde_json::Value = http(
        &a.url,
        "POST",
        "/api/v1/admin/login",
        None,
        Some(json!({"email": "root@example.com", "password": "a-long-admin-password"})),
    )
    .await;
    r["token"].as_str().unwrap().to_owned()
}

/// A plain HTTP call, for the admin side (the client under test is the installation).
async fn http(
    base: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> serde_json::Value {
    let (base, method, path, token) = (
        base.to_owned(),
        method.to_owned(),
        path.to_owned(),
        token.map(str::to_owned),
    );
    tokio::task::spawn_blocking(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let url = format!("{base}{path}");
        let mut response = match (method.as_str(), body) {
            ("POST", Some(b)) => {
                let mut r = agent.post(&url);
                if let Some(t) = &token {
                    r = r.header("authorization", format!("Bearer {t}"));
                }
                r.send_json(b).unwrap()
            }
            ("POST", None) => {
                let mut r = agent.post(&url);
                if let Some(t) = &token {
                    r = r.header("authorization", format!("Bearer {t}"));
                }
                r.send_empty().unwrap()
            }
            _ => agent.get(&url).call().unwrap(),
        };
        response.body_mut().read_json().unwrap()
    })
    .await
    .unwrap()
}

async fn make_license(a: &Authority, token: &str, max: i32) -> (String, String) {
    let c = http(
        &a.url,
        "POST",
        "/api/v1/admin/customers",
        Some(token),
        Some(json!({"name": "Acme", "email": "a@acme.example"})),
    )
    .await;
    let p = http(
        &a.url,
        "POST",
        "/api/v1/admin/products",
        Some(token),
        Some(json!({"slug": "builtpanel", "name": "Built Panel"})),
    )
    .await;
    let plan = http(&a.url, "POST", "/api/v1/admin/plans", Some(token), Some(json!({"product_id": p["id"], "name": "Pro", "features": ["mail"], "limits": {"max_nodes": 2}, "max_activations": max, "valid_days": 30}))).await;
    let l = http(
        &a.url,
        "POST",
        "/api/v1/admin/licenses",
        Some(token),
        Some(json!({"customer_id": c["id"], "plan_id": plan["id"], "domains": ["panel.example"], "hostnames": ["web1"], "ips": ["127.0.0.1"]})),
    )
    .await;
    (
        l["license"]["id"].as_str().unwrap().to_owned(),
        l["key"].as_str().unwrap().to_owned(),
    )
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

fn client(a: &Authority, keys: PublicKeys, identity_dir: &std::path::Path) -> LicenseClient {
    LicenseClient::new(
        &a.url,
        keys,
        Identity::load_or_create(identity_dir).unwrap(),
        "builtpanel-la",
        "builtpanel",
    )
    .unwrap()
    .with_host("web1", "panel.example")
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_installation_activates_refreshes_and_lets_go_and_a_forged_answer_is_refused(
    pool: PgPool,
) {
    let a = serve(&pool, true).await;
    let token = admin_token(&pool, &a).await;
    let (license_id, key) = make_license(&a, &token, 1).await;
    let identity_dir = a.dir.join("identity");
    let keys = a.keys.clone();
    let url = a.url.clone();
    let result = tokio::task::spawn_blocking({
        let (identity_dir, a_dir) = (identity_dir.clone(), a.dir.clone());
        move || {
            let c = LicenseClient::new(&url, keys.clone(), Identity::load_or_create(&identity_dir).unwrap(), "builtpanel-la", "builtpanel").unwrap().with_host("web1", "panel.example");
            // Activation: verified, and the stored form evaluates as an active license.
            let got = c.activate(&format!("  {key}\n")).unwrap();
            assert_eq!(got.claims.features, ["mail"]);
            assert_eq!(got.license.plan, "Pro");
            let stored = Stored { entitlement: Some(got.document.clone()), last_refresh_ok_at: Some(now()), ..Stored::default() };
            let expect = Expect { issuer: "builtpanel-la", audience: "builtpanel", installation_id: c.installation_id(), clock_skew: 300, host: builtpanel_license::HostFacts { hostname: "web1", domain: "panel.example" } };
            let verdict = evaluate(&stored, &keys, &expect, &Policy::default(), now());
            assert_eq!(verdict.mode, Mode::Active);
            assert!(verdict.has_feature("mail"));
            // Refresh and heartbeat.
            assert_ne!(c.refresh().unwrap().document, got.document);
            assert_eq!(c.heartbeat(&json!({"version": "0.1.0", "nodes": 1})).unwrap().license_status, "active");
            // The same machine after a reinstall from the backup (the identity folder is copied):
            // the same installation, so the one place is not used up twice.
            let copy = a_dir.join("identity-restored");
            std::fs::create_dir_all(&copy).unwrap();
            for f in ["installation.id", "installation.key"] {
                std::fs::copy(identity_dir.join(f), copy.join(f)).unwrap();
            }
            let restored = LicenseClient::new(&url, keys.clone(), Identity::load_or_create(&copy).unwrap(), "builtpanel-la", "builtpanel").unwrap().with_host("web1", "panel.example");
            assert_eq!(restored.installation_id(), c.installation_id());
            assert!(restored.activate(&key).is_ok());
            // A different installation cannot take the one place.
            let other_dir = a_dir.join("identity-other");
            let other = LicenseClient::new(&url, keys.clone(), Identity::load_or_create(&other_dir).unwrap(), "builtpanel-la", "builtpanel").unwrap().with_host("web1", "panel.example");
            assert!(matches!(other.activate(&key), Err(ClientError::Refused { ref code, .. }) if code == "activation_limit"));
            // A help request is accepted.
            c.recovery_request("the binaries were replaced by an update that failed").unwrap();
            (c.installation_id(), key)
        }
    })
    .await
    .unwrap();
    let (installation, key) = result;
    let requests: i64 =
        sqlx::query_scalar("SELECT count(*) FROM recovery_requests WHERE installation_id = $1")
            .bind(installation)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(requests, 1);
    // An authority whose key is not in the bundle (somebody pretending to be it) is not believed.
    let fake = serve(&pool, false).await;
    let (real_keys, fake_url, dir) = (a.keys.clone(), fake.url.clone(), a.dir.join("identity"));
    let key2 = key.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let c = LicenseClient::new(
            &fake_url,
            real_keys,
            Identity::load_or_create(&dir).unwrap(),
            "builtpanel-la",
            "builtpanel",
        )
        .unwrap()
        .with_host("web1", "panel.example");
        c.activate(&key2)
    })
    .await
    .unwrap();
    assert!(
        matches!(outcome, Err(ClientError::Invalid(_))),
        "{outcome:?}"
    );
    // Revoked: the next refresh is a refusal of the license, not of the network.
    http(
        &a.url,
        "POST",
        &format!("/api/v1/admin/licenses/{license_id}/revoke"),
        Some(&token),
        Some(json!({"note": "test"})),
    )
    .await;
    let (keys, url, dir) = (a.keys.clone(), a.url.clone(), a.dir.join("identity"));
    let refusal = tokio::task::spawn_blocking(move || client_refresh(&url, keys, &dir))
        .await
        .unwrap();
    assert!(
        matches!(&refusal, Err(e) if e.is_license_refusal()),
        "{refusal:?}"
    );
    // Nobody home: unreachable, which is not a refusal.
    let c = LicenseClient::new(
        "http://127.0.0.1:9",
        a.keys.clone(),
        Identity::load_or_create(&a.dir.join("identity")).unwrap(),
        "builtpanel-la",
        "builtpanel",
    )
    .unwrap()
    .with_host("web1", "panel.example");
    let down = tokio::task::spawn_blocking(move || c.refresh())
        .await
        .unwrap();
    assert!(
        matches!(&down, Err(ClientError::Unreachable(_))),
        "{down:?}"
    );
    assert!(!down.unwrap_err().is_license_refusal());
    // And an address that is not https is not used (loopback aside).
    assert!(
        LicenseClient::new(
            "http://license.example",
            a.keys.clone(),
            Identity::load_or_create(&a.dir.join("identity")).unwrap(),
            "x",
            "y"
        )
        .is_err()
    );
    let _ = client; // (kept for the other tests)
}

fn client_refresh(url: &str, keys: PublicKeys, dir: &std::path::Path) -> Result<(), ClientError> {
    LicenseClient::new(
        url,
        keys,
        Identity::load_or_create(dir).unwrap(),
        "builtpanel-la",
        "builtpanel",
    )
    .unwrap()
    .with_host("web1", "panel.example")
    .refresh()
    .map(|_| ())
}

/// What the authority's outside check finds at the panel's domain.
type AnswerFn = Box<dyn Fn(&str, &str) -> Result<Answer, String> + Send + Sync>;

struct Fake {
    answer: std::sync::Mutex<AnswerFn>,
    seen: std::sync::Mutex<Vec<(String, Vec<std::net::IpAddr>)>>,
}

impl DomainProver for Fake {
    fn fetch<'a>(
        &'a self,
        domain: &'a str,
        nonce: &'a str,
        allowed: &'a [std::net::IpAddr],
    ) -> BoxFuture<'a, Result<Answer, String>> {
        self.seen
            .lock()
            .unwrap()
            .push((domain.to_owned(), allowed.to_vec()));
        let r = (self.answer.lock().unwrap())(domain, nonce);
        Box::pin(async move { r })
    }
}

fn answer_from(dir: std::path::PathBuf, domain: &'static str) -> AnswerFn {
    Box::new(move |_, nonce| {
        let c = LicenseClient::new(
            "http://127.0.0.1:1",
            PublicKeys::new(),
            Identity::load_or_create(&dir).unwrap(),
            "builtpanel-la",
            "builtpanel",
        )
        .unwrap()
        .with_host("web1", domain);
        let p = c.domain_proof(nonce).map_err(|e| e.to_string())?;
        Ok(Answer {
            installation_id: p.installation_id,
            domain: p.domain,
            signature: p.signature,
        })
    })
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_new_installation_must_be_answered_for_at_its_domain_when_the_check_is_on(pool: PgPool) {
    // The panel answers for the right installation: activation goes through; the authority was
    // told to connect only to the license's bound address.
    let identity_dir = std::env::temp_dir().join(format!("bp-proof-{}", Uuid::new_v4().simple()));
    let honest = Arc::new(Fake {
        answer: std::sync::Mutex::new(answer_from(identity_dir.clone(), "panel.example")),
        seen: Default::default(),
    });
    let a = serve_with(&pool, true, honest.clone(), true).await;
    let token = admin_token(&pool, &a).await;
    let (_license, key) = make_license(&a, &token, 3).await;
    let (url, keys) = (a.url.clone(), a.keys.clone());
    let dir = identity_dir.clone();
    let k = key.clone();
    let ok = tokio::task::spawn_blocking(move || {
        client_for(&url, keys, &dir)
            .activate(&k)
            .map(|r| r.license.plan)
    })
    .await
    .unwrap();
    assert_eq!(ok.unwrap(), "Pro");
    {
        let seen = honest.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "panel.example");
        assert_eq!(
            seen[0].1,
            vec!["127.0.0.1".parse::<std::net::IpAddr>().unwrap()]
        );
    }
    // The same installation again (refresh of the activation): no second outside check.
    let (url, keys, dir2, k) = (
        a.url.clone(),
        a.keys.clone(),
        identity_dir.clone(),
        key.clone(),
    );
    tokio::task::spawn_blocking(move || client_for(&url, keys, &dir2).activate(&k).unwrap())
        .await
        .unwrap();
    assert_eq!(honest.seen.lock().unwrap().len(), 1);

    // A domain that does not answer, one that answers with another installation's key, and one
    // that answers for another domain: all refused, and nothing is registered.
    let stranger = std::env::temp_dir().join(format!("bp-proof-{}", Uuid::new_v4().simple()));
    let someone_else = std::env::temp_dir().join(format!("bp-proof-{}", Uuid::new_v4().simple()));
    let cases: Vec<(&str, AnswerFn)> = vec![
        ("down", Box::new(|_, _| Err("connection refused".into()))),
        (
            "other key",
            answer_from(someone_else.clone(), "panel.example"),
        ),
        (
            "other domain",
            answer_from(stranger.clone(), "elsewhere.example"),
        ),
    ];
    for (name, answer) in cases {
        *honest.answer.lock().unwrap() = answer;
        let (url, keys, dir, key) = (a.url.clone(), a.keys.clone(), stranger.clone(), key.clone());
        let refused = tokio::task::spawn_blocking(move || {
            client_for(&url, keys, &dir).activate(&key).map(|_| ())
        })
        .await
        .unwrap();
        assert!(
            matches!(refused, Err(ClientError::Refused { ref code, .. }) if code == "domain_proof_failed"),
            "{name}: {refused:?}"
        );
    }
    let installations: i64 = sqlx::query_scalar("SELECT count(*) FROM installations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(installations, 1, "only the proven installation exists");
}

fn client_for(url: &str, keys: PublicKeys, dir: &std::path::Path) -> LicenseClient {
    LicenseClient::new(
        url,
        keys,
        Identity::load_or_create(dir).unwrap(),
        "builtpanel-la",
        "builtpanel",
    )
    .unwrap()
    .with_host("web1", "panel.example")
}
