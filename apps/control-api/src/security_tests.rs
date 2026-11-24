//! The API's security checks (T19.04): the whole application, as it is mounted, tried the way
//! an attacker would, route by route. The routes come from the OpenAPI document, so a new
//! route is checked without anyone remembering to add it here.
//!
//! * authentication bypass: nothing but the public routes answers without a session;
//! * CSRF: nothing that changes something works without the session's CSRF header;
//! * privilege: a hosting client cannot use any administrator route;
//! * IDOR: another client's ids, put into every client route, never give a success;
//! * injection and hostile input: no route answers 500 to injection strings, wrong types,
//!   NUL bytes, huge or broken bodies;
//! * rate limit: repeated bad sign-ins from one address are stopped;
//! * headers: answers carry the headers that keep a browser from guessing.
//!
//! Needs PostgreSQL and Redis (`DATABASE_URL`, `REDIS_URL`, `--include-ignored`).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_auth_core::{
    password::{Argon2Params, PasswordHasher},
    secretbox::MasterKey,
};
use builtpanel_cache::{
    RedisHandle,
    lockout::{FailureLimiter, LockoutPolicy},
};
use builtpanel_http_kit::health::Readiness;
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::{app, modules, openapi};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

/// One route of the API.
#[derive(Debug, Clone)]
struct Route {
    method: &'static str,
    path: String,
}

impl Route {
    fn mutates(&self) -> bool {
        !matches!(self.method, "GET" | "HEAD" | "OPTIONS")
    }

    fn public(&self) -> bool {
        self.path.starts_with("/api/v1/public/")
    }

    /// The path with every `{parameter}` replaced by `value`.
    fn with(&self, value: &str) -> String {
        let mut out = String::new();
        let mut rest = self.path.as_str();
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let end = rest[start..]
                .find('}')
                .map_or(rest.len(), |e| start + e + 1);
            out.push_str(value);
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    }

    fn has_parameter(&self) -> bool {
        self.path.contains('{')
    }
}

fn routes() -> Vec<Route> {
    let doc = openapi::document();
    let mut out = Vec::new();
    for (path, item) in &doc.paths.paths {
        for (method, name) in [
            (&item.get, "GET"),
            (&item.post, "POST"),
            (&item.put, "PUT"),
            (&item.patch, "PATCH"),
            (&item.delete, "DELETE"),
        ] {
            if method.is_some() {
                out.push(Route {
                    method: name,
                    path: path.clone(),
                });
            }
        }
    }
    out
}

/// The sweeps are heavy and each opens many connections: one at a time, so that they do not
/// starve the other tests of this program of connections.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Who {
    cookie: String,
    csrf: String,
}

struct App {
    _one: tokio::sync::MutexGuard<'static, ()>,
    router: Router,
    sessions: SessionStore,
    pool: PgPool,
}

