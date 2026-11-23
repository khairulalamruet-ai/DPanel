//! Databases over HTTP, against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_cache::RedisHandle;
use builtpanel_database::modules as db;
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_databases::{
    DatabasesState, SETTINGS_SCHEMA, admin_router, client_router, public_router,
};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use builtpanel_settings::{Scope, SettingsRegistry};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const KEY: [u8; 32] = [9u8; 32];

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
    settings: Arc<SettingsRegistry>,
    spool: std::path::PathBuf,
}

struct Browser {
    cookie: String,
    csrf: String,
}

async fn env(pool: &PgPool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let settings = Arc::new(
        SettingsRegistry::build(vec![(
            "databases".to_owned(),
            serde_json::from_str(SETTINGS_SCHEMA).unwrap(),
        )])
        .unwrap(),
    );
    let spool = std::env::temp_dir().join(format!("bp-spool-{}", Uuid::new_v4()));
    let manifest = Manifest::parse(
        "databases",
        "[module]\nname = \"databases\"\nversion = \"1.0.0\"\ntitle = { en = \"Databases\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions, settings, spool) = (
                pool.clone(),
                sessions.clone(),
                settings.clone(),
                spool.clone(),
            );
            move || {
                let state = DatabasesState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    settings: settings.clone(),
                    master_key: MasterKey::from_bytes(&KEY),
                    spool_dir: spool.clone(),
                    agent_url: Some("https://panel.example.com:8443".to_owned()),
                };
                ModuleRoutes {
                    admin: Some(admin_router().with_state(state.clone())),
                    client: Some(client_router().with_state(state.clone())),
                    public: Some(public_router().with_state(state)),
                }
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
    Env {
        app: mount.service(),
        sessions,
        settings,
        spool,
    }
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
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

impl Env {
    /// A request with a raw body (a chunk) or none; the whole answer.
    async fn raw(
        &self,
        who: Option<&Browser>,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(who) = who {
            req = req
                .header(header::COOKIE, &who.cookie)
                .header(CSRF_HEADER, &who.csrf);
        }
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let res = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 24)
            .await
            .unwrap();
        (status, headers, bytes.to_vec())
    }
}

async fn client(pool: &PgPool, name: &str, uid: i32) -> (Uuid, Uuid) {
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', $1, $1 || '@example.com', $2) RETURNING id",
    )
    .bind(name)
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap();
    let node: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name) VALUES ('node1') ON CONFLICT (name) DO UPDATE SET name = 'node1' RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, $4, $5, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node)
    .bind(plan)
    .bind(name)
    .bind(uid)
    .fetch_one(pool)
    .await
    .unwrap();
    (account, tenant)
}

