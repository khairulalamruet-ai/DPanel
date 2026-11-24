//! A real import, end to end through the whole application as it is mounted (T15.04).
//!
//! The panel is the real one (every module, the real routes); only the node is played by a task
//! that does what a node would: it finishes the queued operations and marks what they made as
//! ready. So this checks the executor's orchestration against the real API, limits and database;
//! it does not check what a node does with the work (the node agent has its own tests).
//!
//! Needs PostgreSQL and Redis (`DATABASE_URL`, `REDIS_URL`, `--include-ignored`).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{io::Write as _, sync::Arc, time::Duration};

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

use crate::{app, modules};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

struct App {
    router: Router,
    sessions: SessionStore,
    pool: PgPool,
}

struct Who {
    cookie: String,
    csrf: String,
}

async fn app(pool: &PgPool, spool: &std::path::Path) -> App {
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
        public_url: Some("http://panel.test".into()),
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
            spool_dir: spool.to_path_buf(),
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
        router,
        sessions,
        pool: pool.clone(),
    }
}

impl App {
    async fn root(&self) -> Who {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (kind, username, email, password_hash)
             VALUES ('admin', 'rootuser', 'rootuser@example.com', $1) RETURNING id",
        )
        .bind(HASH)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        builtpanel_database::rbac::assign_role(&self.pool, id, "super_admin", None)
            .await
            .unwrap();
        let (token, data) = self
            .sessions
            .create(NewSession {
                account_id: id,
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
        who: &Who,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, &who.cookie)
            .header(CSRF_HEADER, &who.csrf);
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.router.clone().oneshot(req).await.unwrap();
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

/// What a node would do: finish the work it is given and say what it made is there.
fn play_the_node(pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            for sql in [
                "UPDATE operations SET status = 'dispatched', dispatched_at = now(), attempts = 1 \
                 WHERE status = 'pending'",
                "UPDATE operations SET status = 'succeeded', finished_at = now(), \
                 output = '{\"type\": \"file\", \"result\": {\"result\": \"done\", \"count\": 1}}' \
                 WHERE status = 'dispatched'",
                "UPDATE tenants SET status = 'active' WHERE status = 'provisioning'",
                "UPDATE databases SET status = 'ready' WHERE status = 'pending'",
                "UPDATE db_users SET status = 'ready' WHERE status = 'pending'",
                "UPDATE file_transfers SET status = 'done' WHERE status IN ('queued', 'running')",
                "UPDATE db_imports SET status = 'done' WHERE status IN ('queued', 'running')",
            ] {
                let _ = sqlx::query(sql).execute(&pool).await;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    })
}

fn tar_gz(files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, content) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(content.len() as u64);
        h.set_mode(0o644);
        h.set_entry_type(tar::EntryType::Regular);
        tar.append_data(&mut h, path, content.as_bytes()).unwrap();
    }
    let raw = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

fn cpmove() -> Vec<u8> {
    tar_gz(&[
        ("cpmove-alice/version", "pkgacct version 11.110\n"),
        (
            "cpmove-alice/cp/alice",
            "USER=alice\nDNS=example.com\nCONTACTEMAIL=alice@example.net\nPLAN=gold\nSTARTDATE=1700000000\nLOCALE=en\n",
        ),
        (
            "cpmove-alice/userdata/main",
            "---\nmain_domain: example.com\naddon_domains:\n  shop.example.net: shop.example.com\nparked_domains: []\nsub_domains:\n  - blog.example.com\n",
        ),
        (
            "cpmove-alice/userdata/example.com",
            "---\ndocumentroot: /home/alice/public_html\nhomedir: /home/alice\nphpversion: ea-php81\n",
        ),
        (
            "cpmove-alice/userdata/shop.example.net",
            "documentroot: /home/alice/shop\nphpversion: ea-php74\n",
        ),
        (
            "cpmove-alice/mysql/alice_wp.sql",
            "-- dump\nCREATE TABLE t (id int);\n",
        ),
        (
            "cpmove-alice/dnszones/shop.example.net.db",
            "$TTL 14400\n@ 14400 IN SOA ns1.old.example. hostmaster.shop.example.net. ( 2024010101 3600 1800 1209600 86400 )\nshop.example.net. 14400 IN NS ns1.old.example.\nshop.example.net. 14400 IN A 203.0.113.10\nwww 14400 IN CNAME shop.example.net.\nmail 14400 IN A 203.0.113.99\nshop.example.net. 14400 IN MX 10 mail.shop.example.net.\nshop.example.net. 14400 IN TXT \"v=spf1 +a +mx ~all\"\n",
        ),
        (
            "cpmove-alice/mysql.sql",
            "GRANT USAGE ON *.* TO 'alice_wpu'@'localhost' IDENTIFIED BY PASSWORD '*2470C0C06DEE42FD1618BB99005ADCA2EC9D1E19';\nGRANT ALL PRIVILEGES ON `alice\\_wp`.* TO 'alice_wpu'@'localhost';\n",
        ),
        (
            "cpmove-alice/cron/alice",
            "*/10 * * * * /usr/bin/php /home/alice/cron.php\n",
        ),
        (
            "cpmove-alice/homedir/public_html/index.php",
            "<?php echo 1;",
        ),
        ("cpmove-alice/homedir/public_html/a.txt", "abcd"),
        ("cpmove-alice/homedir/shop/index.html", "shop"),
        ("cpmove-alice/homedir/mail/secret", "not restored"),
    ])
}

