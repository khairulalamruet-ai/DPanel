//! Setting up a whole new server over SSH (System, Migrate), against real PostgreSQL and Redis and a small SSH
//! server that runs in the test (it answers the two commands the panel sends).
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Request, StatusCode, header},
};
use builtpanel_cache::{
    RedisHandle,
    lockout::{FailureLimiter, LockoutPolicy},
};
use builtpanel_database::rbac::assign_role;
use builtpanel_mod_nodes::{NodesState, router, server_migration::MigrateSettings};
use builtpanel_pki::CertificateAuthority;
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server::{Auth, ChannelOpenHandle, Msg, Server as _, Session},
};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::Mutex;
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const PASSWORD: &str = "correct horse battery";
const FACTS: &str = "os=Ubuntu 24.04.2 LTS\nos_id=ubuntu:24.04\narch=x86_64\ncpus=4\nmemory_kb=8000000\ndisk_free_gb=80\nkvm=yes\nuid=0\nsudo=no\ncgroup2=yes\nsystemd=yes\n";

// --------------------------------------------------------------------- the server being set up

#[derive(Clone)]
struct Fake {
    commands: Arc<Mutex<Vec<String>>>,
}

impl russh::server::Server for Fake {
    type Handler = Ssh;
    fn new_client(&mut self, _: Option<SocketAddr>) -> Ssh {
        Ssh {
            commands: self.commands.clone(),
        }
    }
}

struct Ssh {
    commands: Arc<Mutex<Vec<String>>>,
}

impl russh::server::Handler for Ssh {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "root" && password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        session.channel_success(id)?;
        let out = if command.contains("MemTotal") {
            FACTS.to_owned()
        } else {
            // The installer echoes secrets by mistake: the panel must not keep them.
            let token = command
                .split("BUILTPANEL_ENROLL_TOKEN='")
                .nth(1)
                .and_then(|r| r.split('\'').next())
                .unwrap_or("")
                .to_owned();
            format!("==> installing the node\nlogin was {PASSWORD}\ntoken was {token}\nall done\n")
        };
        self.commands.lock().await.push(command);
        session.data(id, axum::body::Bytes::from(out.into_bytes()))?;
        session.exit_status_request(id, 0)?;
        session.eof(id)?;
        session.close(id)?;
        Ok(())
    }
}

/// Starts the fake server; answers its port, its host key's fingerprint and the commands it was sent.
async fn serve() -> (u16, String, Arc<Mutex<Vec<String>>>) {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fingerprint = key
        .public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string();
    let config = russh::server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let mut server = Fake {
        commands: commands.clone(),
    };
    tokio::spawn(async move {
        let _ = server.run_on_socket(Arc::new(config), &listener).await;
    });
    (port, fingerprint, commands)
}

// --------------------------------------------------------------------- the panel

struct Env {
    app: Router,
    sessions: SessionStore,
}

struct Browser {
    cookie: String,
    csrf: String,
}

