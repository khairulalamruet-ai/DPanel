//! Email over HTTP and the mail tick, against real PostgreSQL and Redis (T13). The node's
//! answers are made up here; the mail server itself is tested by the node agent's `mail_live`.
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
use builtpanel_mod_mail::{MailState, SETTINGS_SCHEMA, admin_router, client_router};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use builtpanel_settings::SettingsRegistry;
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const KEY: [u8; 32] = [7u8; 32];
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
    settings: Arc<SettingsRegistry>,
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
            "mail".to_owned(),
            serde_json::from_str(SETTINGS_SCHEMA).unwrap(),
        )])
        .unwrap(),
    );
    let manifest = Manifest::parse(
        "mail",
        "[module]\nname = \"mail\"\nversion = \"1.0.0\"\ntitle = { en = \"Mail\" }\ncategory = \"core\"\ncore = true\n",
    )
    .unwrap();
    let module = Arc::new(Fake {
        manifest,
        routes: Box::new({
            let state = MailState {
                db: pool.clone(),
                sessions: sessions.clone(),
                settings: settings.clone(),
                master_key: MasterKey::from_bytes(&KEY),
            };
            move || ModuleRoutes {
                admin: Some(admin_router().with_state(state.clone())),
                client: Some(client_router().with_state(state.clone())),
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
        settings,
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

async fn admin(pool: &PgPool, name: &str, roles: &[&str]) -> Uuid {
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

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
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
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, $2, 'primary') RETURNING id",
    )
    .bind(tenant)
    .bind(fqdn)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO dns_zones (domain_id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(fqdn)
        .execute(pool)
        .await
        .unwrap();
    id
}

/// What the node would say: finishes the waiting operations of this kind and returns what
/// they asked for.
async fn answer(pool: &PgPool, kind: &str, output: Value) -> Vec<Value> {
    sqlx::query(
        "UPDATE operations SET status = 'dispatched', attempts = attempts + 1, dispatched_at = now()
          WHERE kind = $1 AND status IN ('pending', 'retryable')",
    )
    .bind(kind)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "UPDATE operations SET status = 'succeeded', output = $2, finished_at = now()
          WHERE kind = $1 AND status = 'dispatched'
          RETURNING payload",
    )
    .bind(kind)
    .bind(output)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn waiting(pool: &PgPool, kind: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE kind = $1 AND status IN ('pending', 'retryable', 'dispatched')",
    )
    .bind(kind)
    .fetch_one(pool)
    .await
    .unwrap()
}

const STRONG: &str = "Correct-Horse-9-Battery";

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn mail_is_made_sent_to_the_node_and_put_in_dns(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, alice_tenant) = client(&pool, "alice", 10_000).await;
    let (bob_account, _) = client(&pool, "bob", 10_001).await;
    let alice = e.sign_in(alice_account).await;
    let bob = e.sign_in(bob_account).await;
    let root = e
        .sign_in(admin(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let domain_id = domain(&pool, alice_tenant, "alice-mail.org").await;
    let node: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE id = $1")
        .bind(alice_tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    let mailboxes = "/api/v1/email/mailboxes";
    let new_box = |name: &str, password: &str| json!({"username": name, "domain": "alice-mail.org", "password": password, "quota_mb": 512});

    // The mail server has no name yet: nothing can be made.
    let (status, got) = e
        .call(&alice, "POST", mailboxes, Some(new_box("info", STRONG)))
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "not_configured"),
        "{got}"
    );
    // An administrator names it.
    let (status, got) = e
        .call(
            &root,
            "PUT",
            &format!("/api/v1/admin/email/servers/{node}"),
            Some(json!({"hostname": "mail.panel-host.org"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");

    // Wrong names, weak passwords, other people's domains.
    for (body, field) in [
        (new_box("Info Me", STRONG), "username"),
        (new_box("info", "short"), "password"),
        (
            json!({"username": "info", "domain": "not-mine.org", "password": STRONG}),
            "domain",
        ),
    ] {
        let (status, got) = e.call(&alice, "POST", mailboxes, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{got}");
        assert!(got.to_string().contains(field), "{field}: {got}");
    }
    let (status, made) = e
        .call(&alice, "POST", mailboxes, Some(new_box("info", STRONG)))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["email"], "info@alice-mail.org");
    assert_eq!(made["status"], "pending");
    let box_id = made["id"].as_str().unwrap().to_owned();
    let (status, got) = e
        .call(&alice, "POST", mailboxes, Some(new_box("info", STRONG)))
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "address_taken")
    );
    // Nobody else sees it or can touch it.
    let (_, list) = e.call(&bob, "GET", mailboxes, None).await;
    assert_ne!(list.as_array().map(Vec::len), Some(1), "{list}");
    let (status, _) = e
        .call(&bob, "DELETE", &format!("{mailboxes}/{box_id}"), None)
        .await;
    assert_ne!(status, StatusCode::NO_CONTENT);

    // The tick: first the server, then the domain, then the rest.
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    assert_eq!(waiting(&pool, "ensure_mail_server").await, 1);
    assert_eq!(
        waiting(&pool, "set_mail_domain").await,
        0,
        "the domain waits for the server"
    );
    answer(
        &pool,
        "ensure_mail_server",
        json!({"type": "mail_server_ready", "created": true}),
    )
    .await;
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let asked = answer(
        &pool,
        "set_mail_domain",
        json!({"type": "mail_domain_set", "records": [
            {"name": "@", "rtype": "MX", "content": "mail.panel-host.org", "priority": 10},
            {"name": "@", "rtype": "TXT", "content": "v=spf1 mx -all"},
            {"name": "builtpanel._domainkey", "rtype": "TXT", "content": "v=DKIM1; k=rsa; p=AAAA"},
            {"name": "_dmarc", "rtype": "TXT", "content": "v=DMARC1; p=quarantine"}
        ]}),
    )
    .await;
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["domain"], "alice-mail.org");
    assert_eq!(asked[0]["per_hour"], 200);
    assert_eq!(asked[0]["held"], false);
    assert_eq!(
        waiting(&pool, "upsert_mailbox").await,
        0,
        "the mailbox waits for the domain"
    );

    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    // The records are in the zone, once.
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT r.name, r.rtype, r.content FROM dns_records r JOIN dns_zones z ON z.id = r.zone_id
          WHERE z.domain_id = $1 ORDER BY r.name, r.rtype",
    )
    .bind(domain_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    for want in [
        ("@", "MX"),
        ("@", "TXT"),
        ("builtpanel._domainkey", "TXT"),
        ("_dmarc", "TXT"),
    ] {
        assert_eq!(
            rows.iter()
                .filter(|r| (r.0.as_str(), r.1.as_str()) == want)
                .count(),
            1,
            "{want:?} in {rows:?}"
        );
    }
    let zone_status: String =
        sqlx::query_scalar("SELECT status FROM dns_zones WHERE domain_id = $1")
            .bind(domain_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        zone_status, "pending",
        "the zone is pushed to the DNS server"
    );
    // The mailbox is sent without its password (it is put in when the frame goes out).
    let sent = answer(
        &pool,
        "upsert_mailbox",
        json!({"type": "mailbox_set", "created": true}),
    )
    .await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["local"], "info");
    assert_eq!(sent[0]["quota_bytes"], 512 * 1024 * 1024);
    assert!(
        sent[0].get("password").is_none_or(Value::is_null),
        "{}",
        sent[0]
    );
    assert!(!sent[0].to_string().contains(STRONG));
    let sealed = builtpanel_database::mail::sealed_password_of(&pool, "alice-mail.org", "info")
        .await
        .unwrap();
    let (id, blob) = sealed.expect("the sealed password waits for the frame");
    let opened = MasterKey::from_bytes(&KEY)
        .open(&builtpanel_database::mail::password_context(id), &blob)
        .unwrap();
    assert_eq!(opened, STRONG.as_bytes());
    answer(
        &pool,
        "set_mail_forwarders",
        json!({"type": "mail_forwarders_set", "count": 0}),
    )
    .await;
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let (_, list) = e
        .call(
            &alice,
            "GET",
            &format!("{mailboxes}?domain=alice-mail.org"),
            None,
        )
        .await;
    assert_eq!(list[0]["status"], "active", "{list}");
    assert!(
        builtpanel_database::mail::sealed_password_of(&pool, "alice-mail.org", "info")
            .await
            .unwrap()
            .is_none(),
        "the password is not kept once the node has it"
    );

    // A new password is sent again.
    let (status, _) = e
        .call(
            &alice,
            "POST",
            &format!("{mailboxes}/{box_id}/password"),
            Some(json!({"password": "Another-Good-Pass-7"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    assert_eq!(
        answer(
            &pool,
            "upsert_mailbox",
            json!({"type": "mailbox_set", "created": false})
        )
        .await
        .len(),
        1
    );

    // Forwarders, the catch-all, and an address that is a mailbox already.
    let fwd = "/api/v1/email/forwarders";
    let (status, got) = e
        .call(
            &alice,
            "POST",
            fwd,
            Some(json!({"source": "info@alice-mail.org", "destination": "x@crm.example.org", "domain": "alice-mail.org"})),
        )
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "address_taken"),
        "{got}"
    );
    let (status, made) = e
        .call(
            &alice,
            "POST",
            fwd,
            Some(json!({"source": "sales@alice-mail.org", "destination": "team@crm.example.org", "domain": "alice-mail.org"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let fwd_id = made["id"].as_str().unwrap().to_owned();
    for (source, destination, field) in [
        ("sales@other.org", "a@crm.example.org", "source"),
        ("sales@alice-mail.org", "not an address", "destination"),
        (
            "sales@alice-mail.org",
            "sales@alice-mail.org",
            "destination",
        ),
    ] {
        let (status, got) = e
            .call(&alice, "POST", fwd, Some(json!({"source": source, "destination": destination, "domain": "alice-mail.org"})))
            .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{source} {destination}: {got}"
        );
        assert!(got.to_string().contains(field), "{got}");
    }
    let (status, _) = e
        .call(
            &alice,
            "POST",
            fwd,
            Some(json!({"source": "*@alice-mail.org", "destination": "boss@crm.example.org", "domain": "alice-mail.org"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let domain_asked = answer(
        &pool,
        "set_mail_domain",
        json!({"type": "mail_domain_set", "records": []}),
    )
    .await;
    assert_eq!(domain_asked[0]["catch_all"], "boss@crm.example.org");
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let fwd_asked = answer(
        &pool,
        "set_mail_forwarders",
        json!({"type": "mail_forwarders_set", "count": 1}),
    )
    .await;
    assert_eq!(
        fwd_asked[0]["forwarders"],
        json!([{"local": "sales", "to": ["team@crm.example.org"]}])
    );
    let (_, list) = e.call(&alice, "GET", fwd, None).await;
    assert_eq!(list.as_array().unwrap().len(), 2, "{list}");
    assert!(list.to_string().contains("sales@alice-mail.org"));
    let (status, _) = e
        .call(&alice, "DELETE", &format!("{fwd}/{fwd_id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // An automatic reply.
    let ar = "/api/v1/email/autoresponders";
    let (status, got) = e
        .call(
            &alice,
            "POST",
            ar,
            Some(json!({"email": "ghost@alice-mail.org", "domain": "alice-mail.org", "subject": "Away", "body": "Back soon"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{got}");
    let (status, got) = e
        .call(
            &alice,
            "POST",
            ar,
            Some(json!({"email": "info@alice-mail.org", "domain": "alice-mail.org", "subject": "Away", "body": "Back soon", "start_date": "2026-12-24", "end_date": "2027-01-02"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{got}");
    assert_eq!(got["is_active"], false);
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let asked = answer(
        &pool,
        "set_mail_autoresponder",
        json!({"type": "mail_autoresponder_set"}),
    )
    .await;
    assert_eq!(asked[0]["enabled"], true);
    assert_eq!(asked[0]["from"], 1_798_070_400_i64);
    let (status, _) = e
        .call(&alice, "DELETE", &format!("{ar}/{box_id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let asked = answer(
        &pool,
        "set_mail_autoresponder",
        json!({"type": "mail_autoresponder_set"}),
    )
    .await;
    assert_eq!(asked[0]["enabled"], false);

    // What a mail program needs.
    let (status, got) = e
        .call(
            &alice,
            "GET",
            "/api/v1/email/config?domain=alice-mail.org",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["incoming_server"], "mail.panel-host.org");
    assert_eq!(got["incoming_imap_port"], 993);

    // The admin sees how the domain stands: the zone has what the mail server asked for.
    let (status, got) = e
        .call(&root, "GET", "/api/v1/admin/email/deliverability", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got[0]["domain"], "alice-mail.org");
    for key in ["spf_status", "dkim_status", "dmarc_status", "mx_status"] {
        assert_eq!(got[0][key], "pass", "{key}: {got}");
    }

    // A spam storm: the queue has far more waiting than the limit.
    let items: Vec<Value> = (0..600)
        .map(|n| {
            json!({"id": format!("Q{n}"), "sender": "spam@alice-mail.org", "recipients": ["x@far.example.org"],
                   "status": "queued", "queued_at": 1_790_000_000, "retries": 0, "size_bytes": 900})
        })
        .collect();
    builtpanel_database::mail::save_queue(&pool, node, &Value::Array(items), 600)
        .await
        .unwrap();
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let (_, domains) = e
        .call(&root, "GET", "/api/v1/admin/email/domains", None)
        .await;
    assert_eq!(domains[0]["held"], true, "{domains}");
    let removed = sqlx::query_scalar::<_, Value>(
        "SELECT payload FROM operations WHERE kind = 'mail_queue_action' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(removed["action"], "delete");
    assert_eq!(removed["ids"].as_array().unwrap().len(), 600);
    let told: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE event_type = 'mail_domain_held' AND account_id IN ($1, (SELECT id FROM accounts WHERE username = 'rootuser'))",
    )
    .bind(alice_account)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(told, 2, "the owner and the administrator are told");
    // The next domain settings carry the hold to the node.
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let asked = answer(
        &pool,
        "set_mail_domain",
        json!({"type": "mail_domain_set", "records": []}),
    )
    .await;
    assert!(asked.iter().any(|a| a["held"] == true), "{asked:?}");
    // The administrator lets it go.
    let domain_row = domains[0]["id"].as_str().unwrap();
    let (status, got) = e
        .call(
            &root,
            "POST",
            &format!("/api/v1/admin/email/domains/{domain_row}/release"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{got}");
    assert_eq!(got["held"], false);

    // The queue as the administrator sees it, and what they can do with it.
    let (status, got) = e
        .call(&root, "GET", "/api/v1/admin/email/queue", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got.as_array().unwrap().len(), 600);
    let (status, _) = e
        .call(&root, "DELETE", "/api/v1/admin/email/queue/Q7", None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, got) = e
        .call(&root, "POST", "/api/v1/admin/email/queue/flush", None)
        .await;
    assert_eq!((status, got["count"].as_i64()), (StatusCode::OK, Some(600)));
    let (status, _) = e
        .call(&alice, "GET", "/api/v1/admin/email/queue", None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Letting it go reaches the node too.
    // (The node answers while the panel keeps changing things, so it takes a few rounds.)
    let mut released = false;
    for _ in 0..6 {
        builtpanel_mod_mail::tick::tick(&pool, &e.settings)
            .await
            .unwrap();
        let asked = answer(
            &pool,
            "set_mail_domain",
            json!({"type": "mail_domain_set", "records": []}),
        )
        .await;
        released |= asked.iter().any(|a| a["held"] == false);
        let caught_up: bool =
            sqlx::query_scalar("SELECT version = applied_version FROM mail_domains")
                .fetch_one(&pool)
                .await
                .unwrap();
        if caught_up {
            break;
        }
    }
    assert!(released, "the release reached the node");

    // Removing a mailbox removes it on the node, then here.
    let (status, _) = e
        .call(&alice, "DELETE", &format!("{mailboxes}/{box_id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = e.call(&alice, "GET", mailboxes, None).await;
    assert_eq!(list.as_array().unwrap().len(), 0, "{list}");
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let dropped = answer(
        &pool,
        "drop_mailbox",
        json!({"type": "mailbox_dropped", "existed": true}),
    )
    .await;
    if dropped.len() != 1 {
        let ops: Vec<(String, String)> =
            sqlx::query_as("SELECT kind, status FROM operations ORDER BY created_at")
                .fetch_all(&pool)
                .await
                .unwrap();
        let state: Vec<(String, i64, i64, Option<String>)> =
            sqlx::query_as("SELECT state, version, applied_version, last_error FROM mail_domains")
                .fetch_all(&pool)
                .await
                .unwrap();
        panic!("no drop: {state:?} {ops:?}");
    }
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM mailboxes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);

    // Deleting the domain takes its mail off the node too, and then the row goes.
    sqlx::query("DELETE FROM domains WHERE id = $1")
        .bind(domain_id)
        .execute(&pool)
        .await
        .unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM mail_domains")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "removing");
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let removed = answer(
        &pool,
        "remove_mail_domain",
        json!({"type": "mail_domain_removed", "existed": true}),
    )
    .await;
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0]["domain"], "alice-mail.org");
    builtpanel_mod_mail::tick::tick(&pool, &e.settings)
        .await
        .unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM mail_domains")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "the mail domain is gone");
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_plan_without_email_or_with_a_small_limit_stops_mailboxes(pool: PgPool) {
    let e = env(&pool).await;
    let (alice_account, alice_tenant) = client(&pool, "alice", 10_000).await;
    let alice = e.sign_in(alice_account).await;
    domain(&pool, alice_tenant, "limited.org").await;
    let node: Uuid = sqlx::query_scalar("SELECT node_id FROM tenants WHERE id = $1")
        .bind(alice_tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    builtpanel_database::mail::want_hostname(&pool, node, "mail.limited-host.org")
        .await
        .unwrap();
    let body =
        |n: u8| json!({"username": format!("u{n}"), "domain": "limited.org", "password": STRONG});
    sqlx::query(
        "UPDATE plans SET limits = limits || '{\"max_email_accounts\": 2}'::jsonb WHERE is_default",
    )
    .execute(&pool)
    .await
    .unwrap();
    for n in 0..2 {
        let (status, got) = e
            .call(&alice, "POST", "/api/v1/email/mailboxes", Some(body(n)))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{got}");
    }
    let (status, got) = e
        .call(&alice, "POST", "/api/v1/email/mailboxes", Some(body(9)))
        .await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::CONFLICT, "limit_reached"),
        "{got}"
    );
    sqlx::query("UPDATE feature_sets SET all_features = false, features = ARRAY['files']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE is_default)")
        .execute(&pool)
        .await
        .unwrap();
    let (status, got) = e.call(&alice, "GET", "/api/v1/email/mailboxes", None).await;
    assert_eq!(
        (status, code(&got)),
        (StatusCode::FORBIDDEN, "feature_disabled")
    );
}