async fn wait_for_end(a: &App, root: &Who, id: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..120 {
        let (status, t) = a
            .send(
                root,
                "GET",
                &format!("/api/v1/admin/migration/transfers/{id}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{t}");
        last = t.clone();
        if t["status"] != "running" && t["status"] != "pending" {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let ops: Vec<(String, String)> =
        sqlx::query_as("SELECT kind, status FROM operations ORDER BY created_at")
            .fetch_all(&a.pool)
            .await
            .unwrap();
    let transfers: Vec<(String, String)> =
        sqlx::query_as("SELECT kind, status FROM file_transfers")
            .fetch_all(&a.pool)
            .await
            .unwrap();
    panic!(
        "the transfer did not end: {}\noperations {ops:?}\nfile transfers {transfers:?}",
        last["logs"]
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL + Redis"]
async fn a_real_import_makes_the_account_and_everything_in_it(pool: PgPool) {
    let spool = std::env::temp_dir().join(format!("bp-import-{}", Uuid::new_v4()));
    std::fs::create_dir_all(spool.join("migration")).unwrap();
    std::fs::write(spool.join("migration/cpmove-alice.tar.gz"), cpmove()).unwrap();
    let a = app(&pool, &spool).await;
    sqlx::query("INSERT INTO nodes (name, status) VALUES ('node1', 'online') ON CONFLICT (name) DO UPDATE SET status = 'online'")
        .execute(&pool)
        .await
        .unwrap();
    let root = a.root().await;
    let node = play_the_node(pool.clone());

    let (status, made) = a
        .send(
            &root,
            "POST",
            "/api/v1/admin/migration/transfers",
            Some(json!({
                "source_type": "cpanel", "backup_file": "cpmove-alice.tar.gz",
                "tenant_username": "alice", "target_domain": "alice.example.org",
                "dry_run": false
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["id"].as_str().unwrap().to_owned();
    let done = wait_for_end(&a, &root, &id).await;
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["progress_percent"], 100, "{done}");
    assert!(
        done["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["status"] == "completed"),
        "{done}"
    );

    // The account, its domains, database, user, grant and job are there.
    let tenant: (String, String) = sqlx::query_as(
        "SELECT t.username, t.status FROM tenants t JOIN accounts a ON a.id = t.owner_account_id WHERE a.username = 'alice'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tenant, ("alice".to_owned(), "active".to_owned()));
    let domains: Vec<(String, String)> =
        sqlx::query_as("SELECT fqdn::text, kind FROM domains ORDER BY fqdn")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        domains
            .iter()
            .any(|(f, k)| f == "alice.example.org" && k == "primary"),
        "{domains:?}"
    );
    assert!(
        domains.iter().any(|(f, _)| f == "shop.example.net"),
        "{domains:?}"
    );
    let db: String = sqlx::query_scalar("SELECT status FROM databases WHERE name = 'alice_wp'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(db, "ready");
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM db_grants")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(grants, 1);
    // The records that do not point at the old server came over; the one that does did not.
    let records: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT r.name, r.rtype, r.content FROM dns_records r JOIN dns_zones z ON z.id = r.zone_id
         WHERE z.name = 'shop.example.net'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        records
            .iter()
            .any(|(n, t, c)| n == "mail" && t == "A" && c == "203.0.113.99"),
        "{records:?}"
    );
    assert!(
        records
            .iter()
            .any(|(_, t, c)| t == "MX" && c.trim_end_matches('.') == "mail.shop.example.net"),
        "{records:?}"
    );
    assert!(
        records
            .iter()
            .any(|(_, t, c)| t == "TXT" && c.starts_with("v=spf1")),
        "{records:?}"
    );
    assert!(
        !records.iter().any(|(_, _, c)| c == "203.0.113.10"),
        "{records:?}"
    );
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM cron_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(jobs, 1);
    // The dump and the files went through the same routes a person uses.
    let imports: i64 = sqlx::query_scalar("SELECT count(*) FROM db_imports WHERE status = 'done'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(imports, 1);
    let uploads: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM file_transfers WHERE kind = 'upload' AND status = 'done'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(uploads, 1);
    // The audit trail names the administrator who started it, not the account.
    let actors: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'admin.user_created'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(actors, 1);
    // Nothing is left in the spool but the backup.
    let left: Vec<String> = std::fs::read_dir(spool.join("migration"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["cpmove-alice.tar.gz"]);

    // The same backup again: the names are taken, so the plan stops it before anything is done.
    let (status, again) = a
        .send(
            &root,
            "POST",
            "/api/v1/admin/migration/transfers",
            Some(json!({
                "source_type": "cpanel", "backup_file": "cpmove-alice.tar.gz",
                "tenant_username": "alice", "target_domain": "alice.example.org",
                "dry_run": false
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{again}");
    assert_eq!(again["status"], "failed", "{again}");
    assert_eq!(again["plan"]["ok"], false);
    node.abort();
}
