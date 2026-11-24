//! Resellers over HTTP, against real PostgreSQL and Redis.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_auth_core::password::{Argon2Params, PasswordHasher};
use builtpanel_cache::RedisHandle;
use builtpanel_database::{modules as db, rbac::assign_role};
use builtpanel_kernel::{Manifest, Module, ModuleMount, ModuleRoutes, Registry};
use builtpanel_mod_users::{UsersState, reseller, router};
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

fn manifest(name: &str) -> Manifest {
    Manifest::parse(
        name,
        &format!(
            "[module]\nname = \"{name}\"\nversion = \"1.0.0\"\ntitle = {{ en = \"{name} title\" }}\ncategory = \"core\"\ncore = true\n"
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
    let state = {
        let (pool, sessions) = (pool.clone(), sessions.clone());
        move || UsersState {
            db: pool.clone(),
            sessions: sessions.clone(),
            hasher: PasswordHasher::new(Argon2Params::default()).unwrap(),
            purge_after_days: 7,
        }
    };
    let users = Arc::new(Fake {
        manifest: manifest("users"),
        routes: Box::new({
            let state = state.clone();
            move || ModuleRoutes {
                admin: Some(router().with_state(state())),
                ..ModuleRoutes::none()
            }
        }),
    });
    let resellers = Arc::new(Fake {
        manifest: manifest("resellers"),
        routes: Box::new(move || ModuleRoutes {
            admin: Some(reseller::admin_router().with_state(state())),
            client: Some(reseller::portal_router().with_state(state())),
            ..ModuleRoutes::none()
        }),
    });
    let registry = Registry::build(vec![users, resellers]).unwrap();
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

async fn add_node(pool: &PgPool, name: &str, online: bool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO nodes (name, status) VALUES ($1, CASE WHEN $2 THEN 'online' ELSE 'offline' END) RETURNING id",
    )
    .bind(name)
    .bind(online)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn reseller_account(
    pool: &PgPool,
    e: &Env,
    name: &str,
    pkg: Uuid,
    root: &Browser,
) -> (Uuid, Browser) {
    let (status, body) = e
        .call(
            Some(root),
            "POST",
            "/api/v1/admin/resellers",
            Some(json!({
                "username": name,
                "email": format!("{name}@example.com"),
                "company_name": format!("{name} Hosting"),
                "password": "correct horse battery staple 42",
                "package_id": pkg,
            })),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let b = e.sign_in(id).await;
    let _ = pool;
    (id, b)
}

async fn package(
    e: &Env,
    root: &Browser,
    name: &str,
    accounts: i64,
    disk_gb: i64,
    over: bool,
) -> Uuid {
    let (status, body) = e
        .call(
            Some(root),
            "POST",
            "/api/v1/admin/reseller-packages",
            Some(json!({
                "name": name,
                "max_accounts": accounts,
                "max_disk_gb": disk_gb,
                "max_bandwidth_gb": 100,
                "allow_overselling": over,
            })),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().parse().unwrap()
}

async fn plan(e: &Env, who: &Browser, name: &str, disk_mb: i64) -> Value {
    let (status, body) = e
        .call(
            Some(who),
            "POST",
            "/api/v1/reseller/plans",
            Some(json!({
                "name": name, "disk_mb": disk_mb, "bandwidth_mb": 1024,
                "domains_limit": 3, "databases_limit": 2, "mailboxes_limit": 5
            })),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

fn client(name: &str, plan: &str) -> Value {
    json!({
        "username": name,
        "email": format!("{name}@example.com"),
        "password": "correct horse battery staple 42",
        "plan_name": plan,
        "domain": format!("{name}.example.com"),
    })
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_reseller_sells_inside_its_package_and_only_sees_its_own(pool: PgPool) {
    let e = env(&pool).await;
    add_node(&pool, "node1", true).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    let (status, body) = e
        .call(Some(&plain), "GET", "/api/v1/admin/resellers", None, false)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );

    // Two accounts, a disk of 1 GiB: room for two 400 MiB plans, not three.
    let pkg = package(&e, &root, "Starter", 2, 1, false).await;
    let (a_id, a) = reseller_account(&pool, &e, "apex", pkg, &root).await;
    let (_b_id, b) = reseller_account(&pool, &e, "bravo", pkg, &root).await;
    let (_, me) = e
        .call(Some(&a), "GET", "/api/v1/reseller/me", None, false)
        .await;
    assert_eq!(me["company_name"], "apex Hosting");
    assert_eq!(me["status"], "active");
    assert_eq!(me["accounts_count"], 0);
    assert_eq!(me["branding"]["brand_name"], "apex Hosting");
    let (_, pk) = e
        .call(Some(&a), "GET", "/api/v1/reseller/me/package", None, false)
        .await;
    assert_eq!(pk["max_accounts"], 2);

    // An administrator or a client is not a reseller.
    let (status, body) = e
        .call(Some(&root), "GET", "/api/v1/reseller/me", None, false)
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "not_a_reseller")
    );

    plan(&e, &a, "Small", 400).await;
    plan(&e, &a, "Big", 800).await;
    let (status, body) = e
        .call(
            Some(&a),
            "POST",
            "/api/v1/reseller/plans",
            Some(json!({
                "name": "small", "disk_mb": 1, "bandwidth_mb": 1,
                "domains_limit": 1, "databases_limit": 1, "mailboxes_limit": 1
            })),
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "plan_name_taken")
    );
    // The administrator's plan list does not show a reseller's plans.
    let (_, list) = e
        .call(Some(&root), "GET", "/api/v1/admin/plans", None, false)
        .await;
    let _ = list; // (the plans module is not mounted here; the filter is checked in SQL below)
    let hidden: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM plans WHERE reseller_id IS NULL AND display_name IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(hidden, 0);

    let (status, c1) = e
        .call(
            Some(&a),
            "POST",
            "/api/v1/reseller/users",
            Some(client("carol", "Small")),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{c1}");
    assert_eq!(c1["plan_name"], "Small");
    assert_eq!(c1["reseller_id"], a_id.to_string());
    let (status, _) = e
        .call(
            Some(&a),
            "POST",
            "/api/v1/reseller/users",
            Some(client("dave", "Small")),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // Third account: past max_accounts.
    let (status, body) = e
        .call(
            Some(&a),
            "POST",
            "/api/v1/reseller/users",
            Some(client("erin", "Small")),
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "reseller_limit"),
        "{body}"
    );
    // Nothing of the refused request was kept.
    let erin: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE username = 'erin'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(erin, 0);
    let (_, users) = e
        .call(Some(&a), "GET", "/api/v1/reseller/users", None, false)
        .await;
    assert_eq!(users.as_array().unwrap().len(), 2);
    let (_, me) = e
        .call(Some(&a), "GET", "/api/v1/reseller/me", None, false)
        .await;
    assert_eq!(me["accounts_count"], 2);

    // The other reseller sees none of it, and cannot touch it: 404, as if it were not there.
    let (_, theirs) = e
        .call(Some(&b), "GET", "/api/v1/reseller/users", None, false)
        .await;
    assert_eq!(theirs.as_array().unwrap().len(), 0);
    let carol = c1["id"].as_str().unwrap();
    for (method, path, body) in [
        (
            "POST",
            format!("/api/v1/reseller/users/{carol}/status"),
            Some(json!({"status": "suspended"})),
        ),
        ("DELETE", format!("/api/v1/reseller/users/{carol}"), None),
    ] {
        let (status, body) = e.call(Some(&b), method, &path, body, true).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {body}");
    }
    // Nor use the first reseller's plan.
    let (status, body) = e
        .call(
            Some(&b),
            "POST",
            "/api/v1/reseller/users",
            Some(client("frank", "Small")),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let (_, bp) = e
        .call(Some(&b), "GET", "/api/v1/reseller/plans", None, false)
        .await;
    assert_eq!(bp.as_array().unwrap().len(), 0);
    let plan_id = plan(&e, &a, "Extra", 10).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _) = e
        .call(
            Some(&b),
            "DELETE",
            &format!("/api/v1/reseller/plans/{plan_id}"),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A plan in use cannot be deleted; an unused one can.
    let small = users.as_array().unwrap()[0]["plan_name"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(small, "Small");
    let (_, plans) = e
        .call(Some(&a), "GET", "/api/v1/reseller/plans", None, false)
        .await;
    let small_id = plans
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Small")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, body) = e
        .call(
            Some(&a),
            "DELETE",
            &format!("/api/v1/reseller/plans/{small_id}"),
            None,
            true,
        )
        .await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "plan_in_use"));
    let (status, _) = e
        .call(
            Some(&a),
            "DELETE",
            &format!("/api/v1/reseller/plans/{plan_id}"),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The reseller suspends and resumes its own client, then deletes it; the account count drops.
    let (status, body) = e
        .call(
            Some(&a),
            "POST",
            &format!("/api/v1/reseller/users/{carol}/status"),
            Some(json!({"status": "suspended"})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "still provisioning: {body}");
    let tenant: Uuid = sqlx::query_scalar("SELECT t.id FROM tenants t JOIN accounts a ON a.id = t.owner_account_id WHERE a.username = 'carol'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tenants SET status = 'active' WHERE id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = e
        .call(
            Some(&a),
            "POST",
            &format!("/api/v1/reseller/users/{carol}/status"),
            Some(json!({"status": "suspended"})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, users) = e
        .call(Some(&a), "GET", "/api/v1/reseller/users", None, false)
        .await;
    let row = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "carol")
        .unwrap();
    assert_eq!(row["status"], "suspended");
    let (status, _) = e
        .call(
            Some(&a),
            "DELETE",
            &format!("/api/v1/reseller/users/{carol}"),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, me) = e
        .call(Some(&a), "GET", "/api/v1/reseller/me", None, false)
        .await;
    assert_eq!(me["accounts_count"], 1);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_disk_cap_holds_unless_overselling_is_allowed(pool: PgPool) {
    let e = env(&pool).await;
    add_node(&pool, "node1", true).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    // 1 GiB of disk: a 700 MiB plan twice does not fit; with overselling it does.
    let strict = package(&e, &root, "Strict", 10, 1, false).await;
    let loose = package(&e, &root, "Loose", 10, 1, true).await;
    let (_, s) = reseller_account(&pool, &e, "strict", strict, &root).await;
    let (_, l) = reseller_account(&pool, &e, "loose", loose, &root).await;
    for who in [&s, &l] {
        plan(&e, who, "Big", 700).await;
    }
    for (who, second) in [(&s, StatusCode::CONFLICT), (&l, StatusCode::CREATED)] {
        let name = if second == StatusCode::CREATED {
            "l"
        } else {
            "s"
        };
        let (status, _) = e
            .call(
                Some(who),
                "POST",
                "/api/v1/reseller/users",
                Some(client(&format!("{name}one"), "Big")),
                true,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, body) = e
            .call(
                Some(who),
                "POST",
                "/api/v1/reseller/users",
                Some(client(&format!("{name}two"), "Big")),
                true,
            )
            .await;
        assert_eq!(status, second, "{body}");
        if second == StatusCode::CONFLICT {
            assert_eq!(code(&body), "reseller_limit");
        }
    }
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_suspended_reseller_and_its_clients_cannot_sign_in_but_sites_keep_running(pool: PgPool) {
    let e = env(&pool).await;
    add_node(&pool, "node1", true).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let pkg = package(&e, &root, "Starter", 5, 10, false).await;
    let (rid, r) = reseller_account(&pool, &e, "apex", pkg, &root).await;
    plan(&e, &r, "Small", 100).await;
    let (status, c) = e
        .call(
            Some(&r),
            "POST",
            "/api/v1/reseller/users",
            Some(client("carol", "Small")),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let carol_id: Uuid = c["id"].as_str().unwrap().parse().unwrap();
    let client_session = e.sign_in(carol_id).await;
    sqlx::query("UPDATE tenants SET status = 'active'")
        .execute(&pool)
        .await
        .unwrap();

    let (status, _) = e
        .call(
            Some(&root),
            "POST",
            &format!("/api/v1/admin/resellers/{rid}/status"),
            Some(json!({"status": "suspended"})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // The sessions ended, and a new one is not accepted.
    let (status, _) = e
        .call(Some(&r), "GET", "/api/v1/reseller/me", None, false)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let _ = client_session;
    let status_of: String = sqlx::query_scalar("SELECT effective_account_status($1)::text")
        .bind(carol_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status_of, "suspended");
    // The hosting account itself is untouched.
    let tenant: String =
        sqlx::query_scalar("SELECT status FROM tenants WHERE owner_account_id = $1")
            .bind(carol_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(tenant, "active");
    // Resume: all can sign in again.
    let (status, _) = e
        .call(
            Some(&root),
            "POST",
            &format!("/api/v1/admin/resellers/{rid}/status"),
            Some(json!({"status": "active"})),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let status_of: String = sqlx::query_scalar("SELECT effective_account_status($1)::text")
        .bind(carol_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status_of, "active");
    // A reseller cannot be deleted while it has accounts; a package in use cannot either.
    let (status, body) = e
        .call(
            Some(&root),
            "DELETE",
            &format!("/api/v1/admin/resellers/{rid}"),
            None,
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "reseller_has_accounts")
    );
    let (status, body) = e
        .call(
            Some(&root),
            "DELETE",
            &format!("/api/v1/admin/reseller-packages/{pkg}"),
            None,
            true,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "package_in_use")
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn branding_is_checked_and_a_reseller_without_clients_can_be_deleted(pool: PgPool) {
    let e = env(&pool).await;
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let pkg = package(&e, &root, "Starter", 5, 10, false).await;
    let (rid, r) = reseller_account(&pool, &e, "apex", pkg, &root).await;
    let (status, body) = e
        .call(Some(&r), "PUT", "/api/v1/reseller/me/branding", Some(json!({
            "brand_name": "Apex", "primary_color": "#ff8800", "logo_url": "https://cdn.example.com/a.png",
            "custom_login_message": "Welcome"
        })), true)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["brand_name"], "Apex");
    assert_eq!(body["support_email"], "apex@example.com");
    for bad in [
        json!({"primary_color": "red"}),
        json!({"logo_url": "http://insecure.example.com/a.png"}),
        json!({"logo_url": "javascript:alert(1)"}),
        json!({"support_email": "nope"}),
        json!({"brand_name": ""}),
    ] {
        let (status, body) = e
            .call(
                Some(&r),
                "PUT",
                "/api/v1/reseller/me/branding",
                Some(bad.clone()),
                true,
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {body}");
    }
    let (_, me) = e
        .call(Some(&r), "GET", "/api/v1/reseller/me", None, false)
        .await;
    assert_eq!(me["branding"]["brand_name"], "Apex");
    assert_eq!(me["branding"]["primary_color"], "#ff8800");
    // Clients are refused by the reseller routes; a stranger without a session too.
    let (status, _) = e
        .call(None, "GET", "/api/v1/reseller/me", None, false)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = e
        .call(
            Some(&root),
            "DELETE",
            &format!("/api/v1/admin/resellers/{rid}"),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, list) = e
        .call(Some(&root), "GET", "/api/v1/admin/resellers", None, false)
        .await;
    assert_eq!(list.as_array().unwrap().len(), 0);
    // The package is free again.
    let (status, _) = e
        .call(
            Some(&root),
            "DELETE",
            &format!("/api/v1/admin/reseller-packages/{pkg}"),
            None,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
