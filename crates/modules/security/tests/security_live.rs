//! The firewall policy API over HTTP against real PostgreSQL and Redis (T11.02). The node is played by
//! the test. Ignored by default; run with DATABASE_URL, REDIS_URL and `--include-ignored`.
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
use builtpanel_mod_security::{SecurityState, admin_router, client_router};
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
        "security",
        "[module]\nname = \"logs\"\nversion = \"1.0.0\"\ntitle = { en = \"Security\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let (pool, sessions) = (pool.clone(), sessions.clone());
            move || ModuleRoutes {
                admin: Some(admin_router().with_state(SecurityState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                })),
                client: Some(client_router().with_state(SecurityState {
                    db: pool.clone(),
                    sessions: sessions.clone(),
                })),
                ..ModuleRoutes::none()
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
    sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, $2, 'primary') RETURNING id",
    )
    .bind(tenant)
    .bind(fqdn)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_most_specific_policy_decides_and_clients_are_kept_to_their_own_domains(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, alice) = client(&pool, "alice", 10_000).await;
    let (bob_account, bob) = client(&pool, "bob", 10_001).await;
    let (a_dom, b_dom) = (
        domain(&pool, alice, "alice.example.com").await,
        domain(&pool, bob, "bob.example.com").await,
    );
    let a = e.sign_in(alice_account).await;
    let b = e.sign_in(bob_account).await;
    let admin_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(&pool, admin_id, "super_admin", None)
        .await
        .unwrap();
    let root = e.sign_in(admin_id).await;
    let plain_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'plain', 'plain@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    let plain = e.sign_in(plain_id).await;
    let synced = |v: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query("UPDATE tenants SET routes_synced_version = $1")
                .bind(v)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    let version_of = |t: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT routes_synced_version FROM tenants WHERE id = $1")
                .bind(t)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let global = "/api/v1/admin/waf/policy";

    // Nothing set: the default, detect.
    let (status, st) = e.call(&root, "GET", global, None).await;
    assert_eq!(status, StatusCode::OK, "{st}");
    assert_eq!(
        (
            st["effective"]["mode"].as_str(),
            st["effective"]["mode_from"].as_str()
        ),
        (Some("detect"), Some("default"))
    );
    // Who may do what.
    assert_eq!(
        e.call(&plain, "GET", global, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        e.call(&a, "GET", global, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        e.call(&root, "GET", "/api/v1/client/waf/domains", None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // Global: block, one rule off. Every account's routes are sent again.
    synced(7).await;
    let (status, st) = e
        .call(
            &root,
            "PUT",
            global,
            Some(json!({"mode": "block", "disabled_rules": [942100]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{st}");
    assert_eq!(st["policy"]["mode"], "block");
    assert_eq!((version_of(alice).await, version_of(bob).await), (0, 0));
    let (_, list) = e.call(&a, "GET", "/api/v1/client/waf/domains", None).await;
    assert_eq!(
        list["items"].as_array().unwrap().len(),
        1,
        "only her own domain: {list}"
    );
    assert_eq!(
        (
            list["items"][0]["effective"]["mode"].as_str(),
            list["items"][0]["effective"]["mode_from"].as_str()
        ),
        (Some("block"), Some("global"))
    );
    assert!(list["items"][0]["policy"].is_null());

    // The account's policy overrides the panel's; only her routes are sent again.
    synced(7).await;
    let (status, _) = e
        .call(
            &root,
            "PUT",
            &format!("/api/v1/admin/waf/tenants/{alice}"),
            Some(json!({"mode": "detect", "disabled_rules": [941100]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((version_of(alice).await, version_of(bob).await), (0, 7));
    let (_, d) = e
        .call(
            &root,
            "GET",
            &format!("/api/v1/admin/waf/domains/{a_dom}"),
            None,
        )
        .await;
    assert_eq!(
        (
            d["effective"]["mode"].as_str(),
            d["effective"]["mode_from"].as_str()
        ),
        (Some("detect"), Some("tenant"))
    );
    assert_eq!(d["effective"]["disabled_rules"], json!([941100, 942100]));
    // Bob keeps the panel's.
    let (_, d) = e
        .call(
            &root,
            "GET",
            &format!("/api/v1/admin/waf/domains/{b_dom}"),
            None,
        )
        .await;
    assert_eq!(d["effective"]["mode_from"], "global");

    // The domain's policy overrides both: an administrator may switch it off, a client may not.
    let own = format!("/api/v1/client/waf/domains/{a_dom}");
    let (status, got) = e.call(&a, "PUT", &own, Some(json!({"mode": "off"}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{got}");
    let (status, st) = e
        .call(
            &a,
            "PUT",
            &own,
            Some(json!({"mode": "block", "disabled_rules": [920350]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{st}");
    assert_eq!(
        (
            st["effective"]["mode"].as_str(),
            st["effective"]["mode_from"].as_str()
        ),
        (Some("block"), Some("domain"))
    );
    assert_eq!(
        st["effective"]["disabled_rules"],
        json!([920350, 941100, 942100])
    );
    let (status, st) = e
        .call(
            &root,
            "PUT",
            &format!("/api/v1/admin/waf/domains/{a_dom}"),
            Some(json!({"mode": "off"})),
        )
        .await;
    assert_eq!(
        (status, st["effective"]["mode"].as_str()),
        (StatusCode::OK, Some("off")),
        "{st}"
    );
    // Bad input.
    for bad in [
        json!({"mode": "maybe"}),
        json!({"mode": "block", "disabled_rules": [0]}),
        json!({"mode": "block", "disabled_rules": [1000000]}),
        json!({"mode": "block", "extra": 1}),
    ] {
        let (status, got) = e.call(&a, "PUT", &own, Some(bad.clone())).await;
        assert!(
            status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
            "{bad}: {status} {got}"
        );
    }
    let many: Vec<i32> = (1..=51).collect();
    assert_eq!(
        e.call(
            &a,
            "PUT",
            &own,
            Some(json!({"mode": "block", "disabled_rules": many}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Another account's domain, and one that does not exist: not found.
    for id in [b_dom, Uuid::new_v4()] {
        let path = format!("/api/v1/client/waf/domains/{id}");
        assert_eq!(
            e.call(&a, "PUT", &path, Some(json!({"mode": "block"})))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            e.call(&a, "DELETE", &path, None).await.0,
            StatusCode::NOT_FOUND
        );
    }
    // Taking the domain's policy away brings the account's back; taking all away, the default.
    assert_eq!(
        e.call(&a, "DELETE", &own, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        e.call(&a, "DELETE", &own, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(
            &root,
            "DELETE",
            &format!("/api/v1/admin/waf/tenants/{alice}"),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        e.call(&root, "DELETE", global, None).await.0,
        StatusCode::NO_CONTENT
    );
    let (_, d) = e
        .call(
            &root,
            "GET",
            &format!("/api/v1/admin/waf/domains/{a_dom}"),
            None,
        )
        .await;
    assert_eq!(
        (
            d["effective"]["mode"].as_str(),
            d["effective"]["mode_from"].as_str()
        ),
        (Some("detect"), Some("default"))
    );
    // Everything that changed is in the audit log.
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action IN ('waf_policy.set', 'waf_policy.removed')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 7);

    // Turning addresses away from a domain: only one's own, only canonical ones, sent to the node.
    let blocks = format!("/api/v1/client/waf/domains/{a_dom}/ip-blocks");
    assert_eq!(
        e.call(&root, "GET", &blocks, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        e.call(&b, "GET", &blocks, None).await.0,
        StatusCode::NOT_FOUND,
        "not Bob's domain"
    );
    let (status, listed) = e.call(&a, "GET", &blocks, None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["items"].as_array().unwrap().len(), 0);
    assert_eq!(listed["limit"], 100);
    synced(7).await;
    let (status, got) = e
        .call(
            &a,
            "POST",
            &blocks,
            Some(json!({"cidr": "203.0.113.77/24", "note": "scraper"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{got}");
    assert_eq!(
        got["cidr"], "203.0.113.0/24",
        "the network is made canonical"
    );
    assert_eq!(version_of(alice).await, 0, "her routes are sent again");
    assert_eq!(
        e.call(&a, "POST", &blocks, Some(json!({"cidr": "203.0.113.0/24"})))
            .await
            .0,
        StatusCode::CONFLICT
    );
    for bad in [
        "nonsense",
        "127.0.0.1",
        "0.0.0.0/0",
        "10.0.0.0/7",
        "1.2.3.4/40",
    ] {
        let (status, got) = e
            .call(&a, "POST", &blocks, Some(json!({"cidr": bad})))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {got}");
    }
    assert_eq!(
        e.call(&b, "POST", &blocks, Some(json!({"cidr": "198.51.100.9"})))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // The list the node is sent: what the dispatcher reads for this domain.
    let by = builtpanel_database::ip_blocks::for_domains(&pool, alice, &[a_dom])
        .await
        .unwrap();
    assert_eq!(by[&a_dom], ["203.0.113.0/24"]);
    // The limit.
    for i in 1..100 {
        let (status, got) = e
            .call(
                &a,
                "POST",
                &blocks,
                Some(json!({"cidr": format!("198.51.{}.{}", i / 250, i % 250 + 1)})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{i}: {got}");
    }
    let (status, got) = e
        .call(&a, "POST", &blocks, Some(json!({"cidr": "192.0.2.1"})))
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("ip_block_limit"))
    );
    let id = got_id(&e, &a, &blocks).await;
    assert_eq!(
        e.call(&b, "DELETE", &format!("{blocks}/{id}"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(&a, "DELETE", &format!("{blocks}/{id}"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        e.call(&a, "DELETE", &format!("{blocks}/{id}"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action IN ('ip_block.added', 'ip_block.removed')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 101);
    let _ = b;
}

/// The id of the first entry of a domain's list.
async fn got_id(e: &Env, who: &Browser, blocks: &str) -> String {
    let (_, list) = e.call(who, "GET", blocks, None).await;
    list["items"][0]["id"].as_str().unwrap().to_owned()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn administrators_keep_the_list_of_refused_user_agents(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, _) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice_account).await;
    let admin_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(&pool, admin_id, "super_admin", None)
        .await
        .unwrap();
    let root = e.sign_in(admin_id).await;
    let path = "/api/v1/admin/security/user-agents";
    assert_eq!(e.call(&a, "GET", path, None).await.0, StatusCode::FORBIDDEN);
    let (status, made) = e
        .call(
            &root,
            "POST",
            path,
            Some(json!({"pattern": "  BadBot ", "note": "scrapes"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(
        (made["pattern"].as_str(), made["note"].as_str()),
        (Some("BadBot"), Some("scrapes"))
    );
    let id = made["id"].as_str().unwrap().to_owned();
    // The same text in another case is the same text.
    let (status, got) = e
        .call(&root, "POST", path, Some(json!({"pattern": "badbot"})))
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("already_listed"))
    );
    for bad in [
        json!({"pattern": ""}),
        json!({"pattern": "x".repeat(101)}),
        json!({"pattern": "a\tb"}),
        json!({"pattern": "ok", "note": "n".repeat(201)}),
    ] {
        let (status, _) = e.call(&root, "POST", path, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let (_, list) = e.call(&root, "GET", path, None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    // The list is full at 500.
    sqlx::query("INSERT INTO blocked_user_agents (pattern) SELECT 'filler-' || g FROM generate_series(1, 499) g")
        .execute(&pool)
        .await
        .unwrap();
    let (status, got) = e
        .call(
            &root,
            "POST",
            path,
            Some(json!({"pattern": "one-too-many"})),
        )
        .await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("list_full"))
    );
    assert_eq!(
        e.call(&root, "DELETE", &format!("{path}/{id}"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        e.call(&root, "DELETE", &format!("{path}/{id}"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let audited: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_logs WHERE action LIKE 'user_agent.%' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(audited, ["user_agent.blocked", "user_agent.unblocked"]);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn administrators_set_a_nodes_firewall_and_a_bad_policy_is_refused(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, _) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice_account).await;
    let admin_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(&pool, admin_id, "super_admin", None)
        .await
        .unwrap();
    let root = e.sign_in(admin_id).await;
    let node: Uuid = sqlx::query_scalar("INSERT INTO nodes (name) VALUES ('fw-node') RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let path = format!("/api/v1/admin/nodes/{node}/firewall");
    assert_eq!(
        e.call(&a, "GET", &path, None).await.0,
        StatusCode::FORBIDDEN
    );
    // None yet.
    let (status, got) = e.call(&root, "GET", &path, None).await;
    assert_eq!(
        (status, got["policy"].is_null()),
        (StatusCode::OK, true),
        "{got}"
    );
    // Set: stored and read back as written.
    let policy = json!({
        "default_deny": true,
        "always_allow": ["203.0.113.250"],
        "blocked": ["192.0.2.0/24"],
        "open_ports": [{"port": 80, "proto": "tcp"}, {"port": 21100, "to": 21119, "proto": "tcp"}],
        "rate_limits": [{"port": 80, "proto": "tcp", "per_minute": 120}]
    });
    let (status, put) = e.call(&root, "PUT", &path, Some(policy)).await;
    assert_eq!(status, StatusCode::OK, "{put}");
    assert_eq!(put["policy"]["blocked"][0], "192.0.2.0/24", "{put}");
    let (_, got) = e.call(&root, "GET", &path, None).await;
    assert_eq!(
        got["policy"]["open_ports"].as_array().unwrap().len(),
        2,
        "{got}"
    );
    // Not valid: a deny-everything policy that keeps nobody in, a wrong address, an unknown field.
    for bad in [
        json!({"default_deny": true}),
        json!({"blocked": ["not an address"]}),
        json!({"blocked": ["192.0.2.1/24"]}),
        json!({"blocked": ["0.0.0.0/0"]}),
        json!({"surprise": 1}),
    ] {
        let (status, _) = e.call(&root, "PUT", &path, Some(bad.clone())).await;
        assert!(
            status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
            "{bad} -> {status}"
        );
    }
    // The stored policy did not change.
    let (_, got) = e.call(&root, "GET", &path, None).await;
    assert_eq!(got["policy"]["default_deny"], true);
    // Reset: an empty policy stays (so the node is told to drop the rules).
    let (status, reset) = e.call(&root, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::OK, "{reset}");
    assert_eq!(reset["policy"]["default_deny"], false);
    // No such node.
    let other = format!("/api/v1/admin/nodes/{}/firewall", Uuid::new_v4());
    assert_eq!(
        e.call(&root, "GET", &other, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.call(&root, "PUT", &other, Some(json!({}))).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn malware_scans_are_asked_for_listed_and_only_one_runs_at_a_time(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, tenant) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice_account).await;
    let admin_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(&pool, admin_id, "super_admin", None)
        .await
        .unwrap();
    let root = e.sign_in(admin_id).await;
    let own = "/api/v1/client/malware-scans";
    // A client asks for a scan of a folder of their own files.
    let (status, made) = e
        .call(
            &a,
            "POST",
            own,
            Some(json!({"folder": "wp-content/uploads", "quarantine": true})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        (made["state"].as_str(), made["source"].as_str()),
        (Some("queued"), Some("client"))
    );
    // Not outside the files, and not twice at once.
    for bad in ["../etc", "/etc", "a//b", "a/./b"] {
        let (status, _) = e.call(&a, "POST", own, Some(json!({"folder": bad}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let (status, got) = e.call(&a, "POST", own, Some(json!({}))).await;
    assert_eq!(
        (status, got["code"].as_str()),
        (StatusCode::CONFLICT, Some("scan_under_way")),
        "{got}"
    );
    // The dispatcher's part: sent, then finished with what the node found.
    let id: Uuid = made["id"].as_str().unwrap().parse().unwrap();
    let queued = builtpanel_database::malware::queued(&pool, 10)
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].folder.as_deref(), Some("wp-content/uploads"));
    assert!(queued[0].quarantine);
    // (The operation's own row is the dispatcher's business; the scan only keeps its id.)
    let op = Uuid::new_v4();
    builtpanel_database::malware::sent(&pool, id, op)
        .await
        .unwrap();
    // Still one at a time while it runs.
    let (status, _) = e.call(&a, "POST", own, Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let findings = json!([{"path": "www/wp-content/uploads/x.php", "signature": "Php.Trojan.Agent", "quarantined": true}]);
    builtpanel_database::malware::finish(&pool, id, 1234, &findings, false)
        .await
        .unwrap();
    let (status, list) = e.call(&a, "GET", own, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let first = &list["items"][0];
    assert_eq!(first["state"], "done");
    assert_eq!(first["scanned"], 1234);
    assert_eq!(first["findings"][0]["signature"], "Php.Trojan.Agent");
    assert_eq!(first["findings"][0]["quarantined"], true);
    // After it ended another can be asked for, by an administrator too.
    let path = format!("/api/v1/admin/tenants/{tenant}/malware-scans");
    assert_eq!(
        e.call(&a, "GET", &path, None).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, made) = e.call(&root, "POST", &path, Some(json!({}))).await;
    assert_eq!(
        (status, made["source"].as_str()),
        (StatusCode::ACCEPTED, Some("admin")),
        "{made}"
    );
    let (_, list) = e.call(&root, "GET", &path, None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
    // A tenant just scanned is not due for the scheduled scan; one never scanned would be.
    let due = builtpanel_database::malware::due(&pool, "malware", 7 * 24 * 3600, 10)
        .await
        .unwrap();
    assert!(!due.contains(&tenant));
    // No such hosting account.
    let other = format!("/api/v1/admin/tenants/{}/malware-scans", Uuid::new_v4());
    assert_eq!(
        e.call(&root, "GET", &other, None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn an_old_plugin_is_reported_against_the_advisories(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, tenant) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice_account).await;
    let admin_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(&pool, admin_id, "super_admin", None)
        .await
        .unwrap();
    let root = e.sign_in(admin_id).await;
    let adv = "/api/v1/admin/security/wp-advisories";
    // Advisories are for administrators.
    assert_eq!(e.call(&a, "GET", adv, None).await.0, StatusCode::FORBIDDEN);
    let (status, made) = e
        .call(
            &root,
            "POST",
            adv,
            Some(
                json!({"component": "plugin", "slug": "Old-Forms", "fixed_in": "1.3.0",
                        "severity": "high", "title": "Unauthenticated file upload",
                        "reference": "https://example.com/advisory/1"}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let adv_id = made["id"].as_str().unwrap().to_owned();
    e.call(
        &root,
        "POST",
        adv,
        Some(json!({"component": "core", "slug": "wordpress", "fixed_in": "6.5", "severity": "medium", "title": "Core issue"})),
    )
    .await;
    let (status, dup) = e
        .call(
            &root,
            "POST",
            adv,
            Some(json!({"component": "plugin", "slug": "old-forms", "fixed_in": "1.3.0", "severity": "high", "title": "Unauthenticated file upload"})),
        )
        .await;
    assert_eq!(
        (status, dup["code"].as_str()),
        (StatusCode::CONFLICT, Some("already_listed"))
    );
    for bad in [
        json!({"component": "x", "slug": "a", "fixed_in": "1", "severity": "low", "title": "t"}),
        json!({"component": "plugin", "slug": "a b", "fixed_in": "1", "severity": "low", "title": "t"}),
        json!({"component": "plugin", "slug": "a", "fixed_in": "v1", "severity": "low", "title": "t"}),
        json!({"component": "plugin", "slug": "a", "fixed_in": "1", "severity": "bad", "title": "t"}),
        json!({"component": "plugin", "slug": "a", "fixed_in": "1", "severity": "low", "title": ""}),
        json!({"component": "plugin", "slug": "a", "fixed_in": "1", "severity": "low", "title": "t", "reference": "ftp://x"}),
    ] {
        let (status, _) = e.call(&root, "POST", adv, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    // The client asks for a scan; the node's answer (as the dispatcher writes it) is listed with
    // what is out of date.
    let own = "/api/v1/client/vulnerability-scans";
    let (status, made) = e.call(&a, "POST", own, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    let (status, again) = e.call(&a, "POST", own, None).await;
    assert_eq!(
        (status, again["code"].as_str()),
        (StatusCode::CONFLICT, Some("scan_under_way"))
    );
    let id: Uuid = made["id"].as_str().unwrap().parse().unwrap();
    builtpanel_database::malware::sent(&pool, id, Uuid::new_v4())
        .await
        .unwrap();
    let installs = json!([{
        "path": "www/blog", "core": "6.4.3",
        "plugins": [{"slug": "old-forms", "version": "1.2.3"}, {"slug": "fine", "version": "9.9"}, {"slug": "unknown-version", "version": ""}],
        "themes": []
    }]);
    builtpanel_database::malware::finish(&pool, id, 1, &installs, false)
        .await
        .unwrap();
    let (status, list) = e.call(&a, "GET", own, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let scan = &list["items"][0];
    assert_eq!(scan["state"], "done");
    assert_eq!(scan["installs"][0]["plugins"].as_array().unwrap().len(), 3);
    let issues = scan["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2, "{scan}");
    // Worst first.
    assert_eq!(
        (issues[0]["slug"].as_str(), issues[0]["severity"].as_str()),
        (Some("old-forms"), Some("high"))
    );
    assert_eq!(issues[0]["fixed_in"], "1.3.0");
    assert_eq!(
        (issues[1]["component"].as_str(), issues[1]["slug"].as_str()),
        (Some("core"), Some("wordpress"))
    );
    // The malware scans' list does not mix them in.
    let (_, malware) = e
        .call(&a, "GET", "/api/v1/client/malware-scans", None)
        .await;
    assert_eq!(malware["items"].as_array().unwrap().len(), 0);
    // Removing the advisory clears the issue (judged by what is in force now).
    assert_eq!(
        e.call(&root, "DELETE", &format!("{adv}/{adv_id}"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (_, list) = e.call(&a, "GET", own, None).await;
    assert_eq!(list["items"][0]["issues"].as_array().unwrap().len(), 1);
    let path = format!("/api/v1/admin/tenants/{tenant}/vulnerability-scans");
    assert_eq!(
        e.call(&a, "GET", &path, None).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, admin_list) = e.call(&root, "GET", &path, None).await;
    assert_eq!(
        (status, admin_list["items"].as_array().unwrap().len()),
        (StatusCode::OK, 1)
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn the_images_of_the_list_show_their_findings_and_whether_a_newer_one_exists(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, _) = client(&pool, "alice", 10_000).await;
    let a = e.sign_in(alice_account).await;
    let admin_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'rootuser', 'root@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::rbac::assign_role(&pool, admin_id, "super_admin", None)
        .await
        .unwrap();
    let root = e.sign_in(admin_id).await;
    let digest = format!("sha256:{}", "a".repeat(64));
    let newer = format!("sha256:{}", "b".repeat(64));
    let image = format!("php@{digest}");
    builtpanel_database::images::sync(
        &pool,
        &[
            (
                image.clone(),
                "php".into(),
                Some("8.4-fpm-alpine".into()),
                "PHP".into(),
            ),
            (
                format!("mariadb@{digest}"),
                "db".into(),
                Some("lts".into()),
                "DB".into(),
            ),
        ],
    )
    .await
    .unwrap();
    let path = "/api/v1/admin/security/images";
    assert_eq!(e.call(&a, "GET", path, None).await.0, StatusCode::FORBIDDEN);
    // Not scanned, not checked yet.
    let (status, list) = e.call(&root, "GET", path, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
    assert!(list["items"][0]["scanned_at"].is_null());
    assert_eq!(list["items"][0]["update_available"], false);
    // The dispatcher's findings and the registry's answer arrive.
    let next = builtpanel_database::images::next_to_scan(&pool, 86_400)
        .await
        .unwrap();
    assert!(next.is_some());
    let top = json!([{"id": "CVE-2025-1", "package": "openssl", "installed": "1", "fixed": "2", "severity": "CRITICAL", "title": "bad"}]);
    builtpanel_database::images::store_scan(&pool, &image, [1, 2, 3, 4], &top)
        .await
        .unwrap();
    builtpanel_database::images::store_digest(&pool, "php", "8.4-fpm-alpine", &newer)
        .await
        .unwrap();
    let (_, list) = e.call(&root, "GET", path, None).await;
    let php = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["role"] == "php")
        .unwrap();
    assert_eq!(
        (
            php["critical"].as_i64(),
            php["high"].as_i64(),
            php["low"].as_i64()
        ),
        (Some(1), Some(2), Some(4))
    );
    assert_eq!(php["update_available"], true);
    assert_eq!(php["latest_digest"], newer);
    // A scan just done is not due again for a day; the other image (never scanned) is.
    let due = builtpanel_database::images::next_to_scan(&pool, 86_400)
        .await
        .unwrap();
    assert_eq!(due.as_deref(), Some(format!("mariadb@{digest}").as_str()));
    // The worst findings of one image.
    let (status, vulns) = e
        .call(
            &root,
            "GET",
            &format!("{path}/vulnerabilities?reference={image}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{vulns}");
    assert_eq!(vulns["items"][0]["id"], "CVE-2025-1");
    assert_eq!(
        e.call(
            &root,
            "GET",
            &format!("{path}/vulnerabilities?reference=nope@x"),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // Asking for a scan puts the image first again.
    let (status, _) = e
        .call(
            &root,
            "POST",
            &format!("{path}/scan"),
            Some(json!({"reference": image})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let due = builtpanel_database::images::next_to_scan(&pool, 86_400)
        .await
        .unwrap();
    assert_eq!(due.as_deref(), Some(image.as_str()));
    let (status, _) = e
        .call(
            &root,
            "POST",
            &format!("{path}/scan"),
            Some(json!({"reference": "x@y"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
