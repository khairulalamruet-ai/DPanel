//! The apps module over HTTP against real PostgreSQL and Redis (T12.01). Ignored by default;
//! run with DATABASE_URL, REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_cache::RedisHandle;
use builtpanel_database::modules as db;
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_apps::{AppsState, admin_router, client_router};
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

async fn env(pool: &PgPool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let manifest = Manifest::parse(
        "apps",
        "[module]\nname = \"apps\"\nversion = \"1.0.0\"\ntitle = { en = \"Apps\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions) = (pool.clone(), sessions.clone());
            move || ModuleRoutes {
                admin: Some(admin_router().with_state(AppsState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    master_key: builtpanel_auth_core::secretbox::MasterKey::from_bytes(&[42; 32]),
                    wait: std::time::Duration::from_millis(300),
                    catalog: std::sync::Arc::new(builtpanel_catalog::Catalog::embedded().unwrap()),
                })),
                client: Some(client_router().with_state(AppsState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    master_key: builtpanel_auth_core::secretbox::MasterKey::from_bytes(&[42; 32]),
                    wait: std::time::Duration::from_millis(300),
                    catalog: std::sync::Arc::new(builtpanel_catalog::Catalog::embedded().unwrap()),
                })),
                public: Some(builtpanel_mod_apps::public_router().with_state(AppsState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                    master_key: builtpanel_auth_core::secretbox::MasterKey::from_bytes(&[42; 32]),
                    wait: std::time::Duration::from_millis(300),
                    catalog: std::sync::Arc::new(builtpanel_catalog::Catalog::embedded().unwrap()),
                })),
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
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
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