async fn app(pool: &PgPool) -> App {
    let one = ONE_AT_A_TIME.lock().await;
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis.clone(), pool.clone(), SessionConfig::default());
    let hasher = PasswordHasher::new(Argon2Params {
        memory_kib: 19 * 1024,
        iterations: 2,
        parallelism: 1,
    })
    .unwrap();
    let master_key = MasterKey::from_bytes(&[42; 32]);
    let ns = Uuid::now_v7().simple().to_string();
    let policy = |threshold| LockoutPolicy {
        threshold,
        window: Duration::from_secs(900),
        lockout: Duration::from_secs(900),
    };
    let auth = builtpanel_mod_auth::AuthState {
        db: pool.clone(),
        sessions: sessions.clone(),
        hasher: hasher.clone(),
        limits: builtpanel_mod_auth::LoginLimits {
            account: FailureLimiter::new(redis.clone(), format!("test:{ns}:account"), policy(5)),
            ip: FailureLimiter::new(redis.clone(), format!("test:{ns}:ip"), policy(10)),
        },
        trusted_proxies: Vec::new().into(),
        redis: redis.clone(),
        master_key: master_key.clone(),
        totp_issuer: "Built Panel Test".into(),
        root_login: None,
        audit: builtpanel_audit::AuditWriter::new(pool.clone()),
    };
    let admin = builtpanel_mod_admin::AdminState {
        db: pool.clone(),
        sessions: sessions.clone(),
        hasher,
    };
    let nodes = builtpanel_mod_nodes::NodesState {
        db: pool.clone(),
        sessions: sessions.clone(),
        public_url: None,
        stream_url: None,
        token_ttl: Duration::from_secs(900),
        ca: None,
        limiter: builtpanel_mod_nodes::enrollment_limiter(redis.clone()),
        gateway: None,
        operations: builtpanel_operations::Operations::new(pool.clone()),
        audit: builtpanel_audit::AuditWriter::new(pool.clone()),
    };
    let license = Arc::new(
        builtpanel_mod_license::service::LicenseService::new(
            pool.clone(),
            &builtpanel_config::LicenseConfig::default(),
        )
        .unwrap(),
    );
    let base = app::base_router(app::AppState {
        metrics: Arc::new(String::new),
        readiness: Readiness::new(Duration::from_secs(1)),
        trusted_proxies: Vec::new().into(),
        fleet: None,
    });
    let (mount, _settings) = modules::mount(
        modules::States {
            auth,
            admin,
            nodes,
            spool_dir: std::env::temp_dir().join("bp-sec-spool"),
            license,
        },
        base,
    )
    .unwrap();
    let registry = modules::registry().unwrap();
    modules::sync(pool, &registry).await.unwrap();
    modules::sync_permissions(pool, &registry).await.unwrap();
    mount.apply(
        &builtpanel_database::modules::enabled_names(pool)
            .await
            .unwrap()
            .into_iter()
            .collect(),
    );
    let router = app::router(
        mount.service().layer(axum::middleware::from_fn_with_state(
            builtpanel_mod_license::Restrictor::new(pool.clone()),
            builtpanel_mod_license::restrict,
        )),
        Vec::new().into(),
    );
    // A real server tells the handlers who is on the other end; a test must too.
    let router = router.layer(axum::extract::connect_info::MockConnectInfo(
        std::net::SocketAddr::from(([198, 51, 100, 7], 40_000)),
    ));
    App {
        _one: one,
        router,
        sessions,
        pool: pool.clone(),
    }
}

impl App {
    async fn account(&self, name: &str, kind: &str, role: &str) -> Uuid {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (kind, username, email, password_hash)
             VALUES ($1::account_kind, $2, $2 || '@example.com', $3) RETURNING id",
        )
        .bind(kind)
        .bind(name)
        .bind(HASH)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        builtpanel_database::rbac::assign_role(&self.pool, id, role, None)
            .await
            .unwrap();
        id
    }

    /// A hosting client with an account of their own (and a node to put it on).
    async fn client(&self, name: &str, uid: i32) -> (Uuid, Uuid) {
        let account = self.account(name, "client", "client").await;
        let node: Uuid = sqlx::query_scalar(
            "INSERT INTO nodes (name) VALUES ('node1') ON CONFLICT (name) DO UPDATE SET name = 'node1' RETURNING id",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap();
        let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
            .fetch_one(&self.pool)
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
        .fetch_one(&self.pool)
        .await
        .unwrap();
        (account, tenant)
    }

    async fn sign_in(&self, account: Uuid) -> Who {
        let (token, data) = self
            .sessions
            .create(NewSession {
                account_id: account,
                ip: None,
                user_agent: None,
            })
            .await
            .unwrap();
        Who {
            cookie: set_cookie(&token).split(';').next().unwrap().to_owned(),
            csrf: data.csrf_token.expose().to_owned(),
        }
    }

    async fn send(
        &self,
        who: Option<&Who>,
        csrf: bool,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(w) = who {
            req = req.header(header::COOKIE, &w.cookie);
            if csrf {
                req = req.header(CSRF_HEADER, &w.csrf);
            }
        }
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_owned()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = tokio::time::timeout(Duration::from_secs(20), self.router.clone().oneshot(req))
            .await
            .unwrap_or_else(|_| panic!("{method} {uri} took more than 20 s"))
            .unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        (
            status,
            headers,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        )
    }
}

