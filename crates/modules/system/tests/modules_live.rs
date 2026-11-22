//! Switching modules over HTTP, against real PostgreSQL and Redis, with the
//! routes really appearing and disappearing.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, OnceLock};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::get,
};
use builtpanel_cache::RedisHandle;
use builtpanel_database::{modules as db, rbac::assign_role};
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_system::{SystemState, client_router, router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use builtpanel_settings::SettingsRegistry;
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const BASE: &str = "/api/v1/admin/modules";

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

fn manifest(name: &str, core: bool, deps: &[&str]) -> Manifest {
    let deps = deps
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let category = if core { "core" } else { "web" };
    // Feature modules have a customer menu entry; the core ones do not.
    let ui = if core {
        String::new()
    } else {
        format!(
            "[ui]\nclient_nav = {{ group = \"Web\", order = 10, icon = \"globe\" }}\n[features]\nkeys = [\"{name}_feature\"]\n"
        )
    };
    Manifest::parse(
        name,
        &format!(
            "[module]\nname = \"{name}\"\nversion = \"1.0.0\"\ntitle = {{ en = \"{name} title\" }}\ncategory = \"{category}\"\ncore = {core}\ndepends_on = [{deps}]\n{ui}"
        ),
    )
    .unwrap()
}

/// A module with one route: `/api/v1/client/<name>` answers its own name.
fn feature(name: &'static str, deps: &[&str]) -> Arc<dyn Module> {
    Arc::new(Fake {
        manifest: manifest(name, false, deps),
        routes: Box::new(move || ModuleRoutes {
            client: Some(Router::new().route(
                &format!("/api/v1/client/{name}"),
                get(move || async move { name }),
            )),
            ..ModuleRoutes::none()
        }),
    })
}

/// These tests run one at a time. Together they stall for half a minute (sqlx's test pools share
/// one budget of connections, and the module sync here is not made for being run at once), and
/// the one that has to wait the longest then gives up with a pool time-out.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Env {
    _one: tokio::sync::MutexGuard<'static, ()>,
    app: Router,
    sessions: SessionStore,
    pool: PgPool,
    mount: ModuleMount,
}

struct Browser {
    cookie: String,
    csrf: String,
}

