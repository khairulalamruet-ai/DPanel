//! The terminal on a node (Server: Terminal), against real PostgreSQL and Redis: a WebSocket client plays the
//! browser and a small SSH server plays the node.
//! Ignored by default; run with DATABASE_URL (superuser), REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_cache::RedisHandle;
use builtpanel_database::rbac::assign_role;
use builtpanel_mod_services::{ServicesState, admin_router, terminal::TerminalSettings};
use builtpanel_session::{CSRF_HEADER, NewSession, SessionConfig, SessionStore, set_cookie};
use futures_util::{SinkExt as _, StreamExt as _};
use russh::{
    Channel, ChannelId, Pty,
    keys::{Algorithm, PrivateKey},
    server::{Auth, ChannelOpenHandle, Msg, Server as _, Session},
};
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest as _};
use tower::ServiceExt as _;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const PASSWORD: &str = "correct horse battery";

// --------------------------------------------------------------------- the node's SSH server

#[derive(Clone)]
struct Fake {
    resized: Arc<Mutex<Vec<(u32, u32)>>>,
}

impl russh::server::Server for Fake {
    type Handler = Ssh;
    fn new_client(&mut self, _: Option<SocketAddr>) -> Ssh {
        Ssh {
            resized: self.resized.clone(),
        }
    }
}

struct Ssh {
    resized: Arc<Mutex<Vec<(u32, u32)>>>,
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

    async fn pty_request(
        &mut self,
        id: ChannelId,
        _term: &str,
        cols: u32,
        rows: u32,
        _pw: u32,
        _ph: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.resized.lock().await.push((cols, rows));
        session.channel_success(id)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        id: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(id)?;
        session.data(id, axum::body::Bytes::from_static(b"welcome\r\n"))?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _id: ChannelId,
        cols: u32,
        rows: u32,
        _pw: u32,
        _ph: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.resized.lock().await.push((cols, rows));
        Ok(())
    }

    async fn data(
        &mut self,
        id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if data.starts_with(b"exit") {
            session.exit_status_request(id, 0)?;
            session.eof(id)?;
            session.close(id)?;
        } else {
            let mut out = b"echo:".to_vec();
            out.extend_from_slice(data);
            session.data(id, axum::body::Bytes::from(out))?;
        }
        Ok(())
    }
}

async fn serve_ssh() -> (u16, String, Arc<Mutex<Vec<(u32, u32)>>>) {
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
    let resized = Arc::new(Mutex::new(Vec::new()));
    let mut server = Fake {
        resized: resized.clone(),
    };
    tokio::spawn(async move {
        let _ = server.run_on_socket(Arc::new(config), &listener).await;
    });
    (port, fingerprint, resized)
}

// --------------------------------------------------------------------- the panel