impl App {
    /// Like [`App::send`], but gives up after `limit`: a route that is waiting for a node that
    /// is not there has granted nothing yet.
    async fn send_within(
        &self,
        limit: Duration,
        who: &Who,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> Option<StatusCode> {
        tokio::time::timeout(limit, self.send(Some(who), true, method, uri, body))
            .await
            .ok()
            .map(|(status, _, _)| status)
    }
}

const SOME_ID: &str = "00000000-0000-4000-8000-000000000001";

/// Runs `check` over the routes, a few at a time, and lists what failed.
async fn sweep<F, Fut>(routes: Vec<Route>, check: F) -> Vec<String>
where
    F: Fn(Route) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = Option<String>> + Send + 'static,
{
    let mut failures = Vec::new();
    let mut set = tokio::task::JoinSet::new();
    let mut queue = routes.into_iter();
    loop {
        while set.len() < 4 {
            let Some(route) = queue.next() else { break };
            let check = check.clone();
            set.spawn(async move { check(route).await });
        }
        match set.join_next().await {
            Some(done) => failures.extend(done.unwrap()),
            None => break,
        }
    }
    failures.sort();
    failures
}

async fn nothing_but_the_public_routes_answers_without_a_session(pool: PgPool, _n: u32) {
    let a = Arc::new(app(&pool).await);
    let all = routes();
    assert!(all.len() > 200, "{} routes found", all.len());
    let failures = sweep(all.into_iter().filter(|r| !r.public()).collect(), {
        let a = a.clone();
        move |r| {
            let a = a.clone();
            async move {
                let body = r.mutates().then_some("{}");
                let (status, _, _) = a.send(None, false, r.method, &r.with(SOME_ID), body).await;
                (status != StatusCode::UNAUTHORIZED).then(|| {
                    format!(
                        "{} {} answered {status} without a session",
                        r.method, r.path
                    )
                })
            }
        }
    })
    .await;
    assert!(
        failures.is_empty(),
        "authentication bypass:\n{}",
        failures.join("\n")
    );
}

async fn nothing_that_changes_something_works_without_the_sessions_csrf_header(
    pool: PgPool,
    n: u32,
) {
    let a = Arc::new(app(&pool).await);
    let root = Arc::new(
        a.sign_in(
            a.account(&format!("rootuser{n}"), "admin", "super_admin")
                .await,
        )
        .await,
    );
    let (client_account, _) = a.client(&format!("alice{n}"), 10_000 + n as i32 * 10).await;
    let alice = Arc::new(a.sign_in(client_account).await);
    let mut failures = Vec::new();
    for (who, label) in [(root, "an administrator"), (alice, "a client")] {
        failures.extend(
            sweep(routes().into_iter().filter(|r| r.mutates() && !r.public()).collect(), {
                let (a, who) = (a.clone(), who.clone());
                move |r| {
                    let (a, who) = (a.clone(), who.clone());
                    async move {
                        // The session cookie only: no X-CSRF-Token.
                        let (status, _, body) = a.send(Some(&who), false, r.method, &r.with(SOME_ID), Some("{}")).await;
                        let refused = status == StatusCode::FORBIDDEN && body["code"] == "csrf_failed";
                        (!refused).then(|| format!("{} {} answered {status} ({}) to {label} without the CSRF header", r.method, r.path, body["code"]))
                    }
                }
            })
            .await,
        );
    }
    assert!(failures.is_empty(), "CSRF:\n{}", failures.join("\n"));
}

async fn a_client_cannot_use_any_administrator_route(pool: PgPool, n: u32) {
    let a = Arc::new(app(&pool).await);
    let (client_account, _) = a.client(&format!("alice{n}"), 10_000 + n as i32 * 10).await;
    let alice = Arc::new(a.sign_in(client_account).await);
    let failures = sweep(
        routes()
            .into_iter()
            .filter(|r| r.path.starts_with("/api/v1/admin/"))
            .collect(),
        {
            let (a, alice) = (a.clone(), alice.clone());
            move |r| {
                let (a, alice) = (a.clone(), alice.clone());
                async move {
                    // With a valid CSRF header, so that only the permission is in question.
                    let body = r.mutates().then_some("{}");
                    let (status, _, _) = a
                        .send(Some(&alice), true, r.method, &r.with(SOME_ID), body)
                        .await;
                    (status != StatusCode::FORBIDDEN)
                        .then(|| format!("{} {} answered {status} to a client", r.method, r.path))
                }
            }
        },
    )
    .await;
    assert!(
        failures.is_empty(),
        "privilege escalation:\n{}",
        failures.join("\n")
    );
}

/// Ids of everything the first client owns, from every table that says who owns a row.
async fn ids_of(pool: &PgPool, tenant: Uuid) -> Vec<String> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.columns
         WHERE table_schema = 'public' AND column_name = 'tenant_id'
           AND table_name IN (SELECT table_name FROM information_schema.columns WHERE table_schema = 'public' AND column_name = 'id' AND data_type = 'uuid')",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut ids = Vec::new();
    for table in tables {
        let found: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT id FROM \"{table}\" WHERE tenant_id = $1 LIMIT 5"
        )))
        .bind(tenant)
        .fetch_all(pool)
        .await
        .unwrap();
        ids.extend(found.into_iter().map(|i| i.to_string()));
    }
    ids
}