/// auth (core), system (core, the real routes), dns, domains -> dns, ssl -> domains.
async fn env(pool: &PgPool) -> Env {
    let one = ONE_AT_A_TIME.lock().await;
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let late: Arc<OnceLock<ModuleMount>> = Arc::default();
    let branding: Arc<builtpanel_mod_system::bootstrap::BrandingCache> = Arc::default();
    let settings = Arc::new(
        SettingsRegistry::build(vec![(
            "system".to_owned(),
            serde_json::from_str(builtpanel_mod_system::SETTINGS_SCHEMA).unwrap(),
        ), (
            "dns".to_owned(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "title": "DNS",
                "x-title": {"en": "DNS", "bn": "ডিএনএস"},
                "properties": {
                    "default_ttl": {"type": "integer", "minimum": 60, "maximum": 86400, "default": 3600, "title": "Default TTL"},
                    "provider": {"type": "string", "enum": ["internal", "external"], "default": "internal"},
                    "per_node_cache": {"type": "integer", "default": 5, "x-scope": "node"}
                }
            }),
        )])
        .unwrap(),
    );
    let system = Arc::new(Fake {
        manifest: manifest("system", true, &[]),
        routes: Box::new({
            let (pool, sessions, late) = (pool.clone(), sessions.clone(), late.clone());
            let settings = settings.clone();
            let branding = branding.clone();
            move || match late.get() {
                Some(mount) => {
                    let state = SystemState {
                        db: pool.clone(),
                        sessions: sessions.clone(),
                        mount: mount.clone(),
                        settings: settings.clone(),
                        branding: branding.clone(),
                    };
                    ModuleRoutes {
                        admin: Some(router().with_state(state.clone())),
                        client: Some(client_router().with_state(state)),
                        ..ModuleRoutes::none()
                    }
                }
                None => ModuleRoutes::none(),
            }
        }),
    });
    let registry = Registry::build(vec![
        system,
        feature("dns", &[]),
        feature("domains", &["dns"]),
        feature("ssl", &["domains"]),
    ])
    .unwrap();
    // What control-api does at startup: write the build's modules to the table.
    let values: Vec<(String, Value)> = registry
        .modules()
        .iter()
        .map(|m| {
            (
                m.manifest().module.name.clone(),
                serde_json::to_value(m.manifest()).unwrap(),
            )
        })
        .collect();
    let records: Vec<db::ModuleRecord<'_>> = registry
        .modules()
        .iter()
        .zip(&values)
        .map(|(m, (_, json))| db::ModuleRecord {
            name: &m.manifest().module.name,
            version: &m.manifest().module.version,
            category: m.manifest().module.category.as_str(),
            core: m.manifest().module.core,
            manifest: json,
        })
        .collect();
    db::sync(pool, &records).await.unwrap();

    let base = Router::new().fallback(|| async { StatusCode::NOT_FOUND });
    let mount = ModuleMount::new(registry, base);
    late.set(mount.clone()).ok();
    mount.rebuild();
    mount.apply(&db::enabled_names(pool).await.unwrap().into_iter().collect());
    Env {
        _one: one,
        app: mount.service(),
        sessions,
        pool: pool.clone(),
        mount,
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

    async fn switch(
        &self,
        who: &Browser,
        name: &str,
        enabled: bool,
        cascade: bool,
    ) -> (StatusCode, Value) {
        self.call(
            Some(who),
            "PUT",
            &format!("{BASE}/{name}"),
            Some(json!({"enabled": enabled, "cascade": cascade})),
            true,
        )
        .await
    }

    /// Status of a module's own route, through the same running application.
    async fn route(&self, name: &str) -> StatusCode {
        self.call(None, "GET", &format!("/api/v1/client/{name}"), None, false)
            .await
            .0
    }
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_list_shows_what_exists_what_is_on_and_what_depends_on_what(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;

    assert_eq!(
        e.call(None, "GET", BASE, None, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = e.call(Some(&plain), "GET", BASE, None, false).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );

    let (status, body) = e.call(Some(&root), "GET", BASE, None, false).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    let names: Vec<&str> = items.iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["dns", "domains", "ssl", "system"]);
    let dns = &items[0];
    assert_eq!(dns["title"]["en"], "dns title");
    assert_eq!(
        (
            dns["enabled"].as_bool(),
            dns["active"].as_bool(),
            dns["core"].as_bool()
        ),
        (Some(true), Some(true), Some(false))
    );
    assert_eq!(dns["dependents"], json!(["domains", "ssl"]));
    assert_eq!(items[1]["depends_on"], json!(["dns"]));
    assert_eq!(items[3]["core"], true);
    assert_eq!(dns["present"], true);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_switched_off_module_answers_404_at_once_and_comes_back_when_switched_on(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    assert_eq!(e.route("ssl").await, StatusCode::OK);

    let (status, body) = e.switch(&root, "ssl", false, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed"], json!(["ssl"]));
    assert_eq!(body["active"], json!(["dns", "domains", "system"]));
    // The same running application, no restart.
    assert_eq!(e.route("ssl").await, StatusCode::NOT_FOUND);
    assert_eq!(e.route("domains").await, StatusCode::OK);

    // Asking again changes nothing and is not an error.
    let (status, body) = e.switch(&root, "ssl", false, false).await;
    assert_eq!((status, &body["changed"]), (StatusCode::OK, &json!([])));

    let (status, body) = e.switch(&root, "ssl", true, false).await;
    assert_eq!(
        (status, &body["changed"]),
        (StatusCode::OK, &json!(["ssl"]))
    );
    assert_eq!(e.route("ssl").await, StatusCode::OK);

    let audit: Vec<(String, String, Value)> =
        sqlx::query_as("SELECT action, target_id, metadata FROM audit_logs ORDER BY at, id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(audit.len(), 2, "one entry per real change: {audit:?}");
    assert_eq!(
        (audit[0].0.as_str(), audit[0].1.as_str()),
        ("admin.module_disabled", "ssl")
    );
    assert_eq!(
        (audit[1].0.as_str(), audit[1].1.as_str()),
        ("admin.module_enabled", "ssl")
    );
    assert_eq!(audit[0].2, json!({"requested": "ssl", "cascade": false}));
    // And the matching events are in the outbox, written in the same transaction.
    let events: Vec<(String, Value)> =
        sqlx::query_as("SELECT event_type, payload FROM outbox ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        events,
        [
            (
                "module_disabled".to_owned(),
                json!({"type": "module_disabled", "name": "ssl"})
            ),
            (
                "module_enabled".to_owned(),
                json!({"type": "module_enabled", "name": "ssl"})
            ),
        ]
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn dependencies_are_respected_unless_cascade_is_asked_for(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;

    // dns is needed by domains and ssl: refused, named, nothing changed.
    let (status, body) = e.switch(&root, "dns", false, false).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "module_has_dependents")
    );
    assert!(
        body["detail"].as_str().unwrap().contains("domains, ssl"),
        "{body}"
    );
    assert_eq!(e.route("dns").await, StatusCode::OK);
    assert_eq!(db::enabled_names(&pool).await.unwrap().len(), 4);

    // With cascade all three go, each audited.
    let (status, body) = e.switch(&root, "dns", false, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed"], json!(["dns", "domains", "ssl"]));
    assert_eq!(body["active"], json!(["system"]));
    for name in ["dns", "domains", "ssl"] {
        assert_eq!(e.route(name).await, StatusCode::NOT_FOUND, "{name}");
    }
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'admin.module_disabled'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 3);

    // ssl needs domains, which needs dns: refused, then switched on together.
    let (status, body) = e.switch(&root, "ssl", true, false).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "module_needs_dependencies")
    );
    assert!(
        body["detail"].as_str().unwrap().contains("dns, domains"),
        "{body}"
    );
    assert_eq!(e.route("ssl").await, StatusCode::NOT_FOUND);
    let (status, body) = e.switch(&root, "ssl", true, true).await;
    assert_eq!(
        (status, &body["changed"]),
        (StatusCode::OK, &json!(["dns", "domains", "ssl"]))
    );
    assert_eq!(e.route("ssl").await, StatusCode::OK);
    assert_eq!(e.mount.mounted().len(), 4);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn core_modules_strangers_and_bad_requests_are_refused(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;

    let (status, body) = e.switch(&root, "system", false, true).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "module_is_core")
    );
    let (status, _) = e.switch(&root, "ghost", false, false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = e.switch(&plain, "ssl", false, false).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );
    let (status, _) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{BASE}/ssl"),
            Some(json!({"enabled": false})),
            false,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no CSRF header");
    let (status, _) = e
        .call(
            None,
            "PUT",
            &format!("{BASE}/ssl"),
            Some(json!({"enabled": false})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = e
        .call(
            Some(&root),
            "PUT",
            &format!("{BASE}/ssl"),
            Some(json!({"enabled": false, "force": true})),
            true,
        )
        .await;
    assert!(
        status.is_client_error(),
        "unknown fields are refused: {status}"
    );

    // Nothing of the above changed anything.
    assert_eq!(e.route("ssl").await, StatusCode::OK);
    assert_eq!(db::enabled_names(&e.pool).await.unwrap().len(), 4);
    let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audited, 0);
    let announced: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(announced, 0, "refused requests announce nothing");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn settings_are_listed_validated_saved_and_audited_with_a_diff(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    let path = "/api/v1/admin/settings";

    // Who may look.
    assert_eq!(
        e.call(None, "GET", path, None, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = e.call(Some(&plain), "GET", path, None, false).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );

    // The list: the schema (global settings only) and the defaults.
    let (status, body) = e.call(Some(&root), "GET", path, None, false).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["module"], "dns");
    assert_eq!(items[1]["module"], "system");
    assert_eq!(items[1]["title"]["bn"], "সাধারণ");
    assert_eq!(items[0]["title"]["bn"], "ডিএনএস");
    assert_eq!(
        items[0]["values"],
        json!({"default_ttl": 3600, "provider": "internal"})
    );
    let props = items[0]["schema"]["properties"].as_object().unwrap();
    assert_eq!(
        props.len(),
        2,
        "node-scoped settings are not shown: {props:?}"
    );
    assert_eq!(props["default_ttl"]["maximum"], 86400);

    // A wrong type is refused with a field error and changes nothing (the DoD).
    let put = |values: Value| (format!("{path}/dns"), Some(json!({"values": values})));
    let (url, body) = put(json!({"default_ttl": "an hour", "provider": "internal"}));
    let (status, body) = e.call(Some(&root), "PUT", &url, body, true).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["errors"][0]["field"], "values.default_ttl");
    assert_eq!(body["errors"][0]["code"], "invalid_value");
    let (url, b) = put(
        json!({"default_ttl": 10, "provider": "carrier pigeon", "ghost": 1, "per_node_cache": 3}),
    );
    let (status, body) = e.call(Some(&root), "PUT", &url, b, true).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
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
            ("values.default_ttl".to_owned(), "invalid_value".to_owned()),
            ("values.ghost".to_owned(), "unknown_setting".to_owned()),
            ("values.per_node_cache".to_owned(), "wrong_scope".to_owned()),
            ("values.provider".to_owned(), "invalid_value".to_owned()),
        ]
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM settings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);

    // A good change: saved, and the audit entry shows before and after.
    let (url, b) = put(json!({"default_ttl": 600}));
    let (status, body) = e.call(Some(&root), "PUT", &url, b, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed"], json!(["default_ttl"]));
    assert_eq!(
        body["values"],
        json!({"default_ttl": 600, "provider": "internal"})
    );
    let (_, one) = e
        .call(Some(&root), "GET", &format!("{path}/dns"), None, false)
        .await;
    assert_eq!(one["values"]["default_ttl"], 600);
    let audit: Vec<(String, String, Value)> =
        sqlx::query_as("SELECT action, target_id, metadata FROM audit_logs WHERE action = 'admin.settings_changed'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].1, "dns");
    assert_eq!(
        audit[0].2,
        json!({"changed": {"default_ttl": {"before": 3600, "after": 600}}}),
        "the old and the new value"
    );
    let events: Vec<Value> =
        sqlx::query_scalar("SELECT payload FROM outbox WHERE event_type = 'settings_changed'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        events,
        [json!({"type": "settings_changed", "module": "dns"})]
    );

    // Saving the same values again, or putting a value back to the default.
    let (url, b) = put(json!({"default_ttl": 600}));
    let (_, body) = e.call(Some(&root), "PUT", &url, b, true).await;
    assert_eq!(body["changed"], json!([]));
    let (url, b) = put(json!({"default_ttl": 3600}));
    let (_, body) = e.call(Some(&root), "PUT", &url, b, true).await;
    assert_eq!(body["changed"], json!(["default_ttl"]));
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'admin.settings_changed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        audit_count, 2,
        "a save that changes nothing leaves no trace"
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM settings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0, "back at the default: no row");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn changing_settings_needs_the_manage_permission_csrf_and_a_known_module(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    let body = Some(json!({"values": {"default_ttl": 120}}));
    let url = "/api/v1/admin/settings/dns";

    let (status, b) = e.call(Some(&plain), "PUT", url, body.clone(), true).await;
    assert_eq!(
        (status, code(&b)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );
    assert_eq!(
        e.call(Some(&root), "PUT", url, body.clone(), false).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        e.call(None, "PUT", url, body.clone(), true).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Unknown module, and a module that exists but has no settings.
    for module in ["ghost", "ssl"] {
        let (status, _) = e
            .call(
                Some(&root),
                "PUT",
                &format!("/api/v1/admin/settings/{module}"),
                body.clone(),
                true,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{module}");
        let (status, _) = e
            .call(
                Some(&root),
                "GET",
                &format!("/api/v1/admin/settings/{module}"),
                None,
                false,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{module}");
    }
    // Not the right shape.
    for bad in [
        json!({"default_ttl": 120}),
        json!({"values": {"default_ttl": 120}, "force": true}),
        json!({"values": [1]}),
    ] {
        let (status, _) = e.call(Some(&root), "PUT", url, Some(bad), true).await;
        assert!(status.is_client_error(), "{status}");
    }
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM settings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn bootstrap_gives_the_interface_everything_in_one_fast_request(pool: PgPool) {
    let e = env(&pool).await;
    let root_id = account(&pool, "rootuser", &["super_admin"]).await;
    let root = e.sign_in(root_id).await;
    let path = "/api/v1/admin/bootstrap";

    assert_eq!(
        e.call(None, "GET", path, None, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = e.call(Some(&root), "GET", path, None, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["username"], "rootuser");
    assert_eq!(body["user"]["kind"], "admin");
    assert_eq!(body["roles"], json!(["super_admin"]));
    assert_eq!(body["permissions"], json!(["*"]));
    assert_eq!(body["csrf_token"], root.csrf.as_str());
    assert_eq!(
        body["mfa"],
        json!({"enabled": false, "required": false, "setup_required": false})
    );
    assert_eq!(
        (body["features"].clone(), body["limits"].clone()),
        (json!([]), json!({}))
    );
    // The default language of the settings when the account has no preference of its own.
    assert_eq!(body["branding"]["panel_name"], "Built Panel");
    assert!(body["locale"].is_string());
    // The modules being served, dependencies first, with their menu data.
    let names: Vec<&str> = body["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["dns", "system", "domains", "ssl"]);
    assert_eq!(body["modules"][0]["title"]["en"], "dns title");
    assert_eq!(body["modules"][1]["name"], "system");
    assert_eq!(body["modules"][1]["core"], true);

    // Switching a module off changes the next bootstrap: the menu follows.
    let (status, _) = e.switch(&root, "ssl", false, false).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = e.call(Some(&root), "GET", path, None, false).await;
    let names: Vec<&str> = body["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["dns", "system", "domains"]);

    // It is never cached by a browser or a proxy (it holds the CSRF token).
    let req = Request::builder()
        .uri(path)
        .header(header::COOKIE, &root.cookie)
        .body(Body::empty())
        .unwrap();
    let res = e.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");

    // A customer account is not an administrator.
    let client: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'customer', 'customer@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    let c = e.sign_in(client).await;
    let (status, body) = e.call(Some(&c), "GET", path, None, false).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "not_an_administrator")
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn bootstrap_answers_in_under_fifty_milliseconds_and_follows_the_settings(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let path = "/api/v1/admin/bootstrap";
    // The first call fills the caches; the rest are what users feel.
    e.call(Some(&root), "GET", path, None, false).await;
    let mut times = Vec::new();
    for _ in 0..30 {
        let started = std::time::Instant::now();
        let (status, _) = e.call(Some(&root), "GET", path, None, false).await;
        times.push(started.elapsed());
        assert_eq!(status, StatusCode::OK);
    }
    times.sort();
    let (median, worst) = (times[times.len() / 2], times[times.len() - 1]);
    assert!(
        median < std::time::Duration::from_millis(50),
        "median {median:?}"
    );
    assert!(
        worst < std::time::Duration::from_millis(250),
        "worst {worst:?}"
    );

    // The panel's name is remembered for a few seconds, but a change made through
    // the settings API shows up in the very next bootstrap.
    let (_, body) = e.call(Some(&root), "GET", path, None, false).await;
    assert_eq!(body["branding"]["panel_name"], "Built Panel");
    let (status, saved) = e
        .call(
            Some(&root),
            "PUT",
            "/api/v1/admin/settings/system",
            Some(json!({"values": {"panel_name": "My Hosting", "default_language": "bn"}})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (_, body) = e.call(Some(&root), "GET", path, None, false).await;
    assert_eq!(body["branding"]["panel_name"], "My Hosting");
    assert_eq!(body["branding"]["default_language"], "bn");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_control_planes_own_log_is_for_administrators_with_the_permission_only(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    let url = "/api/v1/admin/system-logs";
    assert_eq!(
        e.call(None, "GET", url, None, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, b) = e.call(Some(&plain), "GET", url, None, false).await;
    assert_eq!(
        (status, code(&b)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );
    // The events of this process go to the ring when a subscriber with its layer is installed;
    // here one is installed for the test.
    use tracing_subscriber::layer::SubscriberExt as _;
    let subscriber = tracing_subscriber::registry().with(builtpanel_observability::ring::RingLayer);
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::warn!(marker = "the-log-api-test", "something worth a look");
    let (status, b) = e
        .call(
            Some(&root),
            "GET",
            &format!("{url}?level=warn&search=the-log-api-test"),
            None,
            false,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{b}");
    let items = b["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{b}");
    assert_eq!(items[0]["level"], "WARN");
    assert!(
        items[0]["text"]
            .as_str()
            .unwrap()
            .contains("something worth a look")
    );
    for q in ["level=loud", "lines=0", "lines=5000"] {
        let (status, _) = e
            .call(Some(&root), "GET", &format!("{url}?{q}"), None, false)
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{q}");
    }
}

async fn client_with_hosting(pool: &PgPool, name: &str) -> Uuid {
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
    sqlx::query(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, $4, 10000, 'active')",
    )
    .bind(account)
    .bind(node)
    .bind(plan)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
    account
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_customer_bootstrap_has_the_menu_the_plan_features_and_limits(pool: PgPool) {
    let e = env(&pool).await;
    let path = "/api/v1/client/bootstrap";
    let root_id = account(&pool, "rootuser", &["super_admin"]).await;
    let root = e.sign_in(root_id).await;
    let alice_id = client_with_hosting(&pool, "alice").await;
    let alice = e.sign_in(alice_id).await;

    assert_eq!(
        e.call(None, "GET", path, None, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Each kind has its own bootstrap.
    let (status, body) = e.call(Some(&root), "GET", path, None, false).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "not_a_customer");
    let (status, body) = e
        .call(Some(&alice), "GET", "/api/v1/admin/bootstrap", None, false)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "not_an_administrator");

    let (status, body) = e.call(Some(&alice), "GET", path, None, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["username"], "alice");
    assert_eq!(body["user"]["kind"], "client");
    assert_eq!(body["csrf_token"], alice.csrf.as_str());
    // Only what has a customer menu entry (system has none).
    let names: Vec<&str> = body["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["dns", "domains", "ssl"]);
    assert_eq!(body["modules"][1]["nav"]["group"], "Web");
    // The default plan: its feature set and its limits.
    let features: Vec<&str> = body["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    let in_plan: Vec<String> = sqlx::query_scalar(
        "SELECT unnest(f.features::text[]) FROM plans p JOIN feature_sets f ON f.id = p.feature_set_id WHERE p.is_default",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let all: bool = sqlx::query_scalar(
        "SELECT f.all_features FROM plans p JOIN feature_sets f ON f.id = p.feature_set_id WHERE p.is_default",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    if all {
        assert_eq!(
            features,
            ["dns_feature", "domains_feature", "ssl_feature"],
            "a plan with every feature gets what the served modules declare"
        );
    } else {
        assert_eq!(
            features,
            in_plan.iter().map(String::as_str).collect::<Vec<_>>()
        );
    }
    assert!(body["limits"].is_object());

    // A plan change shows in the next bootstrap.
    sqlx::query(
        "UPDATE plans SET limits = jsonb_set(limits, '{max_domains}', '7') WHERE is_default",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (_, body) = e.call(Some(&alice), "GET", path, None, false).await;
    assert_eq!(body["limits"]["max_domains"], 7);
}