struct Browser {
    cookie: String,
    csrf: String,
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

async fn sign_in(sessions: &SessionStore, id: Uuid) -> Browser {
    let (t, data) = sessions
        .create(NewSession {
            account_id: id,
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

type Ws = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

async fn open(
    panel: SocketAddr,
    node: Uuid,
    who: &Browser,
    origin: Option<&str>,
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let tcp = tokio::net::TcpStream::connect(panel).await.unwrap();
    let mut req = format!("ws://{panel}/api/v1/admin/nodes/{node}/terminal")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert(header::COOKIE, who.cookie.parse().unwrap());
    if let Some(o) = origin {
        req.headers_mut().insert(header::ORIGIN, o.parse().unwrap());
    }
    tokio_tungstenite::client_async(req, tcp)
        .await
        .map(|(ws, _)| ws)
}

async fn text(ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("an answer in time")
            .expect("the socket is open")
            .unwrap()
        {
            Message::Text(t) => return serde_json::from_str(t.as_str()).unwrap(),
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

async fn bytes_until(ws: &mut Ws, needle: &str) -> String {
    let mut seen = String::new();
    while !seen.contains(needle) {
        match tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("output in time")
            .expect("the socket is open")
            .unwrap()
        {
            Message::Binary(b) => seen.push_str(&String::from_utf8_lossy(&b)),
            Message::Close(_) => panic!("closed while waiting for {needle:?}; saw {seen:?}"),
            _ => {}
        }
    }
    seen
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL + Redis and opens local ports"]
async fn a_terminal_carries_a_shell_to_the_browser_and_keeps_no_secret(pool: PgPool) {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    let sessions = SessionStore::new(redis, pool.clone(), SessionConfig::default());
    let (ssh_port, fingerprint, resized) = serve_ssh().await;
    let node: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name, status, public_ip) VALUES ('n1', 'online', '127.0.0.1') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let app = admin_router().with_state(ServicesState {
        db: pool.clone(),
        sessions: sessions.clone(),
        wait: Duration::from_millis(100),
        terminal: TerminalSettings {
            allow_local_ssh: true,
            ssh_port: Some(ssh_port),
        },
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let panel = listener.local_addr().unwrap();
    {
        let app = app.clone();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
    }
    let root = sign_in(
        &sessions,
        account(&pool, "rootuser", &["super_admin"]).await,
    )
    .await;
    let plain = sign_in(&sessions, account(&pool, "plainadmin", &["admin"]).await).await;

    // The host key is shown first, to a manager only.
    let host_key = |who: &Browser| {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/admin/nodes/{node}/terminal/host-key"))
            .header(header::COOKIE, &who.cookie)
            .header(CSRF_HEADER, &who.csrf)
            .body(Body::empty())
            .unwrap();
        let app = app.clone();
        async move {
            let res = app
                .oneshot({
                    let mut r = req;
                    r.extensions_mut()
                        .insert(axum::extract::ConnectInfo(SocketAddr::from((
                            [198, 51, 100, 7],
                            4000,
                        ))));
                    r
                })
                .await
                .unwrap();
            let status = res.status();
            let body = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
            )
        }
    };
    assert_eq!(host_key(&plain).await.0, StatusCode::FORBIDDEN);
    let (status, shown) = host_key(&root).await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert_eq!(shown["host_key_sha256"], fingerprint);
    assert_eq!(shown["port"], ssh_port);

    // A page of another site, and a non-manager, cannot open a terminal.
    let refused = open(panel, node, &root, Some("https://evil.example")).await;
    assert!(refused.is_err(), "a foreign Origin is refused");
    let refused = open(panel, node, &plain, None).await;
    assert!(refused.is_err(), "no permission, no WebSocket");

    // A wrong password is told, and nothing is kept of it.
    let mut ws = open(panel, node, &root, None).await.unwrap();
    ws.send(Message::text(
        json!({"user": "root", "password": "wrong", "host_key_sha256": fingerprint, "cols": 80, "rows": 24}).to_string(),
    ))
    .await
    .unwrap();
    let said = text(&mut ws).await;
    assert_eq!(said["type"], "error");
    assert_eq!(said["code"], "ssh_login_refused", "{said}");

    // The same with a wrong host key.
    let mut ws = open(panel, node, &root, None).await.unwrap();
    ws.send(Message::text(
        json!({"user": "root", "password": PASSWORD, "host_key_sha256": format!("SHA256:{}", "A".repeat(43)), "cols": 80, "rows": 24}).to_string(),
    ))
    .await
    .unwrap();
    assert_eq!(text(&mut ws).await["code"], "ssh_host_key_changed");

    // A good login: a shell, both ways, a resize, and the end.
    let mut ws = open(panel, node, &root, Some(&format!("http://{panel}")))
        .await
        .unwrap();
    ws.send(Message::text(
        json!({"user": "root", "password": PASSWORD, "host_key_sha256": fingerprint, "cols": 100, "rows": 30}).to_string(),
    ))
    .await
    .unwrap();
    assert_eq!(text(&mut ws).await["type"], "ready");
    bytes_until(&mut ws, "welcome").await;
    ws.send(Message::binary(b"ls -la secret-command\r".to_vec()))
        .await
        .unwrap();
    bytes_until(&mut ws, "echo:ls -la secret-command").await;
    ws.send(Message::text(
        json!({"type": "resize", "cols": 120, "rows": 40}).to_string(),
    ))
    .await
    .unwrap();
    ws.send(Message::binary(b"exit\r".to_vec())).await.unwrap();
    let closed = text(&mut ws).await;
    assert_eq!(closed, json!({"type": "closed", "reason": "shell_ended"}));
    assert_eq!(*resized.lock().await, [(100, 30), (120, 40)]);

    // The audit trail knows that it happened and how long, not what was typed or the password.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action, metadata FROM audit_logs WHERE action LIKE 'node_terminal.%' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let actions: Vec<&str> = rows.iter().map(|(a, _)| a.as_str()).collect();
    assert_eq!(
        actions,
        [
            "node_terminal.login_failed",
            "node_terminal.login_failed",
            "node_terminal.opened",
            "node_terminal.closed"
        ]
    );
    let all: String = rows.iter().map(|(_, m)| m.to_string()).collect();
    assert!(
        !all.contains(PASSWORD) && !all.contains("secret-command") && !all.contains("wrong"),
        "{all}"
    );
    assert_eq!(rows[3].1["reason"], "shell_ended");
    assert!(
        rows[3].1["bytes_in"].as_u64().unwrap() > 0 && rows[3].1["bytes_out"].as_u64().unwrap() > 0
    );
}