async fn domain(pool: &PgPool, tenant: Uuid, fqdn: &str) -> Uuid {
    // One primary domain per account; the others are add-ons.
    let kind = if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM domains WHERE tenant_id = $1)",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .unwrap()
    {
        "addon"
    } else {
        "primary"
    };
    sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(tenant)
    .bind(fqdn)
    .bind(kind)
    .fetch_one(pool)
    .await
    .unwrap()
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

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn what_runs_where_is_tracked_one_application_per_place(pool: PgPool) {
    use builtpanel_database::apps::{self, CreateError, NewApp};
    let e = env(&pool).await;
    let (alice_account, alice) = client(&pool, "alice", 10_000).await;
    let (_, bob) = client(&pool, "bob", 10_001).await;
    let a = e.sign_in(alice_account).await;
    let site = domain(&pool, alice, "alice.example").await;
    let other = domain(&pool, alice, "shop.example").await;
    let bobs = domain(&pool, bob, "bob.example").await;
    let settings = json!({});
    let new = |domain_id, path| NewApp {
        tenant_id: alice,
        domain_id,
        path,
        kind: "wordpress",
        name: "My blog",
        version: Some("6.8"),
        database_id: None,
        settings: &settings,
    };
    // A site and a blog under it; the same place twice is refused; a domain that is not the
    // account's is refused.
    let root = apps::create(&pool, &new(Some(site), "/")).await.unwrap();
    let blog = apps::create(&pool, &new(Some(site), "/blog"))
        .await
        .unwrap();
    apps::create(&pool, &new(Some(other), "/")).await.unwrap();
    assert!(matches!(
        apps::create(&pool, &new(Some(site), "/blog")).await,
        Err(CreateError::Taken)
    ));
    assert!(matches!(
        apps::create(&pool, &new(Some(bobs), "/")).await,
        Err(CreateError::NotOwned)
    ));
    // A failed install does not hold its place.
    apps::set_status(&pool, blog, "failed", None, Some("no space"))
        .await
        .unwrap();
    let again = apps::create(&pool, &new(Some(site), "/blog"))
        .await
        .unwrap();
    assert_ne!(again, blog);
    apps::set_status(&pool, again, "active", Some("6.8.1"), None)
        .await
        .unwrap();
    // The client sees their own applications, all or one domain's.
    let (status, list) = e.call(&a, "GET", "/api/v1/client/apps", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["items"].as_array().unwrap().len(), 4, "{list}");
    let (_, on_site) = e
        .call(
            &a,
            "GET",
            &format!("/api/v1/client/apps?domain_id={site}"),
            None,
        )
        .await;
    let paths: Vec<&str> = on_site["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["/", "/blog", "/blog"]);
    assert_eq!(on_site["items"][0]["fqdn"], "alice.example");
    let (status, one) = e
        .call(&a, "GET", &format!("/api/v1/client/apps/{again}"), None)
        .await;
    assert_eq!(
        (status, one["status"].as_str(), one["version"].as_str()),
        (StatusCode::OK, Some("active"), Some("6.8.1"))
    );
    let (status, failed) = e
        .call(&a, "GET", &format!("/api/v1/client/apps/{blog}"), None)
        .await;
    assert_eq!(
        (status, failed["error"].as_str()),
        (StatusCode::OK, Some("no space"))
    );
    // Nobody else's.
    let (bob_account, _) = (
        sqlx::query_scalar::<_, Uuid>("SELECT owner_account_id FROM tenants WHERE id = $1")
            .bind(bob)
            .fetch_one(&pool)
            .await
            .unwrap(),
        (),
    );
    let b = e.sign_in(bob_account).await;
    assert_eq!(
        e.call(&b, "GET", &format!("/api/v1/client/apps/{root}"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (_, theirs) = e.call(&b, "GET", "/api/v1/client/apps", None).await;
    assert_eq!(theirs["items"].as_array().unwrap().len(), 0);
    // Stopping to track an application: not somebody else's, not while it installs.
    assert_eq!(
        e.call(&b, "DELETE", &format!("/api/v1/client/apps/{root}"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, busy) = e
        .call(&a, "DELETE", &format!("/api/v1/client/apps/{root}"), None)
        .await;
    assert_eq!(
        (status, busy["code"].as_str()),
        (StatusCode::CONFLICT, Some("app_installing"))
    );
    assert_eq!(
        e.call(&a, "DELETE", &format!("/api/v1/client/apps/{blog}"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        e.call(&a, "GET", &format!("/api/v1/client/apps/{blog}"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // The application is gone from the list.
    assert_eq!(
        e.call(&a, "GET", "/api/v1/client/apps", None).await.1["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    // An administrator reads any account's applications; a client may not.
    let root_admin = e.sign_in(admin(&pool).await).await;
    let path = format!("/api/v1/admin/tenants/{alice}/apps");
    assert_eq!(
        e.call(&a, "GET", &path, None).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, all) = e.call(&root_admin, "GET", &path, None).await;
    assert_eq!(
        (status, all["items"].as_array().unwrap().len()),
        (StatusCode::OK, 3)
    );
    assert_eq!(
        e.call(
            &root_admin,
            "GET",
            &format!("/api/v1/admin/tenants/{}/apps", Uuid::new_v4()),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // Removing the domain leaves the application, now without one.
    sqlx::query("DELETE FROM domains WHERE id = $1")
        .bind(other)
        .execute(&pool)
        .await
        .unwrap();
    let (_, list) = e.call(&a, "GET", "/api/v1/client/apps", None).await;
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["domain_id"].is_null())
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn wordpress_is_asked_for_with_a_database_of_its_own_and_sealed_passwords(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, alice) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice_account).await;
    let site = domain(&pool, alice, "alice.example").await;
    let (_, bob) = client(&pool, "bob", 10_001).await;
    let bobs = domain(&pool, bob, "bob.example").await;
    let path = "/api/v1/client/apps/wordpress";
    let good = json!({"domain_id": site, "path": "/blog", "title": "Alice's blog",
        "admin_user": "boss", "admin_email": "boss@example.com",
        "admin_password": "a-long-enough-pass-1", "plugins": ["classic-editor"]});
    let (status, made) = e.call(&a, "POST", path, Some(good.clone())).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        (
            made["status"].as_str(),
            made["kind"].as_str(),
            made["path"].as_str()
        ),
        (Some("installing"), Some("wordpress"), Some("/blog"))
    );
    // A database and a user of its own, and the two operations that make them.
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM databases WHERE tenant_id = $1")
        .bind(alice)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(names, ["alice_wp1"]);
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM operations WHERE tenant_id = $1 ORDER BY kind")
            .bind(alice)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(kinds, ["create_database", "upsert_db_user"]);
    // Neither password is in the ledger, and the sealed ones are not the text.
    let ledger: String =
        sqlx::query_scalar("SELECT string_agg(payload::text, ' ') FROM operations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!ledger.contains("a-long-enough-pass-1"));
    let sealed: Vec<Vec<u8>> = sqlx::query_scalar("SELECT sealed FROM app_secrets ORDER BY name")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(sealed.len(), 2);
    assert!(
        sealed
            .iter()
            .all(|s| !String::from_utf8_lossy(s).contains("a-long-enough-pass-1"))
    );
    // The tracked place is taken; a second site gets the next database name.
    let (status, again) = e.call(&a, "POST", path, Some(good.clone())).await;
    assert_eq!(
        (status, again["code"].as_str()),
        (StatusCode::CONFLICT, Some("place_taken")),
        "{again}"
    );
    let mut second = good.clone();
    second["path"] = json!("/shop");
    let (status, _) = e.call(&a, "POST", path, Some(second)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM databases WHERE tenant_id = $1 ORDER BY name")
            .bind(alice)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(names, ["alice_wp1", "alice_wp2"]);
    // Not valid, and not somebody else's domain.
    for bad in [
        json!({"domain_id": site, "path": "/a b", "title": "t", "admin_user": "boss", "admin_email": "b@e.com", "admin_password": "a-long-enough-pass-1"}),
        json!({"domain_id": site, "path": "/../x", "title": "t", "admin_user": "boss", "admin_email": "b@e.com", "admin_password": "a-long-enough-pass-1"}),
        json!({"domain_id": site, "title": "", "admin_user": "boss", "admin_email": "b@e.com", "admin_password": "a-long-enough-pass-1"}),
        json!({"domain_id": site, "title": "t", "admin_user": "x", "admin_email": "b@e.com", "admin_password": "a-long-enough-pass-1"}),
        json!({"domain_id": site, "title": "t", "admin_user": "boss", "admin_email": "nope", "admin_password": "a-long-enough-pass-1"}),
        json!({"domain_id": site, "title": "t", "admin_user": "boss", "admin_email": "b@e.com", "admin_password": "short"}),
        json!({"domain_id": site, "title": "t", "admin_user": "boss", "admin_email": "b@e.com", "admin_password": "a-long-enough-pass-1", "plugins": ["Bad Slug"]}),
        json!({"domain_id": bobs, "title": "t", "admin_user": "boss", "admin_email": "b@e.com", "admin_password": "a-long-enough-pass-1"}),
    ] {
        let (status, body) = e.call(&a, "POST", path, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad} -> {body}");
    }
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn wordpress_actions_are_sent_to_the_node_and_answered(pool: PgPool) {
    use builtpanel_database::apps::{self, NewApp};
    let e = env(&pool).await;
    let (alice_account, alice) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice_account).await;
    let (bob_account, bob) = client(&pool, "bob", 10_001).await;
    let b = e.sign_in(bob_account).await;
    let site = domain(&pool, alice, "alice.example").await;
    let settings = json!({});
    let id = apps::create(
        &pool,
        &NewApp {
            tenant_id: alice,
            domain_id: Some(site),
            path: "/blog",
            kind: "wordpress",
            name: "Blog",
            version: None,
            database_id: None,
            settings: &settings,
        },
    )
    .await
    .unwrap();
    let base = format!("/api/v1/client/apps/{id}");
    // Not installed yet: refused. Then it is active.
    let (status, got) = e.call(&a, "GET", &format!("{base}/wordpress"), None).await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("app_not_ready")),
        "{got}"
    );
    apps::set_status(&pool, id, "active", Some("7.1.4"), None)
        .await
        .unwrap();
    // The node is slow: 202 with the operation to ask about.
    let (status, pending) = e.call(&a, "GET", &format!("{base}/wordpress"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pending}");
    let op = pending["operation_id"].as_str().unwrap().to_owned();
    let (kind, payload): (String, serde_json::Value) =
        sqlx::query_as("SELECT kind, payload FROM operations WHERE id = $1::uuid")
            .bind(&op)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kind, "run_wp_cli");
    assert_eq!(
        (
            payload["path"].as_str(),
            payload["command"]["type"].as_str()
        ),
        (Some("blog"), Some("overview"))
    );
    let (_, poll) = e
        .call(&a, "GET", &format!("{base}/operations/{op}"), None)
        .await;
    assert_eq!(poll["state"], "running");
    // The node answers; the next ask has the overview.
    let overview = json!({"type": "word_press_overview", "core": "6.4.3", "core_update": "7.1.4", "maintenance": false,
        "plugins": [{"name": "classic-editor", "status": "active", "version": "1.0", "update_available": true, "update_version": "1.7.0"}],
        "themes": []});
    sqlx::query("UPDATE operations SET status = 'dispatched', dispatched_at = now(), attempts = 1 WHERE id = $1::uuid")
        .bind(&op)
        .execute(&pool)
        .await
        .unwrap();
    let done = sqlx::query("UPDATE operations SET status = 'succeeded', output = $2, finished_at = now() WHERE id = $1::uuid")
        .bind(&op)
        .bind(&overview)
        .execute(&pool)
        .await;
    assert!(done.is_ok(), "{done:?}");
    let (status, poll) = e
        .call(&a, "GET", &format!("{base}/operations/{op}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{poll}");
    assert_eq!(poll["state"], "done", "{poll}");
    assert_eq!(poll["overview"]["plugins"][0]["update_version"], "1.7.0");
    // Nobody else's.
    assert_eq!(
        e.call(&b, "GET", &format!("{base}/wordpress"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(&b, "GET", &format!("{base}/operations/{op}"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // A plugin update names its plugin; nothing odd gets through.
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{base}/wordpress/plugins/update"),
            Some(json!({"slug": "classic-editor"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{base}/wordpress/plugins/update"),
            Some(json!({"slug": "x; rm -rf"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{base}/wordpress/core/update"),
            Some(json!({"version": "6.8 beta"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{base}/wordpress/maintenance"),
            Some(json!({"on": true})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    // The scan and the hardening are asked for the same way; an empty hardening is refused.
    let (status, _) = e
        .call(&a, "GET", &format!("{base}/wordpress/scan"), None)
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{base}/wordpress/harden"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{base}/wordpress/harden"),
            Some(json!({"debug_off": true, "xmlrpc_off": true})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let hardening: serde_json::Value = sqlx::query_scalar(
        "SELECT payload -> 'command' FROM operations WHERE payload #>> '{command,type}' = 'harden'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (
            hardening["debug_off"].as_bool(),
            hardening["fix_permissions"].as_bool()
        ),
        (Some(true), Some(false))
    );
    // A password: sealed, never in the ledger.
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{base}/wordpress/admin-password"),
            Some(json!({"user": "boss", "password": "short"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{base}/wordpress/admin-password"),
            Some(json!({"user": "boss", "password": "A-brand-new-Admin-Pass-9"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let ledger: String =
        sqlx::query_scalar("SELECT string_agg(payload::text, ' ') FROM operations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!ledger.contains("A-brand-new"), "{ledger}");
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM app_secrets WHERE name = 'admin_reset'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kept, 1);
    let _ = (bob, apps::list);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_catalog_is_listed_and_any_of_its_applications_can_be_asked_for(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, alice) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice_account).await;
    let site = domain(&pool, alice, "alice.example").await;
    let (status, catalog) = e.call(&a, "GET", "/api/v1/client/apps/catalog", None).await;
    assert_eq!(status, StatusCode::OK, "{catalog}");
    let items = catalog["items"].as_array().unwrap();
    let find = |slug: &str| items.iter().find(|i| i["slug"] == slug).unwrap();
    assert_eq!(
        (
            find("joomla")["needs_database"].as_bool(),
            find("joomla")["needs_admin"].as_bool()
        ),
        (Some(true), Some(true))
    );
    assert_eq!(
        (
            find("static-site")["needs_database"].as_bool(),
            find("static-site")["needs_admin"].as_bool()
        ),
        (Some(false), Some(false))
    );
    assert_eq!(find("laravel")["document_root"], "public");
    let path = "/api/v1/client/apps/install";
    // A site made of files only: no database, no operation yet, straight to the installer.
    let (status, made) = e
        .call(&a, "POST", path, Some(json!({"slug": "static-site", "domain_id": site, "path": "/promo", "title": "Promo page"})))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        (made["kind"].as_str(), made["status"].as_str()),
        (Some("static-site"), Some("installing"))
    );
    let stage: serde_json::Value =
        sqlx::query_scalar("SELECT settings FROM apps WHERE id = $1::uuid")
            .bind(made["id"].as_str().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stage["stage"], "grant");
    assert_eq!(stage["definition"]["slug"], "static-site");
    let databases: i64 = sqlx::query_scalar("SELECT count(*) FROM databases")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(databases, 0);
    // Joomla needs a database and an administrator.
    let joomla = json!({"slug": "joomla", "domain_id": site, "path": "/cms", "title": "My Joomla",
        "admin_user": "boss", "admin_email": "boss@example.com", "admin_password": "a-long-enough-pass-1"});
    let (status, made) = e.call(&a, "POST", path, Some(joomla.clone())).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM databases")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(names, ["alice_joomla1"]);
    let ledger: String =
        sqlx::query_scalar("SELECT coalesce(string_agg(payload::text, ' '), '') FROM operations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!ledger.contains("a-long-enough-pass-1"));
    // The same place is taken; an unknown application, a missing administrator and a title
    // with markup are refused.
    let (status, again) = e.call(&a, "POST", path, Some(joomla.clone())).await;
    assert_eq!(
        (status, again["code"].as_str()),
        (StatusCode::CONFLICT, Some("place_taken")),
        "{again}"
    );
    let (status, _) = e
        .call(
            &a,
            "POST",
            path,
            Some(json!({"slug": "nope", "domain_id": site, "title": "x"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let mut no_admin = joomla.clone();
    no_admin["path"] = json!("/other");
    no_admin["admin_user"] = json!("");
    assert_eq!(
        e.call(&a, "POST", path, Some(no_admin)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut markup = joomla;
    markup["path"] = json!("/markup");
    markup["title"] = json!("<script>alert(1)</script>");
    assert_eq!(
        e.call(&a, "POST", path, Some(markup)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // WordPress through the same door goes the way of its own installer.
    let (status, wp) = e
        .call(&a, "POST", path, Some(json!({"slug": "wordpress", "domain_id": site, "path": "/wp", "title": "WP",
            "admin_user": "boss", "admin_email": "boss@example.com", "admin_password": "a-long-enough-pass-1"})))
        .await;
    assert_eq!(
        (status, wp["kind"].as_str()),
        (StatusCode::ACCEPTED, Some("wordpress")),
        "{wp}"
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_cache_server_is_asked_for_with_a_sealed_password_and_within_the_plans_memory(
    pool: PgPool,
) {
    use builtpanel_database::cache_instances as store;
    let e = env(&pool).await;
    let (alice_account, alice) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice_account).await;
    let path = "/api/v1/client/cache";
    let (status, none) = e.call(&a, "GET", path, None).await;
    assert_eq!(status, StatusCode::OK, "{none}");
    assert_eq!(
        (
            none["items"].as_array().unwrap().len(),
            none["limit_mb"].as_i64()
        ),
        (0, Some(64))
    );
    // A Redis: pending, with a password only the owner sees once it is there.
    let (status, made) = e
        .call(
            &a,
            "POST",
            path,
            Some(json!({"kind": "redis", "memory_mb": 48})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        (
            made["kind"].as_str(),
            made["status"].as_str(),
            made["memory_mb"].as_i64()
        ),
        (Some("redis"), Some("pending"), Some(48))
    );
    let password = made["password"]
        .as_str()
        .expect("the owner sees the password")
        .to_owned();
    assert!(password.len() >= 20);
    let ledger: String =
        sqlx::query_scalar("SELECT string_agg(payload::text, ' ') FROM operations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!ledger.contains(&password), "{ledger}");
    let kind: String = sqlx::query_scalar("SELECT kind FROM operations WHERE tenant_id = $1")
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kind, "set_cache_instance");
    // 48 + 32 > 64: the plan's memory is for all of them together.
    let (status, got) = e
        .call(
            &a,
            "POST",
            path,
            Some(json!({"kind": "memcached", "memory_mb": 32})),
        )
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("limit_reached")),
        "{got}"
    );
    let (status, _) = e
        .call(
            &a,
            "POST",
            path,
            Some(json!({"kind": "memcached", "memory_mb": 16})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    for bad in [
        json!({"kind": "etcd"}),
        json!({"kind": "redis", "memory_mb": 0}),
        json!({"kind": "redis", "memory_mb": 99999}),
    ] {
        assert_eq!(
            e.call(&a, "POST", path, Some(bad)).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    // The node answers; the owner sees how to reach it. A second change keeps the password.
    let row = store::get(&pool, alice, "redis").await.unwrap().unwrap();
    store::ready(&pool, row.id, "bp-alice-redis", 6379)
        .await
        .unwrap();
    let (_, list) = e.call(&a, "GET", path, None).await;
    let redis = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "redis")
        .unwrap();
    assert_eq!(
        (
            redis["status"].as_str(),
            redis["host"].as_str(),
            redis["port"].as_i64()
        ),
        (Some("ready"), Some("bp-alice-redis"), Some(6379))
    );
    assert_eq!(redis["password"].as_str(), Some(password.as_str()));
    let (status, again) = e
        .call(
            &a,
            "POST",
            path,
            Some(json!({"kind": "redis", "memory_mb": 40})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        (again["password"].as_str(), again["memory_mb"].as_i64()),
        (Some(password.as_str()), Some(40))
    );
    // Removing goes through the node too; nobody else's.
    assert_eq!(
        e.call(&a, "DELETE", &format!("{path}/etcd"), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(&a, "DELETE", &format!("{path}/redis"), None).await.0,
        StatusCode::ACCEPTED
    );
    let (_, list) = e.call(&a, "GET", path, None).await;
    let redis = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "redis")
        .unwrap();
    assert_eq!(redis["status"], "removing");
    // A plan without the feature has none.
    sqlx::query("UPDATE feature_sets SET all_features = false, features = ARRAY['files']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)").execute(&pool).await.unwrap();
    let (status, got) = e.call(&a, "GET", path, None).await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("feature_disabled"))
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_client_runs_its_own_applications_within_its_plan_and_only_sees_its_own(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, alice) = client(&pool, "alice", 10_000).await;
    let (bob_account, _bob) = client(&pool, "bob", 10_001).await;
    let a = e.sign_in(alice_account).await;
    let b = e.sign_in(bob_account).await;
    let path = "/api/v1/client/runtime-apps";
    // The plan has to include the feature.
    sqlx::query("UPDATE feature_sets SET all_features = false, features = ARRAY['files']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)").execute(&pool).await.unwrap();
    let (status, body) = e.call(&a, "GET", path, None).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("feature_disabled"))
    );
    sqlx::query("UPDATE feature_sets SET features = features || ARRAY['runtime_apps']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tenants SET status = 'active'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, v) = e.call(&a, "GET", &format!("{path}/runtimes"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        v["node"].as_array().unwrap().iter().any(|x| x == "24"),
        "{v}"
    );
    assert!(
        v["python"].as_array().unwrap().iter().any(|x| x == "3.13"),
        "{v}"
    );
    let (_, none) = e.call(&a, "GET", path, None).await;
    assert_eq!(
        (
            none["items"].as_array().unwrap().len(),
            none["limit"].as_i64()
        ),
        (0, Some(3))
    );

    // Input that is not valid.
    let ok = json!({"name": "api", "kind": "node", "version": "24", "dir": "api", "start_command": "node server.js", "env": {"SECRET_KEY": "s3cr3t-value-123"}});
    for (what, change) in [
        ("name", json!({"name": "A"})),
        ("kind", json!({"kind": "ruby"})),
        ("version", json!({"version": "99"})),
        ("dir", json!({"dir": "../etc"})),
        ("port", json!({"port": 80})),
        ("memory", json!({"memory_mb": 8})),
        ("command", json!({"start_command": "node 'x"})),
        ("env", json!({"env": {"PATH": "/tmp"}})),
        ("domain", json!({"domain_id": Uuid::now_v7()})),
        ("unknown field", json!({"privileged": true})),
    ] {
        let mut body = ok.clone();
        for (k, v) in change.as_object().unwrap() {
            body[k] = v.clone();
        }
        let (status, got) = e.call(&a, "POST", path, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{what}: {got}");
    }

    // A good one: pending, the secret is sealed and never in the ledger.
    let (status, made) = e.call(&a, "POST", path, Some(ok.clone())).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        (
            made["status"].as_str(),
            made["env_names"].clone(),
            made["install"].as_bool()
        ),
        (Some("pending"), json!(["SECRET_KEY"]), Some(true))
    );
    let id = made["id"].as_str().unwrap().to_owned();
    let (kind, payload): (String, String) =
        sqlx::query_as("SELECT kind, payload::text FROM operations WHERE tenant_id = $1")
            .bind(alice)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kind, "set_runtime_app");
    assert!(
        !payload.contains("s3cr3t-value-123") && !payload.contains("\"env\""),
        "{payload}"
    );
    assert!(payload.contains("\"node\"") && payload.contains("server.js"));
    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT sealed_env FROM runtime_apps WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("s3cr3t-value-123"));
    // The same name twice.
    let (status, got) = e.call(&a, "POST", path, Some(ok.clone())).await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("app_taken")),
        "{got}"
    );

    // Another account sees none of it and cannot touch it.
    let (_, theirs) = e.call(&b, "GET", path, None).await;
    assert_eq!(theirs["items"].as_array().unwrap().len(), 0);
    for (method, p, body) in [
        (
            "PUT",
            format!("{path}/{id}"),
            Some(json!({"start_command": "node x.js"})),
        ),
        ("POST", format!("{path}/{id}/redeploy"), None),
        ("DELETE", format!("{path}/{id}"), None),
    ] {
        let (status, got) = e.call(&b, method, &p, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}: {got}");
    }

    // The plan's number and memory: three applications of 256 MiB, not a fourth, not a big one.
    let (status, got) = e
        .call(&a, "POST", path, Some(json!({"name": "big", "kind": "python", "start_command": "python a.py", "memory_mb": 900})))
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("limit_reached")),
        "{got}"
    );
    for name in ["two", "three"] {
        let (status, got) = e
            .call(&a, "POST", path, Some(json!({"name": name, "kind": "python", "version": "3.13", "start_command": "python a.py"})))
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{got}");
    }
    let (status, got) = e
        .call(
            &a,
            "POST",
            path,
            Some(json!({"name": "four", "kind": "node", "start_command": "node a.js"})),
        )
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("limit_reached")),
        "{got}"
    );

    // Change: the environment stays when it is not given, a name cannot change.
    let (status, got) = e
        .call(
            &a,
            "PUT",
            &format!("{path}/{id}"),
            Some(json!({"start_command": "node app.js", "port": 4000})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{got}");
    assert_eq!(
        (
            got["start_command"].as_str(),
            got["port"].as_i64(),
            got["env_names"].clone()
        ),
        (Some("node app.js"), Some(4000), json!(["SECRET_KEY"]))
    );
    let (status, _) = e
        .call(
            &a,
            "PUT",
            &format!("{path}/{id}"),
            Some(json!({"name": "other", "start_command": "node app.js"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // Redeploy without installing.
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{path}/{id}/redeploy"),
            Some(json!({"install": false})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let install: bool = sqlx::query_scalar("SELECT (payload->>'install')::boolean FROM operations WHERE kind = 'set_runtime_app' ORDER BY id DESC LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!install);

    // A domain: the account's own only, one application each; when the application runs the
    // web server's routes know where to send it.
    let domain = domain(&pool, alice, "app.example.com").await;
    let (status, got) = e
        .call(
            &a,
            "PUT",
            &format!("{path}/{id}"),
            Some(json!({"start_command": "node app.js", "domain_id": domain})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{got}");
    assert_eq!(got["domain"], "app.example.com");
    let two: String = sqlx::query_scalar("SELECT id::text FROM runtime_apps WHERE name = 'two'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (status, got) = e
        .call(
            &a,
            "PUT",
            &format!("{path}/{two}"),
            Some(json!({"start_command": "python a.py", "domain_id": domain})),
        )
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("app_taken")),
        "{got}"
    );
    let foreign = domain_of(&pool, bob_account, "bob.example.com").await;
    let (status, _) = e
        .call(
            &a,
            "PUT",
            &format!("{path}/{two}"),
            Some(json!({"start_command": "python a.py", "domain_id": foreign})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let routed = |pool: PgPool| async move {
        builtpanel_database::routes::domains(&pool, alice)
            .await
            .unwrap()
            .into_iter()
            .find(|d| d.fqdn == "app.example.com")
            .unwrap()
            .app_upstream
    };
    assert_eq!(routed(pool.clone()).await, None, "not before it runs");
    let before: i64 = sqlx::query_scalar("SELECT routes_version FROM tenants WHERE id = $1")
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE runtime_apps SET status = 'ready', host = 'bp-alice-app-api:4000' WHERE name = 'api'").execute(&pool).await.unwrap();
    assert_eq!(
        routed(pool.clone()).await,
        Some("bp-alice-app-api:4000".into())
    );
    let after: i64 = sqlx::query_scalar("SELECT routes_version FROM tenants WHERE id = $1")
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        after > before,
        "the site files are made again when the application runs"
    );

    // Removed: pending removal, the domain is the PHP site's again at once.
    let (status, _) = e.call(&a, "DELETE", &format!("{path}/{id}"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(routed(pool.clone()).await, None);
    let (kind, status): (String, String) = sqlx::query_as("SELECT o.kind, r.status FROM runtime_apps r JOIN operations o ON o.id = r.op_id WHERE r.name = 'api'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        (kind.as_str(), status.as_str()),
        ("remove_runtime_app", "removing")
    );
}

async fn domain_of(pool: &PgPool, account: Uuid, fqdn: &str) -> Uuid {
    let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE owner_account_id = $1")
        .bind(account)
        .fetch_one(pool)
        .await
        .unwrap();
    domain(pool, tenant, fqdn).await
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_deploy_from_git_is_asked_for_by_the_owner_and_started_by_a_signed_push(pool: PgPool) {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt as _;
    let e = env(&pool).await;
    let (alice_account, alice) = client(&pool, "alice", 10_000).await;
    let (bob_account, _bob) = client(&pool, "bob", 10_001).await;
    let a = e.sign_in(alice_account).await;
    let b = e.sign_in(bob_account).await;
    let path = "/api/v1/client/git-deploys";
    sqlx::query("UPDATE feature_sets SET all_features = false, features = ARRAY['files']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)").execute(&pool).await.unwrap();
    let (status, body) = e.call(&a, "GET", path, None).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("feature_disabled"))
    );
    sqlx::query("UPDATE feature_sets SET features = features || ARRAY['git_deploy', 'runtime_apps']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)").execute(&pool).await.unwrap();
    sqlx::query("UPDATE tenants SET status = 'active'")
        .execute(&pool)
        .await
        .unwrap();

    // Input that is not valid.
    let ok = json!({"name": "site", "repo_url": "https://github.com/alice/site.git", "dest": "site",
                    "build_steps": ["npm ci", "npm run build"], "build_kind": "node", "token": "ghp_secrettoken123"});
    for (what, change) in [
        ("name", json!({"name": "S"})),
        (
            "http",
            json!({"repo_url": "http://github.com/alice/site.git"}),
        ),
        (
            "login",
            json!({"repo_url": "https://user:pw@github.com/alice/site.git"}),
        ),
        ("ssh", json!({"repo_url": "git@github.com:alice/site.git"})),
        ("branch", json!({"branch": "--upload-pack=x"})),
        ("dest", json!({"dest": "../etc"})),
        ("release folder", json!({"dest": ".bp-releases/x"})),
        (
            "steps",
            json!({"build_steps": ["a", "b", "c", "d", "e", "f"]}),
        ),
        ("quote", json!({"build_steps": ["npm 'x"]})),
        ("kind", json!({"build_kind": "ruby"})),
        ("version", json!({"build_version": "99"})),
        ("app", json!({"app_id": Uuid::now_v7()})),
        ("unknown field", json!({"privileged": true})),
    ] {
        let mut body = ok.clone();
        for (k, v) in change.as_object().unwrap() {
            body[k] = v.clone();
        }
        let (status, got) = e.call(&a, "POST", path, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{what}: {got}");
    }

    // A good one: queued, the webhook secret is shown once, the token and the secret are sealed.
    let (status, made) = e.call(&a, "POST", path, Some(ok.clone())).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        (
            made["status"].as_str(),
            made["has_token"].as_bool(),
            made["branch"].as_str()
        ),
        (Some("deploying"), Some(true), Some("main"))
    );
    let id = made["id"].as_str().unwrap().to_owned();
    let secret = made["webhook_secret"]
        .as_str()
        .expect("shown once")
        .to_owned();
    assert!(secret.len() >= 32);
    assert_eq!(
        made["webhook_path"],
        format!("/api/v1/public/git-webhook/{id}")
    );
    let payload: String =
        sqlx::query_scalar("SELECT payload::text FROM operations WHERE kind = 'run_git_deploy'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !payload.contains("ghp_secrettoken123") && !payload.contains(&secret),
        "{payload}"
    );
    assert!(
        payload.contains("\"npm\"") && payload.contains("\"build\""),
        "the steps are split into words: {payload}"
    );
    let (_, again) = e.call(&a, "GET", path, None).await;
    assert!(
        again["items"][0]["webhook_secret"].is_null(),
        "not shown again"
    );
    assert_eq!(again["limit"], 5);
    // The same name or folder twice; a deploy that is running cannot be started again.
    let (status, got) = e.call(&a, "POST", path, Some(ok.clone())).await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("deploy_taken")),
        "{got}"
    );
    let (status, got) = e
        .call(&a, "POST", &format!("{path}/{id}/deploy"), None)
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("deploy_running")),
        "{got}"
    );

    // Another account sees none of it.
    let (_, theirs) = e.call(&b, "GET", path, None).await;
    assert_eq!(theirs["items"].as_array().unwrap().len(), 0);
    for (method, p) in [
        ("POST", format!("{path}/{id}/deploy")),
        ("POST", format!("{path}/{id}/webhook-secret")),
        ("DELETE", format!("{path}/{id}")),
    ] {
        let (status, got) = e.call(&b, method, &p, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}: {got}");
    }

    // The webhook: no session, the signature is the proof.
    sqlx::query("UPDATE git_deploys SET status = 'idle', op_id = NULL")
        .execute(&pool)
        .await
        .unwrap();
    let push = |headers: Vec<(&'static str, String)>, body: String| {
        let app = e.app.clone();
        let uri = format!("/api/v1/public/git-webhook/{id}");
        async move {
            let mut req = Request::builder().method("POST").uri(uri);
            for (k, v) in headers {
                req = req.header(k, v);
            }
            let res = app
                .oneshot(req.body(Body::from(body)).unwrap())
                .await
                .unwrap();
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 16)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
            )
        }
    };
    let sign = |secret: &str, body: &str| {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let hex: String = ring::hmac::sign(&key, body.as_bytes())
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        vec![
            ("x-hub-signature-256", format!("sha256={hex}")),
            ("x-github-event", "push".to_owned()),
        ]
    };
    let on_main = r#"{"ref":"refs/heads/main"}"#.to_owned();
    let (status, got) = push(vec![], on_main.clone()).await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("bad_signature")),
        "{got}"
    );
    let (status, _) = push(sign("not-the-secret", &on_main), on_main.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'run_git_deploy'")
            .fetch_one(&pool)
            .await
            .unwrap();
    // Another branch, a ping: nothing starts.
    let other = r#"{"ref":"refs/heads/dev"}"#.to_owned();
    let (status, got) = push(sign(&secret, &other), other.clone()).await;
    assert_eq!(
        (status, got["deployed"].as_bool(), got["reason"].as_str()),
        (StatusCode::OK, Some(false), Some("another branch"))
    );
    let mut ping = sign(&secret, "{}");
    ping[1] = ("x-github-event", "ping".to_owned());
    let (status, got) = push(ping, "{}".to_owned()).await;
    assert_eq!(
        (status, got["deployed"].as_bool()),
        (StatusCode::OK, Some(false))
    );
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE kind = 'run_git_deploy'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    // A push to the branch starts it; a second one while it runs does not.
    let (status, got) = push(sign(&secret, &on_main), on_main.clone()).await;
    assert_eq!(
        (status, got["deployed"].as_bool()),
        (StatusCode::ACCEPTED, Some(true)),
        "{got}"
    );
    let (status, got) = push(sign(&secret, &on_main), on_main.clone()).await;
    assert_eq!(
        (status, got["deployed"].as_bool(), got["reason"].as_str()),
        (StatusCode::OK, Some(false), Some("a deploy is running"))
    );
    // GitLab's way.
    sqlx::query("UPDATE git_deploys SET status = 'idle', op_id = NULL")
        .execute(&pool)
        .await
        .unwrap();
    let (status, got) = push(vec![("x-gitlab-token", secret.clone())], on_main.clone()).await;
    assert_eq!(
        (status, got["deployed"].as_bool()),
        (StatusCode::ACCEPTED, Some(true)),
        "{got}"
    );
    // A deploy that does not exist answers like a wrong signature.
    let (status, _) = e
        .call(
            &a,
            "POST",
            &format!("{path}/{}/deploy", Uuid::now_v7()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A new secret: the old one stops working.
    sqlx::query("UPDATE git_deploys SET status = 'idle', op_id = NULL")
        .execute(&pool)
        .await
        .unwrap();
    let (status, rotated) = e
        .call(&a, "POST", &format!("{path}/{id}/webhook-secret"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    let new_secret = rotated["webhook_secret"].as_str().unwrap().to_owned();
    assert_ne!(new_secret, secret);
    let (status, _) = push(sign(&secret, &on_main), on_main.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = push(sign(&new_secret, &on_main), on_main.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // A suspended account's push does nothing.
    sqlx::query("UPDATE git_deploys SET status = 'idle', op_id = NULL")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tenants SET status = 'suspended' WHERE id = $1")
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();
    let (status, got) = push(sign(&new_secret, &on_main), on_main.clone()).await;
    assert_eq!(
        (status, got["deployed"].as_bool()),
        (StatusCode::OK, Some(false))
    );
    sqlx::query("UPDATE tenants SET status = 'active' WHERE id = $1")
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();

    // Removal is queued.
    sqlx::query("UPDATE git_deploys SET status = 'idle', op_id = NULL")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = e.call(&a, "DELETE", &format!("{path}/{id}"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (kind, st): (String, String) = sqlx::query_as(
        "SELECT o.kind, g.status FROM git_deploys g JOIN operations o ON o.id = g.op_id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (kind.as_str(), st.as_str()),
        ("remove_git_deploy", "removing")
    );
}