/// Gives the first client one of what the panel has many of, so that there is something to
/// reach for.
async fn furnish(a: &App, tenant: Uuid) {
    let tag = &tenant.simple().to_string()[20..];
    let domain: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, $2, 'primary') RETURNING id",
    )
    .bind(tenant)
    .bind(format!("{tag}.example"))
    .fetch_one(&a.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, $2, 'addon')")
        .bind(tenant)
        .bind(format!("shop.{tag}.example"))
        .execute(&a.pool)
        .await
        .unwrap();
    let database: Uuid = sqlx::query_scalar(
        "INSERT INTO databases (tenant_id, name, status) VALUES ($1, $2, 'ready') RETURNING id",
    )
    .bind(tenant)
    .bind(format!("db_{tag}"))
    .fetch_one(&a.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO db_users (tenant_id, name, status) VALUES ($1, $2, 'ready')")
        .bind(tenant)
        .bind(format!("u_{tag}"))
        .execute(&a.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO apps (tenant_id, domain_id, path, kind, name, database_id, status) VALUES ($1, $2, '/', 'wordpress', 'Shop', $3, 'active')")
        .bind(tenant)
        .bind(domain)
        .bind(database)
        .execute(&a.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cache_instances (tenant_id, kind, memory_mb, status) VALUES ($1, 'redis', 32, 'ready')").bind(tenant).execute(&a.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO malware_scans (tenant_id, source, state) VALUES ($1, 'client', 'done')",
    )
    .bind(tenant)
    .execute(&a.pool)
    .await
    .unwrap();
}

async fn another_clients_ids_never_give_a_client_anything(pool: PgPool, n: u32) {
    let a = Arc::new(app(&pool).await);
    let (alice_account, alice_tenant) =
        a.client(&format!("alice{n}"), 10_000 + n as i32 * 10).await;
    let (bob_account, _) = a.client(&format!("bob{n}"), 10_001 + n as i32 * 10).await;
    furnish(&a, alice_tenant).await;
    let mut ids = ids_of(&pool, alice_tenant).await;
    assert!(ids.len() >= 6, "a few things to reach for: {ids:?}");
    // One of each is enough (the routes tell kinds apart by what they look up).
    ids.dedup();
    ids.truncate(8);
    let ids = Arc::new(ids);
    let bob = Arc::new(a.sign_in(bob_account).await);
    // Alice can reach her own things (the sweep is looking at real routes, not at nothing).
    let alice = a.sign_in(alice_account).await;
    assert_eq!(
        a.send(Some(&alice), true, "GET", "/api/v1/client/apps", None)
            .await
            .0,
        StatusCode::OK
    );
    let client_routes: Vec<Route> = routes()
        .into_iter()
        .filter(|r| r.path.starts_with("/api/v1/client/") && r.has_parameter())
        .collect();
    assert!(
        client_routes.len() > 20,
        "{} client routes with a parameter",
        client_routes.len()
    );
    let failures = sweep(client_routes, {
        let (a, bob, ids) = (a.clone(), bob.clone(), ids.clone());
        move |r| {
            let (a, bob, ids) = (a.clone(), bob.clone(), ids.clone());
            async move {
                let mut bad = Vec::new();
                for id in ids.iter() {
                    let body = r.mutates().then_some("{}");
                    let Some(status) = a
                        .send_within(Duration::from_secs(3), &bob, r.method, &r.with(id), body)
                        .await
                    else {
                        continue;
                    };
                    // 202 is "queued, the node decides" (a container or file named by a string the
                    // node checks against the account); a real answer or a failure is not allowed.
                    if matches!(
                        status,
                        StatusCode::OK | StatusCode::CREATED | StatusCode::NO_CONTENT
                    ) || status.is_server_error()
                    {
                        bad.push(format!(
                            "{} {} with alice's id {id} answered {status} to bob",
                            r.method, r.path
                        ));
                    }
                }
                (!bad.is_empty()).then(|| bad.join("\n"))
            }
        }
    })
    .await;
    assert!(failures.is_empty(), "IDOR:\n{}", failures.join("\n"));
    // And nothing of alice's changed.
    let left: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM domains WHERE tenant_id = $1) + (SELECT count(*) FROM databases WHERE tenant_id = $1) + (SELECT count(*) FROM apps WHERE tenant_id = $1) + (SELECT count(*) FROM cache_instances WHERE tenant_id = $1)")
        .bind(alice_tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 2 + 1 + 1 + 1);
}

