//! The license module over HTTP, with a real license authority behind it and real PostgreSQL
//! (T17.10, T17.12, T17.13). Ignored by default; run with DATABASE_URL, REDIS_URL and
//! `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::get,
};
use builtpanel_auth_core::password::{Argon2Params, PasswordHasher};
use builtpanel_cache::RedisHandle;
use builtpanel_config::LicenseConfig;
use builtpanel_database::modules as db;
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_license::{FileSigningKey, SigningKey as _};
use builtpanel_license_authority::{
    LaState, Settings, config::LaConfig, migrate, register_key, router as la_router,
};
use builtpanel_mod_license::{
    LicenseState, Restrictor, admin_router, restrict, service::LicenseService,
};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

struct Fake {
    manifest: Manifest,
    routes: Box<dyn Fn() -> ModuleRoutes + Send + Sync>,
}

impl Module for Fake {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn routes(&self) -> ModuleRoutes {
        (self.routes)()
    }
}

struct Env {
    app: Router,
    sessions: SessionStore,
}

struct Browser {
    cookie: String,
    csrf: String,
}

/// A license authority of its own (its own database on the same server), serving on a port.
struct Authority {
    url: String,
    la_db: PgPool,
    admin_token: String,
    dir: std::path::PathBuf,
}

async fn authority() -> Authority {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let admin_conn = sqlx::PgPool::connect(&url).await.unwrap();
    let name = format!("la_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&admin_conn)
        .await
        .unwrap();
    let la_url = url
        .rsplit_once('/')
        .map(|(base, _)| format!("{base}/{name}"))
        .unwrap();
    let la_db = sqlx::PgPool::connect(&la_url).await.unwrap();
    migrate(&la_db).await.unwrap();
    let dir = std::env::temp_dir().join(format!("bp-lic-mod-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let key = FileSigningKey::generate_to_file(&dir.join("license-1.key"), "license-1").unwrap();
    register_key(&la_db, &key).await.unwrap();
    std::fs::write(
        dir.join("keys.txt"),
        format!("license-1 {}\n", base64_of(&key.public_key())),
    )
    .unwrap();
    let config = LaConfig {
        signing_key_file: dir.join("license-1.key"),
        ..LaConfig::default()
    };
    let hasher = Arc::new(
        PasswordHasher::new(Argon2Params {
            memory_kib: 19 * 1024,
            iterations: 2,
            parallelism: 1,
        })
        .unwrap(),
    );
    let state = LaState {
        db: la_db.clone(),
        signer: Arc::new(std::sync::RwLock::new(Arc::new(key))),
        settings: Settings::from_config(&config),
        prover: builtpanel_license_authority::domain_proof::default_prover(),
        hasher: hasher.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(
            listener,
            la_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let hash = hasher
        .hash(&SecretString::from("a-long-admin-password".to_owned()))
        .unwrap();
    sqlx::query("INSERT INTO admin_users (email, password_hash) VALUES ('root@example.com', $1)")
        .bind(hash)
        .execute(&la_db)
        .await
        .unwrap();
    let token = la_call(
        &addr,
        "POST",
        "/api/v1/admin/login",
        None,
        Some(json!({"email": "root@example.com", "password": "a-long-admin-password"})),
    )
    .await["token"]
        .as_str()
        .unwrap()
        .to_owned();
    Authority {
        url: addr,
        la_db,
        admin_token: token,
        dir,
    }
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

async fn la_call(
    base: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Value {
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
        let mut r = if method == "POST" {
            agent.post(&url)
        } else {
            let mut get = agent.get(&url);
            if let Some(t) = &token {
                get = get.header("authorization", format!("Bearer {t}"));
            }
            return get.call().unwrap().body_mut().read_json().unwrap();
        };
        if let Some(t) = &token {
            r = r.header("authorization", format!("Bearer {t}"));
        }
        match body {
            Some(b) => {
                // Some answers have no body (201 for a key): that is no JSON, and not an error.
                let text = r
                    .send_json(b)
                    .unwrap()
                    .body_mut()
                    .read_to_string()
                    .unwrap_or_default();
                serde_json::from_str(&text).unwrap_or(Value::Null)
            }
            None => r
                .send_empty()
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap_or(Value::Null),
        }
    })
    .await
    .unwrap()
}

async fn make_license(a: &Authority, max: i32) -> (String, String) {
    let c = la_call(
        &a.url,
        "POST",
        "/api/v1/admin/customers",
        Some(&a.admin_token),
        Some(json!({"name": "Acme", "email": format!("{}@acme.example", Uuid::new_v4().simple())})),
    )
    .await;
    let p = la_call(&a.url, "POST", "/api/v1/admin/products", Some(&a.admin_token), Some(json!({"slug": format!("p{}", &Uuid::new_v4().simple().to_string()[..8]), "name": "Built Panel"}))).await;
    let plan = la_call(&a.url, "POST", "/api/v1/admin/plans", Some(&a.admin_token), Some(json!({"product_id": p["id"], "name": "Pro", "features": ["mail", "resellers"], "limits": {"max_nodes": 2, "max_tenants": -1}, "max_activations": max, "valid_days": 30}))).await;
    let l = la_call(
        &a.url,
        "POST",
        "/api/v1/admin/licenses",
        Some(&a.admin_token),
        Some(json!({"customer_id": c["id"], "plan_id": plan["id"],
                    "domains": ["panel.example"], "hostnames": [this_host()], "ips": ["127.0.0.1"]})),
    )
    .await;
    (
        l["license"]["id"].as_str().unwrap().to_owned(),
        l["key"].as_str().unwrap().to_owned(),
    )
}

/// This machine's host name, which the licenses are bound to.
fn this_host() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_owned())
        .unwrap_or_default()
}

fn config_for(a: Option<&Authority>) -> LicenseConfig {
    let dir = std::env::temp_dir().join(format!("bp-lic-ident-{}", Uuid::new_v4().simple()));
    LicenseConfig {
        authority_url: a.map(|a| a.url.clone()),
        identity_dir: dir,
        public_keys_file: a.map(|a| a.dir.join("keys.txt")),
        domain: Some("panel.example".to_owned()),
        ..LicenseConfig::default()
    }
}

async fn env(pool: &PgPool, service: Arc<LicenseService>) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let manifest = Manifest::parse(
        "license",
        "[module]\nname = \"license\"\nversion = \"1.0.0\"\ntitle = { en = \"License\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions) = (pool.clone(), sessions.clone());
            move || ModuleRoutes {
                admin: Some(admin_router().with_state(LicenseState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    service: service.clone(),
                })),
                public: Some(
                    builtpanel_mod_license::public_router().with_state(LicenseState {
                        db: pool.clone(),
                        sessions: sessions.clone(),
                        service: service.clone(),
                    }),
                ),
                // Stand-ins for the rest of the panel, so that the restricted mode has something to close.
                client: Some(
                    Router::new()
                        .route(
                            "/api/v1/client/domains",
                            get(|| async { "list" }).post(|| async { "made" }),
                        )
                        .route("/api/v1/client/site", get(|| async { "site" })),
                ),
            }
        }),
    });
    let registry = Registry::build(vec![module]).unwrap();
    let values: Vec<Value> = registry
        .modules()
        .iter()
        .map(|m| serde_json::to_value(m.manifest()).unwrap())
        .collect();
    let records: Vec<db::ModuleRecord<'_>> = registry
        .modules()
        .iter()
        .zip(&values)
        .map(|(m, json)| db::ModuleRecord {
            name: &m.manifest().module.name,
            version: &m.manifest().module.version,
            category: m.manifest().module.category.as_str(),
            core: m.manifest().module.core,
            manifest: json,
        })
        .collect();
    db::sync(pool, &records).await.unwrap();
    let mount = ModuleMount::new(registry, Router::new());
    mount.rebuild();
    mount.apply(&db::enabled_names(pool).await.unwrap().into_iter().collect());
    // The restricted-mode middleware, as the control plane puts it on the whole application.
    let app = mount.service().layer(axum::middleware::from_fn_with_state(
        Restrictor::new(pool.clone()),
        restrict,
    ));
    Env { app, sessions }
}

