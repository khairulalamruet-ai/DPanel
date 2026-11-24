//! Feature sets over HTTP, against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, OnceLock};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_cache::RedisHandle;
use builtpanel_database::{modules as db, rbac::assign_role};
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_plans::{PlansState, router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const SETS: &str = "/api/v1/admin/feature-sets";

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

fn manifest(name: &str, category: &str, core: bool, features: &[&str]) -> Manifest {
    manifest_with(name, category, core, features, "")
}

fn manifest_with(
    name: &str,
    category: &str,
    core: bool,
    features: &[&str],
    extra: &str,
) -> Manifest {
    let keys = features
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ");
    Manifest::parse(
        name,
        &format!(
            "[module]\nname = \"{name}\"\nversion = \"1.0.0\"\ntitle = {{ en = \"{name} title\" }}\ncategory = \"{category}\"\ncore = {core}\n[features]\nkeys = [{keys}]\n{extra}"
        ),
    )
    .unwrap()
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
    let late: Arc<OnceLock<ModuleMount>> = Arc::default();
    let plans = Arc::new(Fake {
        manifest: manifest("plans", "core", true, &[]),
        routes: Box::new({
            let (pool, sessions, late) = (pool.clone(), sessions.clone(), late.clone());
            move || match late.get() {
                Some(mount) => ModuleRoutes {
                    admin: Some(router().with_state(PlansState {
                        db: pool.clone(),
                        sessions: sessions.clone(),
                        mount: mount.clone(),
                    })),
                    ..ModuleRoutes::none()
                },
                None => ModuleRoutes::none(),
            }
        }),
    });
    let plain = |m: Manifest| -> Arc<dyn Module> {
        Arc::new(Fake {
            manifest: m,
            routes: Box::new(ModuleRoutes::none),
        })
    };
    let registry = Registry::build(vec![
        plans,
        plain(manifest_with(
            "domains",
            "web",
            false,
            &["domains", "redirects"],
            "[limits]\nmax_domains = { type = \"int\", default = 10, unlimited = -1 }\nmax_seats = { type = \"int\", default = 3 }\n",
        )),
        plain(manifest("db-mysql", "db", false, &["databases"])),
    ])
    .unwrap();
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
    late.set(mount.clone()).ok();
    mount.rebuild();
    mount.apply(&db::enabled_names(pool).await.unwrap().into_iter().collect());
    Env {
        app: mount.service(),
        sessions,
    }
}

async fn account(pool: &PgPool, name: &str, roles: &[&str]) -> Uuid {
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
        who: Option<&Browser>,
        method: &str,
        path: &str,
        body: Option<Value>,
        csrf: bool,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(b) = who {
            req = req.header(header::COOKIE, &b.cookie);
            if csrf {
                req = req.header(CSRF_HEADER, &b.csrf);
            }
        }
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

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

fn id_of(set: &Value) -> String {
    set["id"].as_str().unwrap().to_owned()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_feature_catalogue_and_the_presets_are_listed(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;

    assert_eq!(
        e.call(None, "GET", SETS, None, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = e.call(Some(&plain), "GET", SETS, None, false).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );

    // The catalogue: what the installed modules declare, by category.
    let (status, body) = e
        .call(Some(&root), "GET", "/api/v1/admin/features", None, false)
        .await;
    assert_eq!(status, StatusCode::OK);
    let items: Vec<(String, String, String)> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["category"].as_str().unwrap().into(),
                f["module"].as_str().unwrap().into(),
                f["key"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        items,
        [
            (
                "db".to_owned(),
                "db-mysql".to_owned(),
                "databases".to_owned()
            ),
            ("web".to_owned(), "domains".to_owned(), "domains".to_owned()),
            (
                "web".to_owned(),
                "domains".to_owned(),
                "redirects".to_owned()
            ),
        ]
    );
    assert_eq!(body["items"][0]["module_title"]["en"], "db-mysql title");

    // The four presets, with how many plans use each.
    let (status, body) = e.call(Some(&root), "GET", SETS, None, false).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["basic", "db_only", "default", "dns_only"]);
    let default = &body["items"][2];
    assert_eq!(
        (
            default["all_features"].as_bool(),
            default["built_in"].as_bool(),
            default["plans"].as_i64()
        ),
        (Some(true), Some(true), Some(1))
    );
    // Preset keys no installed module declares yet are reported, not hidden.
    let db_only = &body["items"][1];
    assert_eq!(db_only["features"], json!(["databases"]));
    assert_eq!(
        db_only["unknown_features"],
        json!([]),
        "this fixture declares `databases`"
    );
    assert_eq!(body["items"][3]["unknown_features"], json!(["dns"]));
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_feature_set_can_be_created_edited_and_deleted_with_an_audit_trail(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;

    let (status, set) = e
        .call(
            Some(&root),
            "POST",
            SETS,
            Some(json!({"name": "web_only", "description": "Sites", "features": ["redirects", "domains", "domains", "mystery"]})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{set}");
    assert_eq!(
        set["features"],
        json!(["domains", "mystery", "redirects"]),
        "sorted, no duplicates"
    );
    assert_eq!(set["unknown_features"], json!(["mystery"]));
    assert_eq!(
        (set["built_in"].clone(), set["plans"].clone()),
        (json!(false), json!(0))
    );
    let id = id_of(&set);

    let (status, same) = e
        .call(Some(&root), "GET", &format!("{SETS}/{id}"), None, false)
        .await;
    assert_eq!(
        (status, &same["name"]),
        (StatusCode::OK, &json!("web_only"))
    );

    // Edit: rename, swap the list. Then switch to "all features".
    let (status, set) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{SETS}/{id}"),
            Some(json!({"name": "sites", "features": ["domains"]})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!(
        (set["name"].clone(), set["features"].clone()),
        (json!("sites"), json!(["domains"]))
    );
    let (_, set) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{SETS}/{id}"),
            Some(json!({"name": "sites", "all_features": true})),
            true,
        )
        .await;
    assert_eq!(
        (set["all_features"].clone(), set["features"].clone()),
        (json!(true), json!([]))
    );

    // A built-in set can be edited but keeps its name.
    let (_, list) = e.call(Some(&root), "GET", SETS, None, false).await;
    let dns = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "dns_only")
        .unwrap();
    let (status, edited) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{SETS}/{}", id_of(dns)),
            Some(json!({"name": "renamed", "features": ["dns", "ssl"]})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (edited["name"].clone(), edited["features"].clone()),
        (json!("dns_only"), json!(["dns", "ssl"]))
    );

    // Delete.
    let (status, _) = e
        .call(Some(&root), "DELETE", &format!("{SETS}/{id}"), None, true)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        e.call(Some(&root), "GET", &format!("{SETS}/{id}"), None, false)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(Some(&root), "DELETE", &format!("{SETS}/{id}"), None, true)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let audit: Vec<(String, Value)> =
        sqlx::query_as("SELECT action, metadata FROM audit_logs ORDER BY at, id")
            .fetch_all(&pool)
            .await
            .unwrap();
    let actions: Vec<&str> = audit.iter().map(|a| a.0.as_str()).collect();
    assert_eq!(
        actions,
        [
            "admin.feature_set_created",
            "admin.feature_set_updated",
            "admin.feature_set_updated",
            "admin.feature_set_updated",
            "admin.feature_set_deleted"
        ]
    );
    assert_eq!(
        audit[1].1["before"]["features"],
        json!(["domains", "mystery", "redirects"])
    );
    assert_eq!(audit[1].1["after"]["features"], json!(["domains"]));
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn bad_input_names_in_use_and_built_in_sets_are_refused(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    let ok = json!({"name": "fine", "features": ["dns"]});

    // Who may write.
    let (status, body) = e
        .call(Some(&plain), "POST", SETS, Some(ok.clone()), true)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );
    assert_eq!(
        e.call(Some(&root), "POST", SETS, Some(ok.clone()), false)
            .await
            .0,
        StatusCode::FORBIDDEN,
        "no CSRF header"
    );
    assert_eq!(
        e.call(None, "POST", SETS, Some(ok.clone()), true).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Invalid fields: nothing is created.
    for bad in [
        json!({"name": "Bad Name"}),
        json!({"name": "ok_name", "features": ["Bad Key"]}),
        json!({"name": "ok_name", "all_features": true, "features": ["dns"]}),
        json!({"name": "ok_name", "description": "x".repeat(256)}),
    ] {
        let (status, body) = e
            .call(Some(&root), "POST", SETS, Some(bad.clone()), true)
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {body}");
    }
    let (status, _) = e
        .call(
            Some(&root),
            "POST",
            SETS,
            Some(json!({"name": "ok_name", "force": true})),
            true,
        )
        .await;
    assert!(status.is_client_error(), "unknown fields are refused");

    // Names are unique, on create and on rename.
    let (status, body) = e
        .call(
            Some(&root),
            "POST",
            SETS,
            Some(json!({"name": "basic"})),
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "feature_set_name_taken")
    );
    let (_, mine) = e.call(Some(&root), "POST", SETS, Some(ok), true).await;
    let (status, body) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{SETS}/{}", id_of(&mine)),
            Some(json!({"name": "db_only"})),
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "feature_set_name_taken")
    );

    // Built-in sets cannot be deleted; sets that plans use cannot either.
    let (_, list) = e.call(Some(&root), "GET", SETS, None, false).await;
    let basic = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "basic")
        .unwrap();
    let (status, body) = e
        .call(
            Some(&root),
            "DELETE",
            &format!("{SETS}/{}", id_of(basic)),
            None,
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "feature_set_built_in")
    );
    sqlx::query("INSERT INTO plans (name, feature_set_id) VALUES ('uses_mine', $1::uuid)")
        .bind(id_of(&mine))
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = e
        .call(
            Some(&root),
            "DELETE",
            &format!("{SETS}/{}", id_of(&mine)),
            None,
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "feature_set_in_use")
    );
    assert_eq!(
        e.call(
            Some(&root),
            "GET",
            &format!("{SETS}/{}", id_of(&mine)),
            None,
            false
        )
        .await
        .0,
        StatusCode::OK
    );

    // Not ids.
    assert_eq!(
        e.call(
            Some(&root),
            "GET",
            &format!("{SETS}/not-a-uuid"),
            None,
            false
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(
            Some(&root),
            "PUT",
            &format!("{SETS}/{}", Uuid::now_v7()),
            Some(json!({"name": "valid_name"})),
            true
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

// -------------------------------------------------------------------- plans

const PLANS: &str = "/api/v1/admin/plans";

fn plan(name: &str, feature_set: &str) -> Value {
    json!({
        "name": name,
        "description": "A plan",
        "feature_set_id": feature_set,
        "resources": {"cpu_millis": 1000, "memory_mb": 512, "disk_mb": 5120, "pids": 128},
    })
}

async fn feature_set_id(pool: &PgPool, name: &str) -> String {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM feature_sets WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
        .to_string()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_plan_limits_and_the_presets_are_listed_in_wire_units(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain_admin = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    assert_eq!(
        e.call(None, "GET", PLANS, None, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = e.call(Some(&plain_admin), "GET", PLANS, None, false).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );

    // The limits the installed modules declare.
    let (status, body) = e
        .call(Some(&root), "GET", "/api/v1/admin/plan-limits", None, false)
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        (
            items[0]["key"].as_str(),
            items[0]["default"].as_i64(),
            items[0]["unlimited_allowed"].as_bool()
        ),
        (Some("max_domains"), Some(10), Some(true))
    );
    assert_eq!(
        (
            items[1]["key"].as_str(),
            items[1]["unlimited_allowed"].as_bool()
        ),
        (Some("max_seats"), Some(false))
    );

    // The presets: bytes become megabytes, NULL becomes -1.
    let (status, body) = e.call(Some(&root), "GET", PLANS, None, false).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["basic", "db_only", "default", "dns_only"]);
    let default = &body["items"][2];
    assert_eq!(default["is_default"], true);
    assert_eq!(default["feature_set_name"], "default");
    assert_eq!(default["users_count"], 0);
    assert_eq!(
        default["resources"],
        json!({"cpu_millis": 2000, "memory_mb": 2048, "disk_mb": 20480, "inodes": 500000, "bandwidth_mb": -1, "pids": 512})
    );
    assert_eq!(default["limits"]["max_domains"], -1);
    assert_eq!(
        default["unknown_limits"],
        json!(["max_databases"]),
        "a preset limit no installed module declares"
    );
    let dns = &body["items"][3];
    assert_eq!(
        dns["resources"]["cpu_millis"], -1,
        "no containers: no CPU share"
    );
    assert_eq!(body["items"][0]["resources"]["bandwidth_mb"], 102_400);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_plan_can_be_created_edited_made_default_and_deleted_with_an_audit_trail(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let basic = feature_set_id(&pool, "basic").await;

    // Declared limits left out get their default; given ones are kept (-1 = unlimited).
    let mut body = plan("starter", &basic);
    body["limits"] = json!({"max_domains": 25, "mystery": 7});
    let (status, made) = e.call(Some(&root), "POST", PLANS, Some(body), true).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(
        made["limits"],
        json!({"max_domains": 25, "max_seats": 3, "mystery": 7})
    );
    assert_eq!(made["unknown_limits"], json!(["mystery"]));
    assert_eq!(
        made["resources"],
        json!({"cpu_millis": 1000, "memory_mb": 512, "disk_mb": 5120, "inodes": -1, "bandwidth_mb": -1, "pids": 128})
    );
    assert_eq!(
        (
            made["isolation"].clone(),
            made["is_default"].clone(),
            made["built_in"].clone()
        ),
        (json!("standard"), json!(false), json!(false))
    );
    let id = made["id"].as_str().unwrap().to_owned();
    let stored: (Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT memory_bytes, inodes FROM plans WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        stored,
        (Some(512 * 1024 * 1024), None),
        "megabytes in, bytes stored, -1 is NULL"
    );

    // Edit: another set, strict isolation, unlimited memory.
    let dns_only = feature_set_id(&pool, "dns_only").await;
    let mut edit = plan("starter", &dns_only);
    edit["isolation"] = json!("strict");
    edit["resources"]["memory_mb"] = json!(-1);
    edit["limits"] = json!({"max_domains": -1});
    let (status, edited) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{PLANS}/{id}"),
            Some(edit),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(
        (
            edited["feature_set_name"].clone(),
            edited["isolation"].clone(),
            edited["resources"]["memory_mb"].clone()
        ),
        (json!("dns_only"), json!("strict"), json!(-1))
    );
    assert_eq!(edited["limits"]["max_domains"], -1);

    // Make it the default: the old default stops being one.
    let mut default = plan("starter", &dns_only);
    default["is_default"] = json!(true);
    let (status, now) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{PLANS}/{id}"),
            Some(default),
            true,
        )
        .await;
    assert_eq!(
        (status, now["is_default"].clone()),
        (StatusCode::OK, json!(true))
    );
    let defaults: Vec<String> = sqlx::query_scalar("SELECT name FROM plans WHERE is_default")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(defaults, ["starter"]);
    // The default plan cannot be deleted or demoted; a built-in plan cannot be deleted.
    let (status, b) = e
        .call(Some(&root), "DELETE", &format!("{PLANS}/{id}"), None, true)
        .await;
    assert_eq!(
        (status, code(&b)),
        (StatusCode::CONFLICT, "plan_is_default")
    );
    let (status, b) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{PLANS}/{id}"),
            Some(plan("starter", &dns_only)),
            true,
        )
        .await;
    assert_eq!(
        (status, code(&b)),
        (StatusCode::CONFLICT, "plan_is_default")
    );
    let (_, list) = e.call(Some(&root), "GET", PLANS, None, false).await;
    let preset = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "default")
        .unwrap();
    assert_eq!(preset["is_default"], false);
    let (status, b) = e
        .call(
            Some(&root),
            "DELETE",
            &format!("{PLANS}/{}", preset["id"].as_str().unwrap()),
            None,
            true,
        )
        .await;
    assert_eq!((status, code(&b)), (StatusCode::CONFLICT, "plan_built_in"));
    // A built-in plan keeps its name when edited.
    let (_, renamed) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{PLANS}/{}", preset["id"].as_str().unwrap()),
            Some(plan("renamed", &feature_set_id(&pool, "default").await)),
            true,
        )
        .await;
    assert_eq!(renamed["name"], "default");

    // Another plan, deleted.
    let (_, other) = e
        .call(
            Some(&root),
            "POST",
            PLANS,
            Some(plan("temporary", &basic)),
            true,
        )
        .await;
    let other_id = other["id"].as_str().unwrap();
    assert_eq!(
        e.call(
            Some(&root),
            "DELETE",
            &format!("{PLANS}/{other_id}"),
            None,
            true
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        e.call(
            Some(&root),
            "GET",
            &format!("{PLANS}/{other_id}"),
            None,
            false
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let audit: Vec<(String, Value)> =
        sqlx::query_as("SELECT action, metadata FROM audit_logs ORDER BY at, id")
            .fetch_all(&pool)
            .await
            .unwrap();
    let actions: Vec<&str> = audit.iter().map(|a| a.0.as_str()).collect();
    assert_eq!(
        actions,
        [
            "admin.plan_created",
            "admin.plan_updated",
            "admin.plan_updated",
            "admin.plan_updated",
            "admin.plan_created",
            "admin.plan_deleted"
        ]
    );
    assert_eq!(audit[1].1["before"]["feature_set"], "basic");
    assert_eq!(audit[1].1["after"]["feature_set"], "dns_only");
    assert_eq!(audit[1].1["after"]["isolation"], "strict");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_plan_over_the_biggest_node_is_refused_with_422(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let basic = feature_set_id(&pool, "basic").await;
    let big = |cpu: i64, memory_mb: i64, disk_mb: i64| {
        let mut p = plan("bigplan", "");
        p["resources"] =
            json!({"cpu_millis": cpu, "memory_mb": memory_mb, "disk_mb": disk_mb, "pids": 128});
        p
    };
    let with_set = |mut p: Value| {
        p["feature_set_id"] = json!(basic);
        p
    };

    // No node has reported its capabilities: nothing to compare with.
    let (status, ok) = e
        .call(
            Some(&root),
            "POST",
            PLANS,
            Some(with_set(big(64_000, 1_000_000, 1_000_000))),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ok}");
    e.call(
        Some(&root),
        "DELETE",
        &format!("{PLANS}/{}", ok["id"].as_str().unwrap()),
        None,
        true,
    )
    .await;

    // Two nodes: the largest figure of each kind counts. A revoked node does not.
    for (name, status, cores, memory, disk) in [
        ("small", "online", 2, 4_i64 << 30, 100_i64 << 30),
        ("large", "offline", 8, 16_i64 << 30, 500_i64 << 30),
        ("gone", "revoked", 128, 1_i64 << 40, 1_i64 << 50),
    ] {
        sqlx::query(
            "INSERT INTO nodes (name, status, capabilities, capabilities_at)
             VALUES ($1, $2, jsonb_build_object('cpu_cores', $3::int, 'memory_bytes', $4::bigint, 'disk_total_bytes', $5::bigint), now())",
        )
        .bind(name)
        .bind(status)
        .bind(cores)
        .bind(memory)
        .bind(disk)
        .execute(&pool)
        .await
        .unwrap();
    }
    // Exactly the largest node: fine.
    let (status, ok) = e
        .call(
            Some(&root),
            "POST",
            PLANS,
            Some(with_set(big(8000, 16_384, 512_000))),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ok}");
    let ok_id = ok["id"].as_str().unwrap().to_owned();
    // One step over on each resource.
    let (status, body) = e
        .call(
            Some(&root),
            "POST",
            PLANS,
            Some(with_set(big(8001, 16_385, 512_001))),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let mut fields: Vec<(String, String)> = body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["field"].as_str().unwrap().to_owned(),
                f["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        [
            (
                "resources.cpu_millis".to_owned(),
                "exceeds_node_capacity".to_owned()
            ),
            (
                "resources.disk_mb".to_owned(),
                "exceeds_node_capacity".to_owned()
            ),
            (
                "resources.memory_mb".to_owned(),
                "exceeds_node_capacity".to_owned()
            ),
        ]
    );
    assert!(
        body["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("largest node")
    );
    // Unlimited is "whatever the node has" and always allowed; editing is checked too.
    let (status, _) = e.call(Some(&root), "POST", PLANS, Some(with_set(json!({"name": "open", "description": "", "resources": {"cpu_millis": -1, "memory_mb": -1, "disk_mb": -1, "pids": -1}}))), true).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{PLANS}/{ok_id}"),
            Some(with_set(big(9000, 512, 5120))),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM plans WHERE name = 'bigplan'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "the refused plan was not stored");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn plan_input_names_feature_sets_and_permissions_are_checked(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain_admin = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    let basic = feature_set_id(&pool, "basic").await;
    let good = plan("fine", &basic);

    let (status, body) = e
        .call(Some(&plain_admin), "POST", PLANS, Some(good.clone()), true)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );
    assert_eq!(
        e.call(Some(&root), "POST", PLANS, Some(good.clone()), false)
            .await
            .0,
        StatusCode::FORBIDDEN,
        "no CSRF header"
    );
    assert_eq!(
        e.call(None, "POST", PLANS, Some(good.clone()), true)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    // Bad values: one 422 listing every field.
    let mut bad = plan("Bad Name", &basic);
    bad["resources"] = json!({"cpu_millis": 5, "memory_mb": 1, "disk_mb": 0, "pids": 0});
    bad["isolation"] = json!("paranoid");
    bad["limits"] = json!({"max_seats": -1});
    let (status, body) = e.call(Some(&root), "POST", PLANS, Some(bad), true).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["errors"].as_array().unwrap().len() >= 7, "{body}");
    // A feature set that does not exist; missing and unknown fields.
    let (status, body) = e
        .call(
            Some(&root),
            "POST",
            PLANS,
            Some(plan("ghostset", &Uuid::now_v7().to_string())),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"][0]["field"], "feature_set_id");
    for body in [
        json!({"name": "nores", "feature_set_id": basic}),
        json!({"name": "extra", "feature_set_id": basic, "resources": {"cpu_millis": 1000, "memory_mb": 512, "disk_mb": 5120, "pids": 128}, "force": true}),
        json!({"name": "partial", "feature_set_id": basic, "resources": {"cpu_millis": 1000}}),
    ] {
        let (status, _) = e
            .call(Some(&root), "POST", PLANS, Some(body.clone()), true)
            .await;
        assert!(status.is_client_error(), "{body}");
    }
    // Names are unique, on create and on rename.
    let (status, body) = e
        .call(
            Some(&root),
            "POST",
            PLANS,
            Some(plan("basic", &basic)),
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "plan_name_taken")
    );
    let (_, mine) = e.call(Some(&root), "POST", PLANS, Some(good), true).await;
    let (status, body) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{PLANS}/{}", mine["id"].as_str().unwrap()),
            Some(plan("db_only", &basic)),
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "plan_name_taken")
    );
    // Not ids.
    assert_eq!(
        e.call(Some(&root), "GET", &format!("{PLANS}/nope"), None, false)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(
            Some(&root),
            "DELETE",
            &format!("{PLANS}/{}", Uuid::now_v7()),
            None,
            true
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM plans")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 5, "four presets and the one good plan");
}
