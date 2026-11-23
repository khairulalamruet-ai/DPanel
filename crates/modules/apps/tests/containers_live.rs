//! Containers from approved images over HTTP against real PostgreSQL and Redis (T12.12). Ignored by default;
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

fn image(name: &str, digest_char: char) -> Value {
    json!({
        "name": name,
        "reference": format!("nginxinc/nginx-unprivileged@sha256:{}", digest_char.to_string().repeat(64)),
        "default_port": 8080, "default_memory_mb": 128, "note": "a web server"
    })
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_catalog_is_the_administrators_and_clients_only_choose_from_it(pool: PgPool) {
    let e = env(&pool).await;
    let root = e.sign_in(admin(&pool).await).await;
    let (account, _) = client(&pool, "alice", 10_000).await;
    let alice = e.sign_in(account).await;
    let path = "/api/v1/admin/container-images";

    // A client has nothing to do with the catalog's administration.
    let (status, _) = e.call(&alice, "POST", path, Some(image("Web", 'a'))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Nothing is offered until the administrator adds something.
    let (status, none) = e
        .call(&alice, "GET", "/api/v1/client/container-images", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{none}");
    assert_eq!(none.as_array().unwrap().len(), 0);

    // A tag can move, so only a digest is accepted.
    let mut tagged = image("Tagged", 'b');
    tagged["reference"] = json!("nginx:latest");
    let (status, got) = e.call(&root, "POST", path, Some(tagged)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{got}");
    assert!(got.to_string().contains("not_pinned"), "{got}");
    let mut bad_port = image("Port", 'c');
    bad_port["default_port"] = json!(80);
    let (status, _) = e.call(&root, "POST", path, Some(bad_port)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, web) = e.call(&root, "POST", path, Some(image("Web", 'a'))).await;
    assert_eq!(status, StatusCode::CREATED, "{web}");
    let id = web["id"].as_str().unwrap().to_owned();
    let (status, got) = e.call(&root, "POST", path, Some(image("Web", 'd'))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    let (status, got) = e.call(&root, "POST", path, Some(image("Same", 'a'))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    let (_, off) = e.call(&root, "POST", path, Some(image("Old", 'e'))).await;
    let off_id = off["id"].as_str().unwrap().to_owned();
    let (status, _) = e
        .call(
            &root,
            "PUT",
            &format!("{path}/{off_id}"),
            Some(json!({"name": "Old", "default_port": 8080, "enabled": false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Clients see the enabled ones, and never the registry address.
    let (_, offered) = e
        .call(&alice, "GET", "/api/v1/client/container-images", None)
        .await;
    let offered = offered.as_array().unwrap();
    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0]["name"], "Web");
    assert!(offered[0]["reference"].is_null());
    let (_, all) = e.call(&root, "GET", path, None).await;
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert!(all[0]["reference"].as_str().unwrap().contains("@sha256:"));
    // An image's reference does not change.
    let mut change = image("Web", 'f');
    change["reference"] = json!(format!("x/y@sha256:{}", "f".repeat(64)));
    let (status, _) = e
        .call(&root, "PUT", &format!("{path}/{id}"), Some(change))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_container_runs_an_approved_image_and_nothing_can_widen_what_it_may_do(pool: PgPool) {
    let e = env(&pool).await;
    let root = e.sign_in(admin(&pool).await).await;
    let (account, tenant) = client(&pool, "alice", 10_000).await;
    let (other_account, other_tenant) = client(&pool, "bob", 10_001).await;
    let alice = e.sign_in(account).await;
    let bob = e.sign_in(other_account).await;
    let site = domain(&pool, tenant, "alice.example").await;
    let bobs_site = domain(&pool, other_tenant, "bob.example").await;
    let (_, web) = e
        .call(
            &root,
            "POST",
            "/api/v1/admin/container-images",
            Some(image("Web", 'a')),
        )
        .await;
    let image_id = web["id"].as_str().unwrap().to_owned();
    let (_, old) = e
        .call(
            &root,
            "POST",
            "/api/v1/admin/container-images",
            Some({
                let mut i = image("Old", 'b');
                i["enabled"] = json!(false);
                i
            }),
        )
        .await;
    let base = "/api/v1/client/container-apps";

    // Privileged mode, mounts of the host, capabilities, another network, a command of the
    // client's own: there is no field for any of it, so none is accepted.
    for (field, value) in [
        ("privileged", json!(true)),
        ("volumes", json!(["/:/host"])),
        ("mounts", json!([{"source": "/", "target": "/host"}])),
        (
            "binds",
            json!(["/var/run/docker.sock:/var/run/docker.sock"]),
        ),
        ("cap_add", json!(["SYS_ADMIN"])),
        ("network_mode", json!("host")),
        ("user", json!("root")),
        ("command", json!(["sh"])),
        ("image", json!("evil@sha256:".to_owned() + &"0".repeat(64))),
        ("pid_mode", json!("host")),
    ] {
        let mut body = json!({"name": "web", "image_id": image_id});
        body[field] = value;
        let (status, got) = e.call(&alice, "POST", base, Some(body)).await;
        assert!(status.is_client_error(), "{field}: {status} {got}");
    }
    let made: i64 = sqlx::query_scalar("SELECT count(*) FROM runtime_apps")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(made, 0, "a refused request made nothing");
    // An image that is not offered, or does not exist, cannot be chosen.
    let (status, _) = e
        .call(
            &alice,
            "POST",
            base,
            Some(json!({"name": "web", "image_id": old["id"]})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = e
        .call(
            &alice,
            "POST",
            base,
            Some(json!({"name": "web", "image_id": Uuid::new_v4()})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // Another account's domain is not yours.
    let (status, _) = e
        .call(
            &alice,
            "POST",
            base,
            Some(json!({"name": "web", "image_id": image_id, "domain_id": bobs_site})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, made) = e
        .call(
            &alice,
            "POST",
            base,
            Some(json!({
                "name": "web", "image_id": image_id, "domain_id": site,
                "env": {"GREETING": "hello"}
            })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(made["image"], "Web");
    assert_eq!(made["port"], 8080);
    assert_eq!(made["memory_mb"], 128);
    assert_eq!(made["env_names"], json!(["GREETING"]));
    assert_eq!(made["status"], "pending");
    let id = made["id"].as_str().unwrap().to_owned();
    // What is sent to the node names the image by digest, has no command and no secret.
    let payload: Value = sqlx::query_scalar(
        "SELECT o.payload FROM runtime_apps r JOIN operations o ON o.id = r.op_id WHERE r.name = 'web'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payload["kind"], "container");
    assert_eq!(payload["command"], json!([]));
    assert_eq!(payload["install"], false);
    assert_eq!(
        payload["image"],
        format!("nginxinc/nginx-unprivileged@sha256:{}", "a".repeat(64))
    );
    assert!(!payload.to_string().contains("hello"), "{payload}");
    assert!(payload.get("env").is_none());

    // It is listed with its image, and only for its owner.
    let (_, list) = e.call(&alice, "GET", base, None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, none) = e.call(&bob, "GET", base, None).await;
    assert_eq!(none.as_array().unwrap().len(), 0);
    let (status, _) = e
        .call(
            &bob,
            "PUT",
            &format!("{base}/{id}"),
            Some(json!({"port": 9000})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The generic route for Node and Python applications does not change a container.
    let (status, _) = e
        .call(
            &alice,
            "PUT",
            &format!("/api/v1/client/runtime-apps/{id}"),
            Some(json!({"start_command": "node a.js"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Change it: the same image, a new port and memory, a new environment.
    let (status, changed) = e
        .call(
            &alice,
            "PUT",
            &format!("{base}/{id}"),
            Some(json!({"port": 9090, "memory_mb": 256, "env": {"A": "1", "B": "2"}})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{changed}");
    assert_eq!(changed["port"], 9090);
    assert_eq!(changed["env_names"], json!(["A", "B"]));
    let (status, _) = e
        .call(
            &alice,
            "PUT",
            &format!("{base}/{id}"),
            Some(json!({"privileged": true})),
        )
        .await;
    assert!(status.is_client_error());
    let (status, _) = e
        .call(
            &alice,
            "PUT",
            &format!("{base}/{id}"),
            Some(json!({"memory_mb": 100_000})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // An image a container runs cannot be deleted; stop offering it instead.
    let (status, got) = e
        .call(
            &root,
            "DELETE",
            &format!("/api/v1/admin/container-images/{image_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    assert_eq!(got["code"], "image_in_use");
    let (status, _) = e
        .call(
            &root,
            "DELETE",
            &format!(
                "/api/v1/admin/container-images/{}",
                old["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn customers_install_only_what_the_administrator_offers(pool: PgPool) {
    let e = env(&pool).await;
    let root = e.sign_in(admin(&pool).await).await;
    let (account, _) = client(&pool, "alice", 10_000).await;
    let alice = e.sign_in(account).await;

    // The administrator sees the whole marketplace; nothing is offered yet.
    let (status, all) = e
        .call(&root, "GET", "/api/v1/admin/marketplace", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let all = all.as_array().unwrap();
    assert!(all.len() > 50);
    assert!(all.iter().all(|s| s["offered"] == json!(false)));
    let container = all
        .iter()
        .find(|s| s["kind"] == "container")
        .expect("a container entry")["slug"]
        .as_str()
        .unwrap()
        .to_owned();
    let builtin = all.iter().find(|s| s["kind"] == "builtin").unwrap()["slug"]
        .as_str()
        .unwrap()
        .to_owned();

    // A customer sees no container software, and cannot install it.
    let (_, mine) = e
        .call(&alice, "GET", "/api/v1/client/marketplace", None)
        .await;
    assert!(
        mine.as_array()
            .unwrap()
            .iter()
            .all(|s| s["kind"] != "container")
    );
    let (status, got) = e
        .call(
            &alice,
            "POST",
            &format!("/api/v1/client/marketplace/{container}/install"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    assert!(got.to_string().contains("not_offered"), "{got}");

    // Only an administrator offers; the panel's own software cannot be offered.
    let path = format!("/api/v1/admin/marketplace/{container}");
    let (status, _) = e.call(&alice, "POST", &path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = e
        .call(
            &root,
            "POST",
            &format!("/api/v1/admin/marketplace/{builtin}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, got) = e.call(&root, "POST", &path, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{got}");
    let (status, got) = e.call(&root, "POST", &path, None).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "offering twice is fine: {got}"
    );

    // Now it is offered, to the administrator's list and to customers.
    let (_, all) = e
        .call(&root, "GET", "/api/v1/admin/marketplace", None)
        .await;
    let row = all
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["slug"] == json!(container))
        .unwrap();
    assert_eq!(row["offered"], json!(true));
    let (_, mine) = e
        .call(&alice, "GET", "/api/v1/client/marketplace", None)
        .await;
    assert!(
        mine.as_array()
            .unwrap()
            .iter()
            .any(|s| s["slug"] == json!(container))
    );

    // Removing it stops the offer again.
    let (status, _) = e.call(&root, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, mine) = e
        .call(&alice, "GET", "/api/v1/client/marketplace", None)
        .await;
    assert!(
        mine.as_array()
            .unwrap()
            .iter()
            .all(|s| s["kind"] != "container")
    );
    let (status, _) = e
        .call(
            &root,
            "DELETE",
            "/api/v1/admin/marketplace/no-such-thing",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