/// Strings an injection would try, in a path, a query or a body.
const HOSTILE: [&str; 5] = [
    "' OR '1'='1' --",
    "1; DROP TABLE accounts; --",
    "../../etc/passwd",
    "a\u{0}b",
    "%00%2e%2e%2f",
];

async fn no_route_answers_500_to_hostile_input(pool: PgPool, n: u32) {
    let a = Arc::new(app(&pool).await);
    let root = Arc::new(
        a.sign_in(
            a.account(&format!("rootuser{n}"), "admin", "super_admin")
                .await,
        )
        .await,
    );
    let (client_account, tenant) = a.client(&format!("alice{n}"), 10_000 + n as i32 * 10).await;
    furnish(&a, tenant).await;
    let alice = Arc::new(a.sign_in(client_account).await);
    let mut failures = Vec::new();
    for (who, label) in [(root, "an administrator"), (alice, "a client")] {
        failures.extend(
            // (The public routes too: the sign-in, the enrolment, the health checks.)
            sweep(routes(), {
                let (a, who) = (a.clone(), who.clone());
                move |r| {
                    let (a, who) = (a.clone(), who.clone());
                    async move {
                        let mut bad = Vec::new();
                        for payload in HOSTILE {
                            let encoded: String = payload.bytes().map(|b| format!("%{b:02X}")).collect();
                            let uri = format!("{}?q={encoded}&limit={encoded}&since={encoded}&domain_id={encoded}", r.with(&encoded));
                            let body = r.mutates().then(|| {
                                json!({"name": payload, "id": payload, "email": payload, "path": payload, "kind": payload,
                                       "domain_id": payload, "title": payload, "slug": payload, "folder": payload, "pattern": payload,
                                       "note": payload, "password": payload, "fqdn": payload, "tags": [payload], "items": [payload]})
                                .to_string()
                            });
                            let (status, _, _) = a.send(Some(&who), true, r.method, &uri, body.as_deref()).await;
                            // 501-504: an unavailable authority or node is not the panel failing.
                            if status == StatusCode::INTERNAL_SERVER_ERROR {
                                bad.push(format!("{} {} answered 500 to {label} for {payload:?}", r.method, r.path));
                            }
                        }
                        // A body that is not JSON, one that is the wrong type, one that is too big.
                        if r.mutates() {
                            for body in ["{", "[1,2,3]", "null", "\"text\"", &format!("{{\"name\":\"{}\"}}", "x".repeat(70_000))] {
                                let (status, _, _) = a.send(Some(&who), true, r.method, &r.with(SOME_ID), Some(body)).await;
                                if status == StatusCode::INTERNAL_SERVER_ERROR {
                                    bad.push(format!("{} {} answered 500 to {label} for a bad body {:?}", r.method, r.path, &body[..body.len().min(20)]));
                                }
                            }
                        }
                        (!bad.is_empty()).then(|| bad.join("\n"))
                    }
                }
            })
            .await,
        );
    }
    assert!(
        failures.is_empty(),
        "hostile input:\n{}",
        failures.join("\n")
    );
    // The tables are all still there.
    let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(accounts >= 2, "accounts table intact: {accounts}");
}