impl Env {
    async fn sign_in(&self, account_id: Uuid) -> Browser {
        let (t, data) = self
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
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, &who.cookie)
            .header(CSRF_HEADER, &who.csrf);
        let req = match body {
            Some(v) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                req.body(Body::from(v.to_string())).unwrap()
            }
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

async fn admin(pool: &PgPool) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(pool, id, "super_admin", None)
        .await
        .unwrap();
    id
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

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_panel_activates_a_license_shows_it_and_follows_the_authority(pool: PgPool) {
    let a = authority().await;
    let (license_id, key) = make_license(&a, 1).await;
    let service = Arc::new(LicenseService::new(pool.clone(), &config_for(Some(&a))).unwrap());
    let e = env(&pool, service.clone()).await;
    let root = e.sign_in(admin(&pool).await).await;
    // The authority's domain check: the panel signs a challenge for its own domain, with no
    // session, and only a proper challenge.
    let probe = |nonce: String| {
        let app = e.app.clone();
        async move {
            let res = app
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/v1/public/license-proof?nonce={nonce}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 16)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let (status, p) = probe("a".repeat(32)).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert_eq!(p["domain"], "panel.example");
    assert_eq!(
        p["installation_id"],
        service.installation_id().unwrap().to_string()
    );
    assert!(p["signature"].as_str().unwrap().len() > 60);
    assert_eq!(
        probe("short".into()).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        probe(format!("{}!", "a".repeat(32))).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Before a key is entered: licensed nothing, and nothing is closed.
    service.evaluate_and_save().await.unwrap();
    let (status, v) = e.call(&root, "GET", "/api/v1/admin/license", None).await;
    assert_eq!(
        (status, v["enforced"].as_bool(), v["mode"].as_str()),
        (StatusCode::OK, Some(true), Some("unlicensed")),
        "{v}"
    );
    // A key that is not one, and one that is not the authority's.
    assert_eq!(
        e.call(
            &root,
            "POST",
            "/api/v1/admin/license/activate",
            Some(json!({"license_key": "x"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, got) = e
        .call(
            &root,
            "POST",
            "/api/v1/admin/license/activate",
            Some(json!({"license_key": "DHP-AAAAA-BBBBB-CCCCC-DDDDD"})),
        )
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("license_refused")),
        "{got}"
    );
    // The real one.
    let (status, v) = e
        .call(
            &root,
            "POST",
            "/api/v1/admin/license/activate",
            Some(json!({"license_key": key})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        (
            v["mode"].as_str(),
            v["plan"].as_str(),
            v["license_status"].as_str()
        ),
        (Some("active"), Some("Pro"), Some("active"))
    );
    assert_eq!(v["features"], json!(["mail", "resellers"]));
    assert_eq!(
        (
            v["limits"]["max_nodes"].as_i64(),
            v["limits"]["max_tenants"].as_i64()
        ),
        (Some(2), Some(-1))
    );
    assert!(
        v["installation_id"].is_string() && v["last_confirmed_at"].as_i64().unwrap() >= now() - 60
    );
    assert!(v["license_expires_at"].as_i64().unwrap() > now() + 20 * 86_400);
    // Refresh and heartbeat reach the authority.
    assert_eq!(
        e.call(&root, "POST", "/api/v1/admin/license/refresh", None)
            .await
            .0,
        StatusCode::OK
    );
    service.heartbeat().await.unwrap();
    let beats: i64 =
        sqlx::query_scalar("SELECT count(*) FROM activations WHERE last_heartbeat_at IS NOT NULL")
            .fetch_one(&a.la_db)
            .await
            .unwrap();
    assert_eq!(beats, 1);
    // Revoked at the authority: the next refresh closes the panel's management at once.
    la_call(
        &a.url,
        "POST",
        &format!("/api/v1/admin/licenses/{license_id}/revoke"),
        Some(&a.admin_token),
        Some(json!({"note": "test"})),
    )
    .await;
    let (status, got) = e
        .call(&root, "POST", "/api/v1/admin/license/refresh", None)
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("license_refused")),
        "{got}"
    );
    let (_, v) = e.call(&root, "GET", "/api/v1/admin/license", None).await;
    assert_eq!(
        (v["mode"].as_str(), v["reason"].as_str()),
        (Some("restricted"), Some("license_revoked")),
        "{v}"
    );
    // A recovery request is sent (and recorded there).
    let (status, _) = e
        .call(
            &root,
            "POST",
            "/api/v1/admin/license/recovery-request",
            Some(json!({"note": "please help"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let asked: i64 = sqlx::query_scalar("SELECT count(*) FROM recovery_requests")
        .fetch_one(&a.la_db)
        .await
        .unwrap();
    assert_eq!(asked, 1);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn offline_it_works_for_72_hours_then_changes_are_closed_with_423_and_sites_and_reading_stay_open(
    pool: PgPool,
) {
    let a = authority().await;
    let (_, key) = make_license(&a, 1).await;
    let service = Arc::new(LicenseService::new(pool.clone(), &config_for(Some(&a))).unwrap());
    let e = env(&pool, service.clone()).await;
    let root = e.sign_in(admin(&pool).await).await;
    service.activate(key).await.unwrap();
    // Active: changes go through.
    assert_eq!(
        e.call(&root, "POST", "/api/v1/client/domains", None)
            .await
            .0,
        StatusCode::OK
    );
    // Offline for a day: it still works (with a warning); the middleware lets changes through.
    let confirmed_ago = |hours: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query("UPDATE license_state SET stored = jsonb_set(stored, '{last_refresh_ok_at}', to_jsonb($1::bigint))")
                .bind(now() - hours * 3600)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    confirmed_ago(25).await;
    let verdict = service.evaluate_and_save().await.unwrap().unwrap();
    assert_eq!(format!("{:?}", verdict.mode), "Grace");
    let (_, v) = e.call(&root, "GET", "/api/v1/admin/license", None).await;
    assert_eq!(v["mode"], "grace");
    assert!(v["grace_ends_at"].as_i64().unwrap() > now());
    // The middleware remembers the verdict for a few seconds; the test waits it out.
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert_eq!(
        e.call(&root, "POST", "/api/v1/client/domains", None)
            .await
            .0,
        StatusCode::OK
    );
    // Past the 72nd hour: management is restricted.
    confirmed_ago(73).await;
    service.evaluate_and_save().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let (_, v) = e.call(&root, "GET", "/api/v1/admin/license", None).await;
    assert_eq!(
        (v["mode"].as_str(), v["reason"].as_str()),
        (Some("restricted"), Some("offline_grace_exceeded"))
    );
    // A new domain is refused with 423; reading (the "site" stand-in, lists, the license page) is not.
    let (status, got) = e.call(&root, "POST", "/api/v1/client/domains", None).await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::LOCKED, Some("license_restricted")),
        "{got}"
    );
    assert_eq!(
        e.call(&root, "GET", "/api/v1/client/site", None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        e.call(&root, "GET", "/api/v1/client/domains", None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        e.call(&root, "GET", "/api/v1/admin/license", None).await.0,
        StatusCode::OK
    );
    // The way out stays open: the license page can still ask the authority, and being confirmed
    // again lifts the restriction.
    assert_eq!(
        e.call(&root, "POST", "/api/v1/admin/license/refresh", None)
            .await
            .0,
        StatusCode::OK
    );
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert_eq!(
        e.call(&root, "POST", "/api/v1/client/domains", None)
            .await
            .0,
        StatusCode::OK
    );
    // Without an authority in the configuration nothing is ever restricted.
    let off = Arc::new(LicenseService::new(pool.clone(), &config_for(None)).unwrap());
    assert!(!off.enabled());
    assert!(off.evaluate_and_save().await.unwrap().is_none());
    let (status, got) = {
        let e2 = env_with(&pool, off).await;
        e2.call(
            &e2.sign_in(admin_again(&pool).await).await,
            "POST",
            "/api/v1/admin/license/refresh",
            None,
        )
        .await
    };
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("licensing_off"))
    );
}

async fn env_with(pool: &PgPool, service: Arc<LicenseService>) -> Env {
    // The module sync of a second environment on the same database is harmless.
    env(pool, service).await
}

async fn admin_again(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM accounts WHERE username = 'rootuser'")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The daily integrity scan from the panel's side (T17.16): the panel hashes the programs it
/// runs where they are installed and the authority compares them with the release; a changed
/// program fails the scan, a missing one is noticed, and nothing is sent that is not a hash.
#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_panel_scans_the_programs_it_runs_and_a_changed_one_fails(pool: PgPool) {
    use builtpanel_license::{ReleaseManifest, SignedManifest};
    let a = authority().await;
    let (_, key) = make_license(&a, 1).await;
    let programs = a.dir.join("installed");
    std::fs::create_dir_all(&programs).unwrap();
    let files = vec![
        ("builtpanel-control-api".to_owned(), b"control api".to_vec()),
        ("builtpanel-worker".to_owned(), b"worker".to_vec()),
    ];
    for (name, bytes) in &files {
        std::fs::write(programs.join(name), bytes).unwrap();
    }
    // The release this panel is: the pipeline's key signs the programs' hashes, the authority
    // is told to trust that key and publishes them.
    let release_key =
        FileSigningKey::generate_to_file(&a.dir.join("release-1.key"), "release-1").unwrap();
    let signed = SignedManifest::sign(
        &ReleaseManifest::build(env!("CARGO_PKG_VERSION"), "abcdef0", 1_700_000_000, &files)
            .unwrap(),
        &release_key,
    )
    .unwrap();
    la_call(
        &a.url,
        "POST",
        "/api/v1/admin/release-keys",
        Some(&a.admin_token),
        Some(json!({"kid": "release-1", "public_key": base64_of(&release_key.public_key())})),
    )
    .await;
    la_call(
        &a.url,
        "POST",
        "/api/v1/admin/releases",
        Some(&a.admin_token),
        Some(json!({"manifest": signed})),
    )
    .await;

    let mut config = config_for(Some(&a));
    config.binaries_dir = programs.clone();
    let service = Arc::new(LicenseService::new(pool.clone(), &config).unwrap());
    service.activate(key).await.unwrap();

    let scan = service.integrity_scan().await.unwrap();
    assert_eq!(scan.result, "passed", "{scan:?}");
    // One program is changed on disk: the next scan fails and names it.
    std::fs::write(programs.join("builtpanel-worker"), b"worker, patched").unwrap();
    let scan = service.integrity_scan().await.unwrap();
    assert_eq!(
        (scan.result.as_str(), scan.mismatched),
        ("failed", vec!["builtpanel-worker".to_owned()])
    );
    // A program that is gone is noticed too.
    std::fs::remove_file(programs.join("builtpanel-control-api")).unwrap();
    let scan = service.integrity_scan().await.unwrap();
    assert_eq!(scan.missing, vec!["builtpanel-control-api".to_owned()]);
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM integrity_events WHERE kind IN ('mismatch', 'unverifiable')",
    )
    .fetch_one(&a.la_db)
    .await
    .unwrap();
    assert_eq!(events, 2);

    // The failed scan restricted management; asking for help, an approval and a clean scan
    // put it right again.
    let verdict = service.evaluate_and_save().await.unwrap().unwrap();
    assert_eq!(
        (format!("{:?}", verdict.mode), verdict.reason.as_deref()),
        ("Restricted".to_owned(), Some("integrity_failed"))
    );
    for (name, bytes) in &files {
        std::fs::write(programs.join(name), bytes).unwrap();
    }
    // Clean again, but nobody has looked: still restricted.
    let scan = service.integrity_scan().await.unwrap();
    assert_eq!(
        (scan.result.as_str(), scan.integrity.as_str()),
        ("passed", "failed")
    );
    service
        .recovery_request("restored the programs from the release".to_owned())
        .await
        .unwrap();
    let requests = la_call(
        &a.url,
        "GET",
        "/api/v1/admin/recovery-requests?state=requested",
        Some(&a.admin_token),
        None,
    )
    .await;
    let id = requests[0]["id"].as_str().unwrap().to_owned();
    la_call(
        &a.url,
        "POST",
        &format!("/api/v1/admin/recovery-requests/{id}/approve"),
        Some(&a.admin_token),
        Some(json!({})),
    )
    .await;
    let scan = service.integrity_scan().await.unwrap();
    assert_eq!(
        (scan.result.as_str(), scan.integrity.as_str()),
        ("passed", "ok")
    );
    let verdict = service.evaluate_and_save().await.unwrap().unwrap();
    assert_eq!(
        format!("{:?}", verdict.mode),
        "Active",
        "{:?}",
        verdict.reason
    );
}