fn env(pool: &PgPool) -> Env {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis.clone(), pool.clone(), SessionConfig::default());
    let limiter = FailureLimiter::new(
        redis,
        format!("test:{}:enroll", Uuid::now_v7().simple()),
        LockoutPolicy {
            threshold: 10,
            window: Duration::from_secs(600),
            lockout: Duration::from_secs(900),
        },
    );
    let ca = Arc::new(
        CertificateAuthority::create(
            "Test CA",
            OffsetDateTime::now_utc(),
            time::Duration::days(3650),
        )
        .unwrap()
        .0,
    );
    let app = router()
        .with_state(NodesState {
            db: pool.clone(),
            sessions: sessions.clone(),
            public_url: Some(Arc::from("https://panel.example.com:8443")),
            stream_url: None,
            token_ttl: Duration::from_secs(900),
            ca: Some(ca),
            limiter,
            gateway: None,
            operations: builtpanel_operations::Operations::new(pool.clone()),
            audit: builtpanel_audit::AuditWriter::new(pool.clone()),
            migrate: MigrateSettings {
                installer_url: "https://example.invalid/get.sh".to_owned(),
                allow_local_ssh: true,
            },
        })
        .layer(MockConnectInfo(SocketAddr::from(([198, 51, 100, 7], 4000))));
    Env { app, sessions }
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

    async fn send(
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

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

fn login(port: u16, password: &str, key: &str) -> Value {
    json!({
        "host": "127.0.0.1", "port": port, "user": "root",
        "password": password, "host_key_sha256": key,
    })
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis and opens a local port"]
async fn a_server_is_checked_set_up_and_joined_without_keeping_its_password(pool: PgPool) {
    let e = env(&pool);
    let root = e
        .sign_in(account(&pool, "rootuser", &["super_admin"]).await)
        .await;
    let plain = e
        .sign_in(account(&pool, "plainadmin", &["admin"]).await)
        .await;
    let (port, fingerprint, commands) = serve().await;

    // Only a holder of the manage permission may point the panel at a server.
    let (status, body) = e
        .send(
            &plain,
            "POST",
            "/api/v1/admin/system/migrate/probe",
            Some(json!({"host": "127.0.0.1", "port": port})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "permission_denied")
    );

    // The probe shows the host key, without a login.
    let (status, body) = e
        .send(
            &root,
            "POST",
            "/api/v1/admin/system/migrate/probe",
            Some(json!({"host": "127.0.0.1", "port": port})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["host_key_sha256"], fingerprint);
    assert!(
        commands.lock().await.is_empty(),
        "nothing was run by the probe"
    );

    // Bad input is refused before any connection.
    let (status, body) = e
        .send(
            &root,
            "POST",
            "/api/v1/admin/system/migrate/check",
            Some(json!({"host": "-oProxyCommand=x", "password": "p", "host_key_sha256": "nope"})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
        "{body}"
    );

    // A wrong password and a wrong host key are told apart, and nothing is run.
    let (status, body) = e
        .send(
            &root,
            "POST",
            "/api/v1/admin/system/migrate/check",
            Some(login(port, "wrong", &fingerprint)),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "ssh_login_refused")
    );
    let other = format!("SHA256:{}", "A".repeat(43));
    let (status, body) = e
        .send(
            &root,
            "POST",
            "/api/v1/admin/system/migrate/check",
            Some(login(port, PASSWORD, &other)),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "ssh_host_key_changed")
    );
    assert!(commands.lock().await.is_empty());

    // The check tells what the server is like.
    let (status, facts) = e
        .send(
            &root,
            "POST",
            "/api/v1/admin/system/migrate/check",
            Some(login(port, PASSWORD, &fingerprint)),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{facts}");
    assert_eq!(facts["os"], "Ubuntu 24.04.2 LTS");
    assert_eq!(facts["problems"], json!([]), "{facts}");
    assert_eq!(
        (facts["cpus"].as_u64(), facts["root"].as_bool()),
        (Some(4), Some(true))
    );

    // Start: answers at once, the job runs in the background.
    let mut start = login(port, PASSWORD, &fingerprint);
    start["node_name"] = json!("new-server");
    let (status, job) = e
        .send(
            &root,
            "POST",
            "/api/v1/admin/system/migrate",
            Some(start.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let id = job["id"].as_str().unwrap().to_owned();
    assert_eq!(job["status"], "running");

    // One at a time.
    let (status, body) = e
        .send(
            &root,
            "POST",
            "/api/v1/admin/system/migrate",
            Some(start.clone()),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "migration_running")
    );
    // Moving accounts waits for the node.
    let (status, body) = e
        .send(
            &root,
            "POST",
            &format!("/api/v1/admin/system/migrate/{id}/move-accounts"),
            Some(json!({"from_node_id": Uuid::now_v7()})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "migration_not_done")
    );

    // The installer has run; the agent then connects and the node comes online.
    let mut node_id = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let sent = commands.lock().await.len();
        if sent >= 2 {
            node_id = Some(
                sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO nodes (name, status) VALUES ('new-server', 'online')
                     ON CONFLICT (name) DO UPDATE SET status = 'online' RETURNING id",
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
            );
            break;
        }
    }
    let node_id = node_id.expect("the installer command was sent");
    let mut done = Value::Null;
    for _ in 0..100 {
        let (_, job) = e
            .send(
                &root,
                "GET",
                &format!("/api/v1/admin/system/migrate/{id}"),
                None,
            )
            .await;
        if job["status"] != "running" {
            done = job;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["phase"], "joined");
    assert_eq!(done["node_id"], node_id.to_string());

    // What was sent: the join command with the token in the environment.
    let sent = commands.lock().await.clone();
    let install = sent.last().unwrap();
    assert!(
        install.contains("BUILTPANEL_ENROLL_TOKEN='dhn_"),
        "{install}"
    );
    assert!(install.contains("--join-server 'https://panel.example.com:8443'"));
    assert!(install.contains("--join-fingerprint 'sha256:"));
    assert!(!install.contains("sudo"), "root needs no sudo");
    assert!(!install.contains(PASSWORD));

    // What was kept: the output, without the password and the token the installer echoed.
    let log = done["log"].as_str().unwrap();
    assert!(
        log.contains("installing the node") && log.contains("all done"),
        "{log}"
    );
    assert!(
        log.contains("login was ***") && log.contains("token was ***"),
        "{log}"
    );
    assert!(!log.contains(PASSWORD) && !log.contains("dhn_"), "{log}");
    let token_text = install
        .split("BUILTPANEL_ENROLL_TOKEN='")
        .nth(1)
        .and_then(|r| r.split('\'').next())
        .unwrap();
    let everywhere: String = sqlx::query_scalar(
        "SELECT (SELECT coalesce(string_agg(metadata::text, ' '), '') FROM audit_logs)
             || (SELECT coalesce(string_agg(log || host || phase, ' '), '') FROM server_migrations)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !everywhere.contains(PASSWORD) && !everywhere.contains(token_text),
        "{everywhere}"
    );

    // The audit trail names the server, not the credential.
    let started: Vec<Value> = sqlx::query_scalar(
        "SELECT metadata FROM audit_logs WHERE action = 'admin.server_migration_started'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0]["node_name"], "new-server");

    // It is listed, and moving accounts needs a backup destination and another node.
    let (status, list) = e
        .send(&root, "GET", "/api/v1/admin/system/migrate", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (status, body) = e
        .send(
            &root,
            "POST",
            &format!("/api/v1/admin/system/migrate/{id}/move-accounts"),
            Some(json!({"from_node_id": node_id})),
        )
        .await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "same_node"));
    let old: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name, status) VALUES ('old-server', 'online') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let (status, body) = e
        .send(
            &root,
            "POST",
            &format!("/api/v1/admin/system/migrate/{id}/move-accounts"),
            Some(json!({"from_node_id": old})),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "no_backup_destination"),
        "{body}"
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis and opens a local port"]
async fn a_job_that_failed_says_why_and_a_silent_one_is_given_up(pool: PgPool) {
    let e = env(&pool);
    let who = account(&pool, "rootuser", &["super_admin"]).await;
    let root = e.sign_in(who).await;
    let (port, fingerprint, _) = serve().await;

    // A panel with no node name rule broken: the name is checked first.
    let mut start = login(port, PASSWORD, &fingerprint);
    start["node_name"] = json!("Bad Name");
    let (status, body) = e
        .send(&root, "POST", "/api/v1/admin/system/migrate", Some(start))
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed")
    );

    // A job that stopped reporting is shown as failed instead of running for ever.
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO server_migrations (id, host, port, ssh_user, node_name, created_by, updated_at)
         VALUES (uuidv7(), 'h.example.com', 22, 'root', 'lost-node', $1, now() - interval '3 hours')
         RETURNING id",
    )
    .bind(who)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (status, job) = e
        .send(
            &root,
            "GET",
            &format!("/api/v1/admin/system/migrate/{id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (job["status"].as_str(), job["phase"].as_str()),
        (Some("failed"), Some("lost"))
    );
}