async fn repeated_bad_sign_ins_are_stopped_and_every_answer_carries_the_security_headers(
    pool: PgPool,
    n: u32,
) {
    let a = app(&pool).await;
    let victim = a.account(&format!("victim{n}"), "client", "client").await;
    let real = PasswordHasher::new(Argon2Params {
        memory_kib: 19 * 1024,
        iterations: 2,
        parallelism: 1,
    })
    .unwrap()
    .hash(&SecretString::from("the-real-password-1234".to_owned()))
    .unwrap();
    sqlx::query("UPDATE accounts SET password_hash = $1 WHERE id = $2")
        .bind(real)
        .bind(victim)
        .execute(&pool)
        .await
        .unwrap();
    let login = |password: &'static str| {
        let a = &a;
        async move {
            a.send(
                None,
                false,
                "POST",
                "/api/v1/public/auth/login",
                Some(&json!({"login": &format!("victim{n}"), "password": password}).to_string()),
            )
            .await
        }
    };
    let mut statuses = Vec::new();
    for _ in 0..8 {
        let (status, headers, body) = login("not-the-password-at-all").await;
        assert!(
            status != StatusCode::INTERNAL_SERVER_ERROR,
            "a wrong password answered 500: {body} {headers:?}"
        );
        statuses.push(status);
    }
    assert_eq!(statuses[0], StatusCode::UNAUTHORIZED);
    assert!(
        statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
        "{statuses:?}"
    );
    // Once locked, even the right password is refused, and says when to try again.
    let (status, headers, _) = login("whatever-it-is-now").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(headers.contains_key(header::RETRY_AFTER));
    // Headers, on an answer that worked, one that refused and one for a path that is not there.
    for (method, uri, expect) in [
        ("GET", "/api/v1/public/health", StatusCode::OK),
        ("GET", "/api/v1/auth/me", StatusCode::UNAUTHORIZED),
        ("GET", "/api/v1/nowhere", StatusCode::NOT_FOUND),
    ] {
        let (status, headers, _) = a.send(None, false, method, uri, None).await;
        assert_eq!(status, expect, "{uri}");
        let h = |n: &str| {
            headers
                .get(n)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned()
        };
        assert_eq!(h("x-content-type-options"), "nosniff", "{uri}");
        assert_eq!(h("x-frame-options"), "DENY", "{uri}");
        assert_eq!(h("referrer-policy"), "no-referrer", "{uri}");
        assert_eq!(h("cache-control"), "no-store", "{uri}");
        assert!(
            h("content-security-policy").contains("default-src 'none'"),
            "{uri}"
        );
    }
    // A session cookie is HttpOnly, Secure and SameSite=Strict, and bound to this origin.
    let someone = a.account(&format!("someone{n}"), "client", "client").await;
    let (token, _) = a
        .sessions
        .create(NewSession {
            account_id: someone,
            ip: None,
            user_agent: None,
        })
        .await
        .unwrap();
    let cookie = set_cookie(&token);
    for flag in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
        assert!(cookie.contains(flag), "{flag} in {cookie}");
    }
    assert!(
        cookie.starts_with("__Host-"),
        "the host prefix pins the cookie to this origin: {cookie}"
    );
}

/// The six checks share one database (and one pool): each gets its own names, so they can run
/// one after another without a test pool each.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_api_resists_the_usual_attacks(pool: PgPool) {
    nothing_but_the_public_routes_answers_without_a_session(pool.clone(), 0).await;
    nothing_that_changes_something_works_without_the_sessions_csrf_header(pool.clone(), 1).await;
    a_client_cannot_use_any_administrator_route(pool.clone(), 2).await;
    another_clients_ids_never_give_a_client_anything(pool.clone(), 3).await;
    no_route_answers_500_to_hostile_input(pool.clone(), 4).await;
    repeated_bad_sign_ins_are_stopped_and_every_answer_carries_the_security_headers(
        pool.clone(),
        5,
    )
    .await;
}