/// Plays the node answering the newest operation of a kind about `name`.
async fn node_answers(pool: &PgPool, kind: &str, name: &str, ok: bool) {
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operations WHERE kind = $1 AND payload ->> 'name' = $2 AND status = 'pending'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(kind)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    if ok {
        assert!(
            builtpanel_database::operations::succeed(pool, id, &json!({}))
                .await
                .unwrap()
        );
    } else {
        builtpanel_database::operations::fail(
            pool,
            id,
            &json!({"code": "internal", "message": "no", "retryable": false}),
            None,
        )
        .await
        .unwrap();
    }
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn databases_users_and_grants_are_made_on_the_node_in_the_background(pool: PgPool) {
    let e = env(&pool).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    let plan: Uuid = sqlx::query_scalar("SELECT plan_id FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE plans SET limits = '{\"max_databases\": 2, \"max_db_users\": 2}' WHERE id = $1",
    )
    .bind(plan)
    .execute(&pool)
    .await
    .unwrap();

    // Reading is open; the prefix and the limit are told.
    let (status, list) = e.call(&a, "GET", "/api/v1/client/databases", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(
        (list["prefix"].clone(), list["limit"].clone()),
        (json!("alice_"), json!(2))
    );
    // Making needs the feature.
    sqlx::query(
        "UPDATE feature_sets SET all_features = false, features = ARRAY['domains']::feature_key[]
         WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (status, body) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "shop"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "feature_disabled")
    );
    sqlx::query(
        "UPDATE feature_sets SET features = ARRAY['domains', 'databases', 'db_remote']::feature_key[]
         WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // A database: pending, queued for the node, ready when the node says so.
    let (status, shop) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "shop"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{shop}");
    assert_eq!(
        (shop["name"].clone(), shop["status"].clone()),
        (json!("alice_shop"), json!("pending"))
    );
    let queued: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'create_database'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(queued["name"], "alice_shop");
    node_answers(&pool, "create_database", "alice_shop", true).await;
    let (_, list) = e.call(&a, "GET", "/api/v1/client/databases", None).await;
    assert_eq!(list["items"][0]["status"], "ready");
    // Typed with the prefix already, the same name is taken.
    let (status, body) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "alice_shop"})),
        )
        .await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "name_taken"));
    // Names that cannot be, are refused with the reason.
    for (name, why) in [
        ("sh op", "invalid_db_name"),
        ("shop;drop", "invalid_db_name"),
        ("mysql", "reserved_db_name"),
        ("", "invalid_db_name"),
        (&"x".repeat(80), "db_name_too_long"),
    ] {
        let (status, body) = e
            .call(
                &a,
                "POST",
                "/api/v1/client/databases",
                Some(json!({"name": name})),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{name}: {body}");
        assert_eq!(body["errors"][0]["code"], why);
    }
    // The plan's limit.
    let (status, _) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "blog"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    node_answers(&pool, "create_database", "alice_blog", true).await;
    let (status, body) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "third"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "limit_reached")
    );
    // Another account sees none of it, and may use the same short name.
    let (_, theirs) = e.call(&b, "GET", "/api/v1/client/databases", None).await;
    assert_eq!(theirs["items"], json!([]));
    let (status, mine_too) = e
        .call(
            &b,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "shop"})),
        )
        .await;
    assert_eq!(
        (status, mine_too["name"].clone()),
        (StatusCode::CREATED, json!("bobby_shop"))
    );

    // A user: weak passwords are refused; without one a strong one is made and shown once.
    for bad in ["short", "alllowercaseonly", "Has\nnewline12A"] {
        let (status, body) = e
            .call(
                &a,
                "POST",
                "/api/v1/client/db-users",
                Some(json!({"name": "web", "password": bad})),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {body}");
        assert_eq!(body["errors"][0]["field"], "password");
    }
    let (status, made) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/db-users",
            Some(json!({"name": "web"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let password = made["password"].as_str().unwrap().to_owned();
    assert!(builtpanel_domain::databases::check_password(&password).is_ok());
    assert_eq!(made["user"]["name"], "alice_web");
    // The ledger has no password; the sealed one opens to what was shown.
    let ledger: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'upsert_db_user'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ledger["name"], "alice_web");
    assert!(ledger.get("password").is_none(), "{ledger}");
    let all_ledger: String =
        sqlx::query_scalar("SELECT string_agg(payload::text, ' ') FROM operations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!all_ledger.contains(&password));
    let (user_id, sealed) = builtpanel_database::databases::sealed_password_of(&pool, "alice_web")
        .await
        .unwrap()
        .unwrap();
    let opened = MasterKey::from_bytes(&KEY)
        .open(
            &builtpanel_database::databases::password_context(user_id),
            &sealed,
        )
        .unwrap();
    assert_eq!(opened, password.as_bytes());
    // Sealed to this user only.
    assert!(
        MasterKey::from_bytes(&KEY)
            .open(
                &builtpanel_database::databases::password_context(Uuid::now_v7()),
                &sealed
            )
            .is_err()
    );
    let audit: String =
        sqlx::query_scalar("SELECT string_agg(metadata::text, ' ') FROM audit_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !audit.contains(&password),
        "the audit log never holds a password"
    );
    // Not ready yet: no grant, no password change.
    let uid = made["user"]["id"].as_str().unwrap().to_owned();
    let shop_id = shop["id"].as_str().unwrap().to_owned();
    let (status, body) = e
        .call(
            &a,
            "PUT",
            &format!("/api/v1/client/databases/{shop_id}/grants/{uid}"),
            Some(json!({"access": "all"})),
        )
        .await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "not_ready"));
    // The node has it: ready, and the sealed password is gone.
    node_answers(&pool, "upsert_db_user", "alice_web", true).await;
    assert!(
        builtpanel_database::databases::sealed_password_of(&pool, "alice_web")
            .await
            .unwrap()
            .is_none()
    );
    let (_, users) = e.call(&a, "GET", "/api/v1/client/db-users", None).await;
    assert_eq!(
        (users["items"][0]["status"].clone(), users["limit"].clone()),
        (json!("ready"), json!(2))
    );

    // Grants: an unknown level is refused; a good one is queued and shown on both sides.
    let grant = |db: &str, user: &str| format!("/api/v1/client/databases/{db}/grants/{user}");
    let (status, body) = e
        .call(
            &a,
            "PUT",
            &grant(&shop_id, &uid),
            Some(json!({"access": "root"})),
        )
        .await;
    assert_eq!(
        (status, body["errors"][0]["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("unknown_access"))
    );
    let (status, view) = e
        .call(
            &a,
            "PUT",
            &grant(&shop_id, &uid),
            Some(json!({"access": "read_write"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["users"][0]["access"], "read_write");
    let queued: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'grant_db'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        queued,
        json!({"type": "grant_db", "database": "alice_shop", "user": "alice_web", "access": "read_write"})
    );
    let (_, users) = e.call(&a, "GET", "/api/v1/client/db-users", None).await;
    assert_eq!(users["items"][0]["databases"][0]["database"], "alice_shop");
    // Another account's user or database is not found.
    let (_, bobs_db) = e.call(&b, "GET", "/api/v1/client/databases", None).await;
    let bob_db = bobs_db["items"][0]["id"].as_str().unwrap().to_owned();
    let (status, _) = e
        .call(
            &a,
            "PUT",
            &grant(&bob_db, &uid),
            Some(json!({"access": "all"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = e
        .call(
            &b,
            "PUT",
            &grant(&bob_db, &uid),
            Some(json!({"access": "all"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        e.call(
            &b,
            "DELETE",
            &format!("/api/v1/client/db-users/{uid}"),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    // A new password goes the same way.
    let (status, changed) = e
        .call(
            &a,
            "PUT",
            &format!("/api/v1/client/db-users/{uid}/password"),
            Some(json!({"password": "Another-Passw0rd"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert!(
        changed["password"].is_null(),
        "a password the person gave is not shown back"
    );
    assert!(
        builtpanel_database::databases::sealed_password_of(&pool, "alice_web")
            .await
            .unwrap()
            .is_some()
    );

    // Remote access: the feature, the shape of the addresses, then saved and queued.
    let remote = format!("/api/v1/client/db-users/{uid}/remote-hosts");
    for bad in ["0.0.0.0/0", "%", "203.0.113.7", "10.0.0.0/7"] {
        let (status, body) = e
            .call(&a, "PUT", &remote, Some(json!({"cidrs": [bad]})))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {body}");
        assert_eq!(body["errors"][0]["code"], "invalid_cidr");
    }
    let many: Vec<String> = (0..21).map(|i| format!("203.0.113.{i}/32")).collect();
    assert_eq!(
        e.call(&a, "PUT", &remote, Some(json!({"cidrs": many})))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, saved) = e
        .call(
            &a,
            "PUT",
            &remote,
            Some(json!({"cidrs": ["203.0.113.7/32", "198.51.100.0/24", "203.0.113.7/32"]})),
        )
        .await;
    assert_eq!(
        (status, saved["cidrs"].clone()),
        (StatusCode::OK, json!(["203.0.113.7/32", "198.51.100.0/24"]))
    );
    let (_, got) = e.call(&a, "GET", &remote, None).await;
    assert_eq!(got["cidrs"].as_array().unwrap().len(), 2);
    let queued: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'set_db_remote_hosts'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(queued["username"], "alice");
    assert_eq!(queued["name"], "alice_web");
    // The node is told which addresses its packet filter lets through: all of them.
    let access: Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'set_db_remote_access' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        access["cidrs"],
        json!(["198.51.100.0/24", "203.0.113.7/32"])
    );

    // Removing a grant, a user and a database.
    assert_eq!(
        e.call(&a, "DELETE", &grant(&shop_id, &uid), None).await.0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        e.call(&a, "DELETE", &grant(&shop_id, &uid), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(
            &a,
            "DELETE",
            &format!("/api/v1/client/db-users/{uid}"),
            None
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    node_answers(&pool, "drop_db_user", "alice_web", true).await;
    let (_, users) = e.call(&a, "GET", "/api/v1/client/db-users", None).await;
    assert_eq!(users["items"], json!([]));
    assert_eq!(
        e.call(
            &a,
            "DELETE",
            &format!("/api/v1/client/databases/{shop_id}"),
            None
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let (_, list) = e.call(&a, "GET", "/api/v1/client/databases", None).await;
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["status"] == "deleting")
    );
    node_answers(&pool, "drop_database", "alice_shop", true).await;
    let (_, list) = e.call(&a, "GET", "/api/v1/client/databases", None).await;
    assert_eq!(
        list["items"].as_array().unwrap().len(),
        1,
        "only alice_blog is left"
    );

    // A database the node could not make is shown as failed (and can be removed).
    let (status, _) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "again"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    node_answers(&pool, "create_database", "alice_again", false).await;
    let (_, list) = e.call(&a, "GET", "/api/v1/client/databases", None).await;
    let failed = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "alice_again")
        .unwrap();
    assert_eq!(failed["status"], "failed");

    // A suspended account changes nothing.
    sqlx::query("UPDATE tenants SET status = 'suspended' WHERE id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "late"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "tenant_not_running")
    );
    let _ = (&e.settings, Scope::Global);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_account_prefix_can_be_switched_off_and_names_stay_unique(pool: PgPool) {
    let e = env(&pool).await;
    let (alice, _) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    sqlx::query(
        "UPDATE feature_sets SET all_features = false, features = ARRAY['databases']::feature_key[]
         WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    builtpanel_settings::set(
        &e.settings,
        &mut tx,
        "databases",
        Scope::Global,
        json!({"require_prefix": false}).as_object().unwrap(),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let (_, list) = e.call(&a, "GET", "/api/v1/client/databases", None).await;
    assert_eq!(list["prefix"], "");
    let (status, made) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "shared_name"})),
        )
        .await;
    assert_eq!(
        (status, made["name"].clone()),
        (StatusCode::CREATED, json!("shared_name"))
    );
    // Unique across the panel; the server's own names are still closed.
    let (status, body) = e
        .call(
            &b,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "shared_name"})),
        )
        .await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "name_taken"));
    let (status, _) = e
        .call(
            &b,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "mysql"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn ready_database(pool: &PgPool, tenant: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO databases (tenant_id, name, status) VALUES ($1, $2, 'ready') RETURNING id",
    )
    .bind(tenant)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_dump_is_uploaded_in_chunks_checked_and_handed_to_the_node_once(pool: PgPool) {
    let e = env(&pool).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    let database = ready_database(&pool, tenant, "alice_shop").await;
    let starter = format!("/api/v1/client/databases/{database}/imports");
    let dump: Vec<u8> = (0..5000u32)
        .flat_map(|i| format!("INSERT INTO t VALUES ({i});\n").into_bytes())
        .collect();
    let sha = sha256_hex(&dump);

    // What cannot be started.
    for (body, why) in [
        (
            json!({"size_bytes": 10, "sha256": "abc"}),
            "invalid_checksum",
        ),
        (json!({"size_bytes": 0, "sha256": sha}), "empty_file"),
        (
            json!({"size_bytes": 2u64 << 30, "sha256": sha}),
            "file_too_big",
        ),
    ] {
        let (status, got) = e.call(&a, "POST", &starter, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{got}");
        assert!(got.to_string().contains(why), "{why}: {got}");
    }
    let (status, _) = e
        .call(
            &b,
            "POST",
            &starter,
            Some(json!({"size_bytes": 10, "sha256": sha})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "someone else's database");

    let (status, import) = e
        .call(
            &a,
            "POST",
            &starter,
            Some(json!({"size_bytes": dump.len(), "sha256": sha.to_uppercase()})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{import}");
    let id = import["id"].as_str().unwrap().to_owned();
    assert_eq!(
        (import["status"].clone(), import["next_chunk"].clone()),
        (json!("uploading"), json!(0))
    );
    let (status, again) = e
        .call(
            &a,
            "POST",
            &starter,
            Some(json!({"size_bytes": 10, "sha256": sha})),
        )
        .await;
    assert_eq!(
        (status, code(&again)),
        (StatusCode::CONFLICT, "import_active")
    );
    let one = format!("/api/v1/client/imports/{id}");
    let (status, _) = e.call(&b, "GET", &one, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = e
        .raw(
            Some(&b),
            "PUT",
            &format!("{one}/chunks/0"),
            &[],
            dump[..10].to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Chunks go in order.
    let chunk = |n: i32| format!("{one}/chunks/{n}");
    let (status, _, body) = e
        .raw(Some(&a), "PUT", &chunk(1), &[], dump[..10].to_vec())
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let (status, _, body) = e.raw(Some(&a), "PUT", &chunk(0), &[], Vec::new()).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let (status, _, body) = e
        .raw(Some(&a), "PUT", &chunk(0), &[], dump[..1000].to_vec())
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let now: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        (now["received_bytes"].clone(), now["next_chunk"].clone()),
        (json!(1000), json!(1))
    );
    let (status, _, _) = e
        .raw(Some(&a), "PUT", &chunk(0), &[], dump[..1000].to_vec())
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "a chunk is taken once");
    let (status, got) = e.call(&a, "POST", &format!("{one}/complete"), None).await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "incomplete"));
    let mut too_much = dump[1000..].to_vec();
    too_much.push(b'x');
    let (status, _, _) = e.raw(Some(&a), "PUT", &chunk(1), &[], too_much).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _, _) = e
        .raw(
            Some(&a),
            "PUT",
            &chunk(1),
            &[],
            vec![0u8; 8 * 1024 * 1024 + 1],
        )
        .await;
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "chunks are at most 8 MiB"
    );
    let (status, _, _) = e
        .raw(Some(&a), "PUT", &chunk(1), &[], dump[1000..].to_vec())
        .await;
    assert_eq!(status, StatusCode::OK);

    // Whole and right: queued for the node, with a token that is only kept sealed.
    let (status, done) = e.call(&a, "POST", &format!("{one}/complete"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{done}");
    assert_eq!(done["status"], "queued");
    let (op_id, payload): (Uuid, Value) =
        sqlx::query_as("SELECT id, payload FROM operations WHERE kind = 'import_sql'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(payload["database"], "alice_shop");
    assert_eq!(payload["username"], "alice");
    assert_eq!(
        payload["url"],
        format!("https://panel.example.com:8443/api/v1/public/imports/{id}/data")
    );
    assert!(
        payload.get("token").is_none_or(Value::is_null),
        "no token in the ledger: {payload}"
    );
    let (hash, sealed): (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT token_hash, sealed_token FROM db_imports WHERE id = $1")
            .bind(id.parse::<Uuid>().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    let token = MasterKey::from_bytes(&KEY)
        .open(
            &builtpanel_database::imports::token_context(id.parse().unwrap()),
            &sealed,
        )
        .unwrap();
    {
        use sha2::{Digest, Sha256};
        assert_eq!(Sha256::digest(&token).to_vec(), hash);
    }
    let token = String::from_utf8(token).unwrap();

    // The node's download: only with the token, and ranges work.
    let data = format!("/api/v1/public/imports/{id}/data");
    let (status, _, _) = e.raw(None, "GET", &data, &[], Vec::new()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = e
        .raw(
            None,
            "GET",
            &data,
            &[("authorization", "Bearer nope")],
            Vec::new(),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let bearer = format!("Bearer {token}");
    let (status, headers, body) = e
        .raw(
            None,
            "GET",
            &data,
            &[("authorization", &bearer), ("accept-encoding", "gzip")],
            Vec::new(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body, dump,
        "the file as it was uploaded (never compressed on the way)"
    );
    assert!(
        headers
            .get("content-encoding")
            .is_none_or(|v| v == "identity")
    );
    let (status, headers, body) = e
        .raw(
            None,
            "GET",
            &data,
            &[("authorization", &bearer), ("range", "bytes=4000-")],
            Vec::new(),
        )
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, dump[4000..]);
    assert!(headers.get("content-range").is_some());

    // The node starts: running, and it reports how far it has come.
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(op_id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, now) = e.call(&a, "GET", &one, None).await;
    assert_eq!(now["status"], "running", "{now}");
    builtpanel_database::imports::record_progress(
        &pool,
        id.parse().unwrap(),
        "importing",
        2500,
        120,
    )
    .await
    .unwrap();
    let (_, now) = e.call(&a, "GET", &one, None).await;
    assert_eq!(
        (
            now["stage"].clone(),
            now["bytes_done"].clone(),
            now["statements"].clone()
        ),
        (json!("importing"), json!(2500), json!(120))
    );
    let (status, got) = e.call(&a, "DELETE", &one, None).await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "import_running")
    );
    // A node that does not know the import (it restarted) loses it, after a minute.
    builtpanel_database::imports::lost(&pool, id.parse().unwrap())
        .await
        .unwrap();
    let (_, now) = e.call(&a, "GET", &one, None).await;
    assert_eq!(
        now["status"], "running",
        "not lost yet: it was heard of a moment ago"
    );
    assert!(
        builtpanel_database::operations::succeed(
            &pool,
            op_id,
            &json!({"type": "sql_imported", "statements": 5000})
        )
        .await
        .unwrap()
    );
    let (_, now) = e.call(&a, "GET", &one, None).await;
    assert_eq!(now["status"], "done", "{now}");
    let left: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT sealed_token FROM db_imports WHERE id = $1")
            .bind(id.parse::<Uuid>().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(left.is_none(), "the token goes when the import has ended");
    let (status, _, _) = e
        .raw(
            None,
            "GET",
            &data,
            &[("authorization", &bearer)],
            Vec::new(),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "no download after the end"
    );
    assert!(
        e.spool
            .join(format!("{}.dump", id.replace('-', "")))
            .exists()
    );
    assert_eq!(
        builtpanel_mod_databases::imports::sweep(&pool, &e.spool)
            .await
            .unwrap(),
        1
    );
    assert!(
        !e.spool
            .join(format!("{}.dump", id.replace('-', "")))
            .exists(),
        "the spool file is removed"
    );
    let (status, got) = e.call(&a, "DELETE", &one, None).await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "already_ended")
    );

    // A file that is not what it was said to be ends the import at once.
    let (_, import) = e
        .call(
            &a,
            "POST",
            &starter,
            Some(json!({"size_bytes": 4, "sha256": sha})),
        )
        .await;
    let bad = import["id"].as_str().unwrap().to_owned();
    let (status, _, _) = e
        .raw(
            Some(&a),
            "PUT",
            &format!("/api/v1/client/imports/{bad}/chunks/0"),
            &[],
            b"oops".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("/api/v1/client/imports/{bad}/complete"),
            None,
        )
        .await;
    assert_eq!(
        (status, got.to_string().contains("checksum_mismatch")),
        (StatusCode::UNPROCESSABLE_ENTITY, true),
        "{got}"
    );
    let (_, now) = e
        .call(&a, "GET", &format!("/api/v1/client/imports/{bad}"), None)
        .await;
    assert_eq!(now["status"], "failed");

    // An upload can be given up; so can one that waits for the node.
    let (_, import) = e
        .call(
            &a,
            "POST",
            &starter,
            Some(json!({"size_bytes": 4, "sha256": sha256_hex(b"data")})),
        )
        .await;
    let giveup = import["id"].as_str().unwrap().to_owned();
    let (status, _, _) = e
        .raw(
            Some(&a),
            "PUT",
            &format!("/api/v1/client/imports/{giveup}/chunks/0"),
            &[],
            b"data".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = e
        .call(
            &a,
            "DELETE",
            &format!("/api/v1/client/imports/{giveup}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, now) = e
        .call(&a, "GET", &format!("/api/v1/client/imports/{giveup}"), None)
        .await;
    assert_eq!(now["status"], "cancelled");
    assert!(
        !e.spool
            .join(format!("{}.dump", giveup.replace('-', "")))
            .exists()
    );

    // The node's refusal is told to the client.
    let (_, import) = e
        .call(
            &a,
            "POST",
            &starter,
            Some(json!({"size_bytes": 4, "sha256": sha256_hex(b"data")})),
        )
        .await;
    let failing = import["id"].as_str().unwrap().to_owned();
    let (status, _, _) = e
        .raw(
            Some(&a),
            "PUT",
            &format!("/api/v1/client/imports/{failing}/chunks/0"),
            &[],
            b"data".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("/api/v1/client/imports/{failing}/complete"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let op_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operations WHERE kind = 'import_sql' AND status = 'pending'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(op_id)
        .execute(&pool)
        .await
        .unwrap();
    builtpanel_database::operations::fail(
        &pool,
        op_id,
        &json!({"code": "invalid_argument", "message": "statement 3: error 1064: syntax", "retryable": false}),
        None,
    )
    .await
    .unwrap();
    let (_, now) = e
        .call(
            &a,
            "GET",
            &format!("/api/v1/client/imports/{failing}"),
            None,
        )
        .await;
    assert_eq!(now["status"], "failed", "{now}");
    assert_eq!(now["error"], "statement 3: error 1064: syntax");
    let (_, list) = e.call(&a, "GET", "/api/v1/client/imports", None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 4);
    assert_eq!(list["max_size_mb"], 1024);
    let _ = std::fs::remove_dir_all(&e.spool);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn an_export_is_made_by_the_node_and_downloaded_with_a_five_minute_link(pool: PgPool) {
    let e = env(&pool).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    let database = ready_database(&pool, tenant, "alice_shop").await;
    let starter = format!("/api/v1/client/databases/{database}/exports");

    let (status, _) = e.call(&b, "POST", &starter, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "someone else's database");
    let (status, export) = e.call(&a, "POST", &starter, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{export}");
    assert_eq!(export["status"], "queued");
    let id = export["id"].as_str().unwrap().to_owned();
    let uuid: Uuid = id.parse().unwrap();
    let (status, again) = e.call(&a, "POST", &starter, None).await;
    assert_eq!(
        (status, code(&again)),
        (StatusCode::CONFLICT, "export_active")
    );
    let (status, _) = e
        .call(&b, "GET", &format!("/api/v1/client/exports/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The operation: the upload address, and no token in the ledger.
    let (op_id, payload): (Uuid, Value) =
        sqlx::query_as("SELECT id, payload FROM operations WHERE kind = 'export_sql'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(payload["database"], "alice_shop");
    assert_eq!(
        payload["url"],
        format!("https://panel.example.com:8443/api/v1/public/exports/{id}")
    );
    assert!(payload.get("token").is_none_or(Value::is_null), "{payload}");
    let sealed: Vec<u8> = sqlx::query_scalar("SELECT sealed_token FROM db_exports WHERE id = $1")
        .bind(uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
    let token = String::from_utf8(
        MasterKey::from_bytes(&KEY)
            .open(&builtpanel_database::exports::token_context(uuid), &sealed)
            .unwrap(),
    )
    .unwrap();
    let bearer = format!("Bearer {token}");
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(op_id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, now) = e
        .call(&a, "GET", &format!("/api/v1/client/exports/{id}"), None)
        .await;
    assert_eq!(now["status"], "running", "{now}");

    // The node uploads: only with its token, in order.
    let dump: Vec<u8> = (0..3000u32).flat_map(|i| i.to_le_bytes()).collect();
    let chunk = |n: i32| format!("/api/v1/public/exports/{id}/chunks/{n}");
    let (status, _, _) = e
        .raw(None, "PUT", &chunk(0), &[], dump[..100].to_vec())
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = e
        .raw(
            None,
            "PUT",
            &chunk(0),
            &[("authorization", "Bearer nope")],
            dump[..100].to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = e
        .raw(
            None,
            "PUT",
            &chunk(1),
            &[("authorization", &bearer)],
            dump[..100].to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, _) = e
        .raw(
            None,
            "PUT",
            &chunk(0),
            &[("authorization", &bearer)],
            dump[..5000].to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = e
        .raw(
            None,
            "PUT",
            &chunk(1),
            &[("authorization", &bearer)],
            dump[5000..].to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, now) = e
        .call(&a, "GET", &format!("/api/v1/client/exports/{id}"), None)
        .await;
    assert_eq!(now["received_bytes"], dump.len());
    // No link before it is ready.
    let link_path = format!("/api/v1/client/exports/{id}/link");
    let (status, got) = e.call(&a, "POST", &link_path, None).await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "not_ready"));
    // A wrong size or checksum ends it.
    let done = format!("/api/v1/public/exports/{id}/complete");
    let said = json!({"size_bytes": dump.len(), "sha256": sha256_hex(&dump)});
    let (status, _, _) = e
        .raw(
            None,
            "POST",
            &done,
            &[
                ("authorization", &bearer),
                ("content-type", "application/json"),
            ],
            json!({"size_bytes": dump.len(), "sha256": sha256_hex(b"other")})
                .to_string()
                .into_bytes(),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, now) = e
        .call(&a, "GET", &format!("/api/v1/client/exports/{id}"), None)
        .await;
    assert_eq!(
        now["status"], "failed",
        "a dump that does not match ends the export: {now}"
    );

    // Again, rightly this time.
    let (status, export) = e.call(&a, "POST", &starter, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{export}");
    let id = export["id"].as_str().unwrap().to_owned();
    let uuid: Uuid = id.parse().unwrap();
    let sealed: Vec<u8> = sqlx::query_scalar("SELECT sealed_token FROM db_exports WHERE id = $1")
        .bind(uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
    let token = String::from_utf8(
        MasterKey::from_bytes(&KEY)
            .open(&builtpanel_database::exports::token_context(uuid), &sealed)
            .unwrap(),
    )
    .unwrap();
    let bearer = format!("Bearer {token}");
    let (status, _, _) = e
        .raw(
            None,
            "PUT",
            &format!("/api/v1/public/exports/{id}/chunks/0"),
            &[("authorization", &bearer)],
            dump.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = e
        .raw(
            None,
            "POST",
            &format!("/api/v1/public/exports/{id}/complete"),
            &[
                ("authorization", &bearer),
                ("content-type", "application/json"),
            ],
            said.to_string().into_bytes(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, now) = e
        .call(&a, "GET", &format!("/api/v1/client/exports/{id}"), None)
        .await;
    assert_eq!(
        (now["status"].clone(), now["size_bytes"].clone()),
        (json!("ready"), json!(dump.len()))
    );
    assert!(now["expires_at"].is_string());
    let left: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT sealed_token FROM db_exports WHERE id = $1")
            .bind(uuid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(left.is_none(), "the node's token goes with the upload");
    let (status, _, _) = e
        .raw(
            None,
            "PUT",
            &format!("/api/v1/public/exports/{id}/chunks/1"),
            &[("authorization", &bearer)],
            b"late".to_vec(),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "no more uploads once it is ready"
    );

    // The link: someone else cannot ask, it works for the file, not for anything else.
    let link_path = format!("/api/v1/client/exports/{id}/link");
    let (status, _) = e.call(&b, "POST", &link_path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, link) = e.call(&a, "POST", &link_path, None).await;
    assert_eq!(status, StatusCode::OK, "{link}");
    let url = link["url"].as_str().unwrap().to_owned();
    let (status, headers, body) = e
        .raw(
            None,
            "GET",
            &url,
            &[("accept-encoding", "gzip")],
            Vec::new(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, dump);
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"alice_shop.sql.gz\""
    );
    let (status, _, body) = e
        .raw(None, "GET", &url, &[("range", "bytes=8000-")], Vec::new())
        .await;
    assert_eq!(
        (status, body),
        (StatusCode::PARTIAL_CONTENT, dump[8000..].to_vec())
    );
    let wrong = url.replace("token=", "token=x");
    let (status, _, _) = e.raw(None, "GET", &wrong, &[], Vec::new()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Five minutes on, it no longer works.
    sqlx::query(
        "UPDATE db_exports SET link_expires_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(uuid)
    .execute(&pool)
    .await
    .unwrap();
    let (status, _, _) = e.raw(None, "GET", &url, &[], Vec::new()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the link has run out");
    let (status, link) = e.call(&a, "POST", &link_path, None).await;
    assert_eq!(status, StatusCode::OK, "a new one can be asked for: {link}");
    let (status, _, _) = e
        .raw(None, "GET", link["url"].as_str().unwrap(), &[], Vec::new())
        .await;
    assert_eq!(status, StatusCode::OK);

    // Removed by its owner, or by time: the file goes.
    let file = e.spool.join(format!("{}.sql.gz", id.replace('-', "")));
    assert!(file.exists());
    let (status, _) = e
        .call(&a, "DELETE", &format!("/api/v1/client/exports/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!file.exists());
    let (status, got) = e.call(&a, "POST", &link_path, None).await;
    assert_eq!((status, code(&got)), (StatusCode::CONFLICT, "not_ready"));
    let (_, list) = e.call(&a, "GET", "/api/v1/client/exports", None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 2);

    // An export that is not picked up in time expires and its file is swept.
    let (_, export) = e.call(&a, "POST", &starter, None).await;
    let id2: Uuid = export["id"].as_str().unwrap().parse().unwrap();
    sqlx::query("UPDATE db_exports SET status = 'ready', expires_at = now() - interval '1 minute' WHERE id = $1")
        .bind(id2)
        .execute(&pool)
        .await
        .unwrap();
    let stale = e.spool.join(format!("{}.sql.gz", id2.simple()));
    std::fs::write(&stale, b"x").unwrap();
    builtpanel_mod_databases::imports::sweep(&pool, &e.spool)
        .await
        .unwrap();
    assert!(!stale.exists());
    let status: String = sqlx::query_scalar("SELECT status FROM db_exports WHERE id = $1")
        .bind(id2)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "expired");
    // The node's own failure is told.
    let (_, export) = e.call(&a, "POST", &starter, None).await;
    let failing = export["id"].as_str().unwrap().to_owned();
    let op: Uuid = sqlx::query_scalar(
        "SELECT id FROM operations WHERE kind = 'export_sql' AND idempotency_key = $1",
    )
    .bind(format!("export:{failing}"))
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
    builtpanel_database::operations::fail(
        &pool,
        op,
        &json!({"code": "internal", "message": "the dump failed (exit 2): no", "retryable": false}),
        None,
    )
    .await
    .unwrap();
    let (_, now) = e
        .call(
            &a,
            "GET",
            &format!("/api/v1/client/exports/{failing}"),
            None,
        )
        .await;
    assert_eq!(
        (now["status"].clone(), now["error"].clone()),
        (json!("failed"), json!("the dump failed (exit 2): no"))
    );
    let _ = std::fs::remove_dir_all(&e.spool);
}

/// Plays the node for `seconds`: every operation of the kinds is taken and answered.
fn node_plays(
    pool: PgPool,
    kinds: &'static [&'static str],
    ok: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        for _ in 0..200 {
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM operations WHERE kind = ANY($1) AND status = 'pending'",
            )
            .bind(kinds)
            .fetch_all(&pool)
            .await
            .unwrap();
            for id in ids {
                sqlx::query(
                    "UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1",
                )
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
                if ok {
                    builtpanel_database::operations::succeed(
                        &pool,
                        id,
                        &json!({"type": "phpmyadmin_opened", "link_expires_at": 4_102_444_800_i64, "applied": true}),
                    )
                    .await
                    .unwrap();
                } else {
                    builtpanel_database::operations::fail(
                        &pool,
                        id,
                        &json!({"code": "internal", "message": "no", "retryable": false}),
                        None,
                    )
                    .await
                    .unwrap();
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn phpmyadmin_links_are_made_once_per_sign_in_and_the_token_never_reaches_the_ledger(
    pool: PgPool,
) {
    let e = env(&pool).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    let database = ready_database(&pool, tenant, "alice_shop").await;
    let path = format!("/api/v1/client/databases/{database}/phpmyadmin");

    let (status, got) = e.call(&a, "POST", &path, None).await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "phpmyadmin_unavailable")
    );
    let mut tx = pool.begin().await.unwrap();
    builtpanel_settings::set(
        &e.settings,
        &mut tx,
        "databases",
        Scope::Global,
        json!({"phpmyadmin_host": "pma.example.com", "phpmyadmin_session_minutes": 45})
            .as_object()
            .unwrap(),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let (status, _) = e.call(&b, "POST", &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "someone else's database");

    let node = node_plays(
        pool.clone(),
        &["set_phpmyadmin_host", "open_phpmyadmin"],
        true,
    );
    let (status, link) = e.call(&a, "POST", &path, None).await;
    assert_eq!(status, StatusCode::OK, "{link}");
    let url = link["url"].as_str().unwrap();
    let token = url
        .strip_prefix("https://pma.example.com/index.php?dhsignon=")
        .expect(url);
    assert_eq!(token.len(), 64);
    assert_eq!(link["expires_at"], "2100-01-01T00:00:00Z");
    let payload: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'open_phpmyadmin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(payload["database"], "alice_shop");
    assert_eq!(payload["session_minutes"], 45);
    assert_eq!(payload["token_hash"], sha256_hex(token.as_bytes()));
    let everywhere: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE payload::text LIKE '%' || $1 || '%'",
    )
    .bind(token)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(everywhere, 0, "the token is never in the ledger");
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE metadata::text LIKE '%' || $1 || '%'",
    )
    .bind(token)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0, "nor in the audit log");
    // Another sign-in: a new token and a new session; the route is set once.
    let (status, again) = e.call(&a, "POST", &path, None).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_ne!(again["url"], link["url"]);
    let hosts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'set_phpmyadmin_host'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(hosts, 1);
    node.abort();

    // A node that cannot do it says so.
    let node = node_plays(pool.clone(), &["open_phpmyadmin"], false);
    let (status, got) = e.call(&a, "POST", &path, None).await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::SERVICE_UNAVAILABLE, "phpmyadmin_failed"),
        "{got}"
    );
    node.abort();
}

async fn admin_with(pool: &PgPool, name: &str, roles: &[&str]) -> Uuid {
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
        builtpanel_database::rbac::assign_role(pool, id, role, None)
            .await
            .unwrap();
    }
    id
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_administrator_sees_databases_per_node_and_the_biggest_ones(pool: PgPool) {
    let e = env(&pool).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let (_, other) = client(&pool, "bobby", 10_001).await;
    let a = e.sign_in(alice).await;
    let root = e
        .sign_in(admin_with(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let nobody = e.sign_in(admin_with(&pool, "plain", &[]).await).await;
    // A second node with a tenant of its own.
    let node2: Uuid = sqlx::query_scalar("INSERT INTO nodes (name) VALUES ('node2') RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tenants SET node_id = $1 WHERE id = $2")
        .bind(node2)
        .bind(other)
        .execute(&pool)
        .await
        .unwrap();
    for (t, name, size) in [
        (tenant, "alice_small", 1_000_i64),
        (tenant, "alice_big", 9_000_000),
        (other, "bobby_mid", 500_000),
    ] {
        sqlx::query(
            "INSERT INTO databases (tenant_id, name, status, size_bytes, size_checked_at)
             VALUES ($1, $2, 'ready', $3, now())",
        )
        .bind(t)
        .bind(name)
        .bind(size)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO databases (tenant_id, name, status) VALUES ($1, 'bobby_new', 'pending')",
    )
    .bind(other)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO db_users (tenant_id, name, status) VALUES ($1, 'alice_web', 'ready')")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();

    // Only with the permission.
    for who in [&a, &nobody] {
        let (status, _) = e
            .call(who, "GET", "/api/v1/admin/databases/overview", None)
            .await;
        assert!(status == StatusCode::FORBIDDEN, "{status}");
    }
    let (status, overview) = e
        .call(&root, "GET", "/api/v1/admin/databases/overview", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(
        (
            overview["databases"].clone(),
            overview["users"].clone(),
            overview["size_bytes"].clone()
        ),
        (json!(4), json!(1), json!(9_501_000))
    );
    let nodes = overview["nodes"].as_array().unwrap();
    let n1 = nodes.iter().find(|n| n["node"] == "node1").unwrap();
    assert_eq!(
        (
            n1["databases"].clone(),
            n1["users"].clone(),
            n1["size_bytes"].clone()
        ),
        (json!(2), json!(1), json!(9_001_000))
    );
    assert!(n1["measured_at"].is_string());
    let n2 = nodes.iter().find(|n| n["node"] == "node2").unwrap();
    assert_eq!(
        (n2["databases"].clone(), n2["size_bytes"].clone()),
        (json!(2), json!(500_000))
    );
    assert!(
        n2["measured_at"].is_null(),
        "one database was never measured"
    );

    let (_, big) = e
        .call(
            &root,
            "GET",
            "/api/v1/admin/databases/biggest?limit=2",
            None,
        )
        .await;
    let names: Vec<&str> = big["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alice_big", "bobby_mid"]);
    assert_eq!(big["items"][0]["account"], "alice");
    assert_eq!(big["items"][0]["node"], "node1");
    let (_, on2) = e
        .call(
            &root,
            "GET",
            &format!("/api/v1/admin/databases/biggest?node_id={node2}"),
            None,
        )
        .await;
    assert_eq!(on2["items"].as_array().unwrap().len(), 2);
    let (status, _) = e
        .call(
            &root,
            "GET",
            "/api/v1/admin/databases/biggest?limit=0",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_postgresql_database_needs_the_feature_and_only_meets_users_of_its_own_server(
    pool: PgPool,
) {
    let e = env(&pool).await;
    let (alice, _tenant) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice).await;
    sqlx::query(
        "UPDATE feature_sets SET all_features = false, features = ARRAY['domains', 'databases']::feature_key[]
         WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)",
    )
    .execute(&pool)
    .await
    .unwrap();
    // Without the feature in the plan: refused, and a made-up engine is refused too.
    let (status, body) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "pgshop", "engine": "postgres"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "feature_disabled")
    );
    let (status, body) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "x", "engine": "oracle"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["errors"][0]["code"], "unknown_engine");
    sqlx::query(
        "UPDATE feature_sets SET features = ARRAY['domains', 'databases', 'db_postgres']::feature_key[]
         WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // A PostgreSQL database and user; the node is told which server.
    let (status, pg) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "pgshop", "engine": "postgres"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{pg}");
    assert_eq!(pg["engine"], "postgres");
    let queued: Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'create_database' AND payload ->> 'name' = 'alice_pgshop'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(queued["engine"], "postgres");
    node_answers(&pool, "create_database", "alice_pgshop", true).await;
    // A MariaDB database says nothing of an engine on the wire (what every node knows).
    let (_, my) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "mysqlshop"})),
        )
        .await;
    assert_eq!(my["engine"], "mysql");
    let queued: Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'create_database' AND payload ->> 'name' = 'alice_mysqlshop'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(queued.get("engine").is_none(), "{queued}");
    node_answers(&pool, "create_database", "alice_mysqlshop", true).await;
    // A PostgreSQL name is at most 58 characters, with the prefix.
    let (status, body) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "y".repeat(54), "engine": "postgres"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["errors"][0]["code"], "db_name_too_long");

    let (status, made) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/db-users",
            Some(json!({"name": "pgweb", "engine": "postgres"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["user"]["engine"], "postgres");
    let ledger: Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'upsert_db_user' AND payload ->> 'name' = 'alice_pgweb'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger["engine"], "postgres");
    assert!(ledger.get("password").is_none(), "{ledger}");
    node_answers(&pool, "upsert_db_user", "alice_pgweb", true).await;
    let (_, mysql_user) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/db-users",
            Some(json!({"name": "myweb"})),
        )
        .await;
    node_answers(&pool, "upsert_db_user", "alice_myweb", true).await;
    let my_user = mysql_user["user"]["id"].as_str().unwrap().to_owned();
    let pg_user = made["user"]["id"].as_str().unwrap().to_owned();
    let (pg_id, my_id) = (
        pg["id"].as_str().unwrap().to_owned(),
        my["id"].as_str().unwrap().to_owned(),
    );

    // A grant joins a database and a user of the same server only.
    for (db, user) in [(&pg_id, &my_user), (&my_id, &pg_user)] {
        let (status, body) = e
            .call(
                &a,
                "PUT",
                &format!("/api/v1/client/databases/{db}/grants/{user}"),
                Some(json!({"access": "read_only"})),
            )
            .await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::CONFLICT, "engine_mismatch")
        );
    }
    let (status, body) = e
        .call(
            &a,
            "PUT",
            &format!("/api/v1/client/databases/{pg_id}/grants/{pg_user}"),
            Some(json!({"access": "read_write"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let grant: Value = sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'grant_db'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(grant["engine"], "postgres");

    // There is no web client for PostgreSQL yet, and it says so.
    let (status, body) = e
        .call(
            &a,
            "POST",
            &format!("/api/v1/client/databases/{pg_id}/phpmyadmin"),
            None,
        )
        .await;
    assert!(
        status == StatusCode::CONFLICT
            && matches!(
                code(&body),
                "web_client_unavailable" | "phpmyadmin_unavailable"
            ),
        "{status} {body}"
    );
    // Remote access for a PostgreSQL user is told to the PostgreSQL server alone.
    sqlx::query(
        "UPDATE feature_sets SET features = ARRAY['domains', 'databases', 'db_postgres', 'db_remote']::feature_key[]
         WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (status, body) = e
        .call(
            &a,
            "PUT",
            &format!("/api/v1/client/db-users/{pg_user}/remote-hosts"),
            Some(json!({"cidrs": ["203.0.113.7/32"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let access: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'set_db_remote_access'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(access["engine"], "postgres");
    assert_eq!(access["cidrs"], json!(["203.0.113.7/32"]));
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn databases_of_a_tenant_with_its_own_server_are_made_there_and_wait_while_they_move(
    pool: PgPool,
) {
    let e = env(&pool).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice).await;
    sqlx::query(
        "UPDATE feature_sets SET all_features = false, features = ARRAY['domains', 'databases', 'db_remote']::feature_key[]
         WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)",
    )
    .execute(&pool)
    .await
    .unwrap();
    // On the shared server nothing says whose server it is.
    let (status, _) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "one"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let shared: Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'create_database' AND payload ->> 'name' = 'alice_one'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(shared.get("dedicated").is_none(), "{shared}");
    node_answers(&pool, "create_database", "alice_one", true).await;

    // The tenant's own server: new databases and users are made there.
    builtpanel_database::tenant_db::set_state(&pool, tenant, "mysql", "dedicated", None)
        .await
        .unwrap();
    let (status, _) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "two"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let own: Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'create_database' AND payload ->> 'name' = 'alice_two'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(own["dedicated"], "alice");
    node_answers(&pool, "create_database", "alice_two", true).await;
    let (_, made) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/db-users",
            Some(json!({"name": "web"})),
        )
        .await;
    let user = made["user"]["id"].as_str().unwrap().to_owned();
    let ledger: Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'upsert_db_user' AND payload ->> 'name' = 'alice_web'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger["dedicated"], "alice");
    node_answers(&pool, "upsert_db_user", "alice_web", true).await;
    // phpMyAdmin and remote addresses are for the shared server.
    let two = e.call(&a, "GET", "/api/v1/client/databases", None).await.1["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "alice_two")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, body) = e
        .call(
            &a,
            "POST",
            &format!("/api/v1/client/databases/{two}/phpmyadmin"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = e
        .call(
            &a,
            "PUT",
            &format!("/api/v1/client/db-users/{user}/remote-hosts"),
            Some(json!({"cidrs": ["203.0.113.7/32"]})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "remote_unavailable")
    );

    // While the move runs, work waits.
    builtpanel_database::tenant_db::set_state(&pool, tenant, "mysql", "moving_to_shared", None)
        .await
        .unwrap();
    let (status, body) = e
        .call(
            &a,
            "POST",
            "/api/v1/client/databases",
            Some(json!({"name": "three"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "database_server_moving")
    );
    let (status, _) = e
        .call(
            &a,
            "DELETE",
            &format!("/api/v1/client/databases/{two}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

/// Plays a node that answers every `studio_op` with `answer`.
fn studio_node(pool: PgPool, answer: Value) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        for _ in 0..400 {
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM operations WHERE kind = 'studio_op' AND status = 'pending'",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            for id in ids {
                sqlx::query(
                    "UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1",
                )
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
                builtpanel_database::operations::succeed(
                    &pool,
                    id,
                    &json!({"type": "studio", "result": answer}),
                )
                .await
                .unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    })
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_studio_works_on_ones_own_ready_database_and_its_sql_stays_out_of_the_audit_log(
    pool: PgPool,
) {
    let e = env(&pool).await;
    let (alice, tenant) = client(&pool, "alice", 10_000).await;
    let (bobby, _) = client(&pool, "bobby", 10_001).await;
    let (a, b) = (e.sign_in(alice).await, e.sign_in(bobby).await);
    let database = ready_database(&pool, tenant, "alice_shop").await;
    let path = format!("/api/v1/client/databases/{database}/studio");
    let node = studio_node(
        pool.clone(),
        json!({"type": "tables", "tables": [{"name": "items", "kind": "table", "rows": 3, "size_bytes": 16384}]}),
    );

    // Someone else's database is not found; the body is not trusted for the database name.
    let (status, _) = e
        .call(&b, "POST", &path, Some(json!({"type": "tables"})))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, got) = e
        .call(&a, "POST", &path, Some(json!({"type": "tables"})))
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got["tables"][0]["name"], "items");
    let payload: Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'studio_op'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(payload["database"], "alice_shop");
    assert_eq!(payload["request"]["type"], "tables");

    // A request that is not one of ours is refused before it reaches the node.
    for bad in [
        json!({"type": "shell", "command": "id"}),
        json!({"type": "rows", "table": "items"}),
        json!({"nothing": true}),
    ] {
        let (status, _) = e.call(&a, "POST", &path, Some(bad)).await;
        assert!(
            status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
            "{status}"
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'studio_op'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "nothing refused was queued");

    // A statement is queued once only (it may have changed something) and audited by kind only.
    let secret = "SELECT 'hunter2-the-password'";
    let (status, _) = e
        .call(
            &a,
            "POST",
            &path,
            Some(json!({"type": "query", "sql": secret})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let attempts: i32 = sqlx::query_scalar(
        "SELECT max_attempts FROM operations WHERE payload->'request'->>'type' = 'query'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(attempts, 1);
    let audit: String = sqlx::query_scalar(
        "SELECT string_agg(metadata::text, ' ') FROM audit_logs WHERE action = 'database.studio'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        audit.contains("query") && !audit.contains("hunter2"),
        "{audit}"
    );
    // Reading is not audited (it would be noise).
    let reads: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'database.studio' AND metadata->>'request' = 'tables'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reads, 0);
    node.abort();

    // A PostgreSQL database has its studio too: the node is told which server.
    let pg: Uuid = sqlx::query_scalar(
        "INSERT INTO databases (tenant_id, name, status, engine) VALUES ($1, 'alice_pg', 'ready', 'postgres') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let node = studio_node(pool.clone(), json!({"type": "tables", "tables": []}));
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("/api/v1/client/databases/{pg}/studio"),
            Some(json!({"type": "tables"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    let engines: Vec<String> = sqlx::query_scalar(
        "SELECT COALESCE(payload->>'engine', 'mysql') FROM operations WHERE kind = 'studio_op' AND payload->>'database' = 'alice_pg'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(engines, ["postgres"]);
    node.abort();
    // A database that is not ready has none yet.
    let pending: Uuid = sqlx::query_scalar(
        "INSERT INTO databases (tenant_id, name, status) VALUES ($1, 'alice_new', 'pending') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (status, got) = e
        .call(
            &a,
            "POST",
            &format!("/api/v1/client/databases/{pending}/studio"),
            Some(json!({"type": "tables"})),
        )
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "database_not_ready")
    );
}
