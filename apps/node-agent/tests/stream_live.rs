//! The real agent connection code against the real agent port: mutual TLS 1.3,
//! WebSocket, heartbeat, presence in PostgreSQL, reconnect.
//! Ignored by default; run with DATABASE_URL (superuser) and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use builtpanel_database::nodes::{self, NewNode, NewNodeCertificate};
use builtpanel_node_agent::stream::{self, Params, client_config};
use builtpanel_node_gateway::{Gateway, Settings, tls};
use builtpanel_node_protocol::{
    AgentMessage, ControlMessage, Hello, PROTOCOL_VERSION, STREAM_PATH, decode, encode,
};
use builtpanel_pki::CertificateAuthority;
use futures_util::{SinkExt as _, StreamExt as _};
use rcgen::{CertificateParams, KeyPair};
use rustls::pki_types::{ServerName, pem::PemObject as _};
use secrecy::SecretString;
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::{net::TcpStream, sync::watch};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::{
    WebSocketStream, client_async,
    tungstenite::{Message, protocol::CloseFrame},
};
use uuid::Uuid;

/// Short timings so the tests finish in seconds; production uses 15 s and 60 s.
const PING: Duration = Duration::from_millis(200);
const LIVENESS: Duration = Duration::from_secs(1);

struct Plane {
    ca: Arc<CertificateAuthority>,
    gateway: Gateway,
    addr: SocketAddr,
    stop: watch::Sender<bool>,
}

fn settings() -> Settings {
    Settings {
        ping_interval: PING,
        liveness_timeout: LIVENESS,
        handshake_timeout: Duration::from_secs(5),
        max_connections: 64,
    }
}

fn gateway_for(ca: &CertificateAuthority, pool: &PgPool) -> Gateway {
    let (server, key) = ca
        .issue_server_certificate(
            &["127.0.0.1".to_owned()],
            OffsetDateTime::now_utc(),
            time::Duration::days(30),
        )
        .unwrap();
    use secrecy::ExposeSecret as _;
    let config = tls::server_config(
        ca.certificate_pem(),
        server.cert_pem.as_bytes(),
        key.expose_secret().as_bytes(),
    )
    .unwrap();
    Gateway::new(pool.clone(), config, settings())
}

async fn plane_on(pool: &PgPool, ca: Arc<CertificateAuthority>, bind: &str) -> Plane {
    let gateway = gateway_for(&ca, pool);
    // A port just given up by a stopped plane (or a reserved one) can stay busy for a moment
    // on a loaded machine, so binding is tried again for a few seconds.
    let started = std::time::Instant::now();
    let listener = loop {
        match builtpanel_node_gateway::bind(bind.parse().unwrap()).await {
            Ok(listener) => break listener,
            Err(error) if started.elapsed() > Duration::from_secs(10) => {
                panic!("could not bind {bind}: {error}")
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    };
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = watch::channel(false);
    tokio::spawn(gateway.clone().serve(listener, stopped));
    Plane {
        ca,
        gateway,
        addr,
        stop,
    }
}

fn new_ca() -> Arc<CertificateAuthority> {
    Arc::new(
        CertificateAuthority::create(
            "Test CA",
            OffsetDateTime::now_utc(),
            time::Duration::days(3650),
        )
        .unwrap()
        .0,
    )
}

async fn plane(pool: &PgPool) -> Plane {
    plane_on(pool, new_ca(), "127.0.0.1:0").await
}

/// A node with a certificate from the plane's CA, registered in the database.
struct TestNode {
    id: Uuid,
    name: String,
    ca_pem: String,
    cert_pem: String,
    key_pem: String,
    fingerprint: String,
}

impl TestNode {
    fn params(&self, addr: SocketAddr) -> Params {
        Params {
            address: format!("https://{addr}"),
            ca_pem: self.ca_pem.clone(),
            cert_pem: self.cert_pem.clone(),
            key_pem: SecretString::from(self.key_pem.clone()),
        }
    }
}

async fn make_node(
    ca: &CertificateAuthority,
    pool: &PgPool,
    name: &str,
    register: bool,
) -> TestNode {
    let id = Uuid::now_v7();
    let key = KeyPair::generate().unwrap();
    let csr = CertificateParams::new(Vec::<String>::new())
        .unwrap()
        .serialize_request(&key)
        .and_then(|c| c.pem())
        .unwrap();
    let issued = ca
        .issue_node_certificate(
            &csr,
            id,
            OffsetDateTime::now_utc(),
            time::Duration::days(90),
        )
        .unwrap();
    if register {
        let mut conn = pool.acquire().await.unwrap();
        nodes::insert_node(
            &mut conn,
            &NewNode {
                id,
                name,
                hostname: Some("box"),
                public_ip: None,
                agent_version: None,
            },
        )
        .await
        .unwrap();
        nodes::insert_node_certificate(
            &mut conn,
            &NewNodeCertificate {
                node_id: id,
                serial: &issued.serial_hex,
                fingerprint_sha256: &issued.fingerprint_sha256,
                not_before: issued.not_before,
                not_after: issued.not_after,
            },
        )
        .await
        .unwrap();
    }
    TestNode {
        id,
        name: name.to_owned(),
        ca_pem: ca.certificate_pem().to_owned(),
        cert_pem: issued.cert_pem,
        key_pem: key.serialize_pem(),
        fingerprint: issued.fingerprint_sha256,
    }
}

async fn status(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM nodes WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn wait_for_status(pool: &PgPool, id: Uuid, wanted: &str, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if status(pool, id).await == wanted {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn test_executor() -> Arc<builtpanel_node_agent::ops::Executor> {
    use builtpanel_node_agent::ops::{Executor, Registry, cache::ResultCache};
    Executor::new(
        Registry::builtin(&std::env::temp_dir()),
        ResultCache::in_memory().unwrap(),
        8,
    )
}

/// Starts the real agent loop; the returned sender stops it.
fn start_agent(params: Params) -> (watch::Sender<bool>, tokio::task::JoinHandle<()>) {
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(async move {
        stream::run(params, test_executor(), stopped).await.unwrap();
    });
    (stop, task)
}

type Raw = WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>;

/// A hand-driven client: connects with the node's certificate and says hello,
/// but does not read unless the test does (so it never answers pings).
async fn raw_connect(
    node: &TestNode,
    addr: SocketAddr,
    protocol_version: u16,
) -> Result<Raw, String> {
    let config = client_config(&node.params(addr)).unwrap();
    let tcp = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
    let tls = TlsConnector::from(config)
        .connect(ServerName::try_from("127.0.0.1".to_owned()).unwrap(), tcp)
        .await
        .map_err(|e| e.to_string())?;
    let (mut ws, _) = client_async(format!("wss://{addr}{STREAM_PATH}"), tls)
        .await
        .map_err(|e| e.to_string())?;
    let hello = encode(&AgentMessage::Hello(Hello {
        protocol_version,
        agent_version: "raw-test".into(),
    }))
    .unwrap();
    ws.send(Message::Text(hello.into()))
        .await
        .map_err(|e| e.to_string())?;
    Ok(ws)
}

async fn read_welcome(ws: &mut Raw) -> Result<(), String> {
    match tokio::time::timeout(Duration::from_secs(3), ws.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => match decode::<ControlMessage>(t.as_str()) {
            Ok(ControlMessage::Welcome(_)) => Ok(()),
            other => Err(format!("not a welcome: {other:?}")),
        },
        other => Err(format!("no welcome: {other:?}")),
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_agent_connects_shows_online_and_goes_offline_when_it_stops(pool: PgPool) {
    let p = plane(&pool).await;
    let node = make_node(&p.ca, &pool, "alpha", true).await;
    assert_eq!(status(&pool, node.id).await, "offline");

    let (stop, task) = start_agent(node.params(p.addr));
    assert!(
        wait_for_status(&pool, node.id, "online", Duration::from_secs(5)).await,
        "node must come online"
    );
    assert!(p.gateway.is_connected(node.id));
    let (version, ip, seen): (String, String, bool) = sqlx::query_as(
        "SELECT agent_version, host(public_ip), last_seen_at > now() - interval '5 seconds' FROM nodes WHERE id = $1",
    )
    .bind(node.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(version, builtpanel_node_agent::AGENT_VERSION);
    assert_eq!(ip, "127.0.0.1");
    assert!(seen);

    // Heartbeats keep last_seen_at moving while the agent is idle.
    let before: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM last_seen_at)::float8 FROM nodes WHERE id = $1",
    )
    .bind(node.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let after: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM last_seen_at)::float8 FROM nodes WHERE id = $1",
    )
    .bind(node.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        after > before,
        "last_seen_at must advance: {before} -> {after}"
    );

    // A clean stop is noticed at once.
    stop.send(true).unwrap();
    task.await.unwrap();
    assert!(wait_for_status(&pool, node.id, "offline", Duration::from_secs(3)).await);
    assert!(!p.gateway.is_connected(node.id));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_agent_reconnects_by_itself_when_the_control_plane_comes_back(pool: PgPool) {
    let ca = new_ca();
    // Reserve a port, release it, and start the agent before anything listens.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let node = make_node(&ca, &pool, "beta", true).await;
    let (stop, task) = start_agent(node.params(addr));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(status(&pool, node.id).await, "offline");

    let first = plane_on(&pool, ca.clone(), &addr.to_string()).await;
    assert!(
        wait_for_status(&pool, node.id, "online", Duration::from_secs(8)).await,
        "the agent must find the control plane once it is up"
    );

    // The control plane goes away: the node ends up offline and the agent keeps trying.
    first.stop.send(true).unwrap();
    assert!(wait_for_status(&pool, node.id, "offline", Duration::from_secs(8)).await);
    let second = plane_on(&pool, ca.clone(), &addr.to_string()).await;
    assert!(
        wait_for_status(&pool, node.id, "online", Duration::from_secs(10)).await,
        "and come back after a restart"
    );
    drop(second);
    stop.send(true).unwrap();
    task.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn only_known_unrevoked_certificates_from_our_ca_get_in(pool: PgPool) {
    let p = plane(&pool).await;

    // Signed by our CA but never recorded (for example a certificate whose row was removed).
    let unknown = make_node(&p.ca, &pool, "ghost", false).await;
    let refused = raw_connect(&unknown, p.addr, PROTOCOL_VERSION).await;
    if let Ok(mut ws) = refused {
        assert!(
            read_welcome(&mut ws).await.is_err(),
            "unknown certificate got a session"
        );
    }
    assert!(!p.gateway.is_connected(unknown.id));

    // Signed by another CA: the TLS handshake itself fails.
    let other = new_ca();
    let stranger = make_node(&other, &pool, "stranger", true).await;
    let mut params = stranger.params(p.addr);
    params.ca_pem = p.ca.certificate_pem().to_owned(); // trusts our server, presents their certificate
    let config = client_config(&params).unwrap();
    let tcp = TcpStream::connect(p.addr).await.unwrap();
    let tls = TlsConnector::from(config)
        .connect(ServerName::try_from("127.0.0.1".to_owned()).unwrap(), tcp)
        .await;
    let upgraded = match tls {
        Ok(tls) => client_async(format!("wss://{}{STREAM_PATH}", p.addr), tls)
            .await
            .is_ok(),
        Err(_) => false,
    };
    assert!(
        !upgraded,
        "a certificate from another CA must not get a WebSocket"
    );
    assert_eq!(status(&pool, stranger.id).await, "offline");

    // No client certificate at all.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    for c in rustls::pki_types::CertificateDer::pem_slice_iter(p.ca.certificate_pem().as_bytes()) {
        roots.add(c.unwrap()).unwrap();
    }
    let anonymous = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tcp = TcpStream::connect(p.addr).await.unwrap();
    let tls = TlsConnector::from(Arc::new(anonymous))
        .connect(ServerName::try_from("127.0.0.1".to_owned()).unwrap(), tcp)
        .await;
    let upgraded = match tls {
        Ok(tls) => client_async(format!("wss://{}{STREAM_PATH}", p.addr), tls)
            .await
            .is_ok(),
        Err(_) => false,
    };
    assert!(!upgraded, "no certificate, no WebSocket");

    // A revoked certificate and a revoked node are refused at the next connect.
    let revoked_cert = make_node(&p.ca, &pool, "revoked-cert", true).await;
    sqlx::query("UPDATE node_certificates SET revoked_at = now(), revoke_reason = 'admin' WHERE fingerprint_sha256 = $1")
        .bind(&revoked_cert.fingerprint)
        .execute(&pool)
        .await
        .unwrap();
    let revoked_node = make_node(&p.ca, &pool, "revoked-node", true).await;
    sqlx::query("UPDATE nodes SET status = 'revoked' WHERE id = $1")
        .bind(revoked_node.id)
        .execute(&pool)
        .await
        .unwrap();
    for node in [&revoked_cert, &revoked_node] {
        if let Ok(mut ws) = raw_connect(node, p.addr, PROTOCOL_VERSION).await {
            assert!(
                read_welcome(&mut ws).await.is_err(),
                "{} got a session",
                node.name
            );
        }
        assert!(!p.gateway.is_connected(node.id), "{}", node.name);
        assert_ne!(status(&pool, node.id).await, "online", "{}", node.name);
    }

    // And a good one still works next to all that.
    let good = make_node(&p.ca, &pool, "good", true).await;
    let mut ws = raw_connect(&good, p.addr, PROTOCOL_VERSION).await.unwrap();
    read_welcome(&mut ws).await.unwrap();
    assert!(wait_for_status(&pool, good.id, "online", Duration::from_secs(3)).await);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_newer_connection_replaces_the_older_one_and_the_node_stays_online(pool: PgPool) {
    let p = plane(&pool).await;
    let node = make_node(&p.ca, &pool, "gamma", true).await;

    let mut first = raw_connect(&node, p.addr, PROTOCOL_VERSION).await.unwrap();
    read_welcome(&mut first).await.unwrap();
    assert!(wait_for_status(&pool, node.id, "online", Duration::from_secs(3)).await);

    let mut second = raw_connect(&node, p.addr, PROTOCOL_VERSION).await.unwrap();
    read_welcome(&mut second).await.unwrap();

    // The first one is told to go (a Close frame, or the connection simply ends).
    let mut closed = false;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_millis(500), first.next()).await {
            Ok(Some(Ok(Message::Close(Some(CloseFrame { reason, .. }))))) => {
                assert!(reason.contains("replaced"), "{reason}");
                closed = true;
                break;
            }
            Ok(None | Some(Err(_) | Ok(Message::Close(None)))) => {
                closed = true;
                break;
            }
            Ok(Some(Ok(_))) | Err(_) => {}
        }
    }
    assert!(closed, "the old session must be closed");
    // Its goodbye must not take the node offline: the new session owns it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(status(&pool, node.id).await, "online");
    assert!(p.gateway.is_connected(node.id));
    drop(second);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_silent_agent_is_dropped_and_shows_offline_within_the_liveness_timeout(pool: PgPool) {
    let p = plane(&pool).await;
    let node = make_node(&p.ca, &pool, "delta", true).await;
    // Connects, then never reads again: pings go unanswered.
    let mut ws = raw_connect(&node, p.addr, PROTOCOL_VERSION).await.unwrap();
    read_welcome(&mut ws).await.unwrap();
    assert!(wait_for_status(&pool, node.id, "online", Duration::from_secs(3)).await);
    let silent_since = tokio::time::Instant::now();
    assert!(
        wait_for_status(&pool, node.id, "offline", LIVENESS * 4).await,
        "a node that stops answering must go offline"
    );
    assert!(silent_since.elapsed() < LIVENESS * 4);
    assert!(!p.gateway.is_connected(node.id));
    drop(ws);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn bad_first_messages_do_not_get_a_session(pool: PgPool) {
    let p = plane(&pool).await;
    let node = make_node(&p.ca, &pool, "epsilon", true).await;

    // A protocol version nobody speaks.
    let mut ws = raw_connect(&node, p.addr, 0).await.unwrap();
    assert!(read_welcome(&mut ws).await.is_err());
    assert_eq!(status(&pool, node.id).await, "offline");

    // Something that is not a hello.
    let mut ws = raw_connect(&node, p.addr, PROTOCOL_VERSION).await.unwrap();
    read_welcome(&mut ws).await.unwrap();
    ws.send(Message::Text("{\"nonsense\":true}".into()))
        .await
        .unwrap();
    let mut ended = false;
    for _ in 0..10 {
        if let Ok(None | Some(Err(_) | Ok(Message::Close(_)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
        {
            ended = true;
            break;
        }
    }
    assert!(ended, "a malformed message must end the session");
    assert!(wait_for_status(&pool, node.id, "offline", Duration::from_secs(3)).await);

    // Binary frames are not part of the protocol.
    let mut ws = raw_connect(&node, p.addr, PROTOCOL_VERSION).await.unwrap();
    read_welcome(&mut ws).await.unwrap();
    ws.send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    let mut ended = false;
    for _ in 0..10 {
        if let Ok(None | Some(Err(_) | Ok(Message::Close(_)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
        {
            ended = true;
            break;
        }
    }
    assert!(ended, "a binary frame must end the session");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn nodes_left_online_by_a_stopped_control_plane_are_swept_and_listed_as_offline(
    pool: PgPool,
) {
    let p = plane(&pool).await;
    let node = make_node(&p.ca, &pool, "zeta", true).await;
    // As if a previous control plane died while the node was online.
    sqlx::query("UPDATE nodes SET status = 'online', last_seen_at = now() - interval '5 minutes' WHERE id = $1")
        .bind(node.id)
        .execute(&pool)
        .await
        .unwrap();
    // The listing never claims a stale node is online, even before the sweep.
    let listed = nodes::list_nodes(&pool, 10, 60).await.unwrap();
    assert_eq!(
        listed.iter().find(|n| n.id == node.id).unwrap().status,
        "offline"
    );
    // And the sweep fixes the stored value.
    assert!(wait_for_status(&pool, node.id, "offline", Duration::from_secs(3)).await);

    // A fresh heartbeat keeps a node online in the listing.
    sqlx::query("UPDATE nodes SET status = 'online', last_seen_at = now() WHERE id = $1")
        .bind(node.id)
        .execute(&pool)
        .await
        .unwrap();
    let listed = nodes::list_nodes(&pool, 10, 60).await.unwrap();
    assert_eq!(
        listed.iter().find(|n| n.id == node.id).unwrap().status,
        "online"
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_revoked_node_loses_its_connection_and_cannot_come_back(pool: PgPool) {
    let p = plane(&pool).await;
    let node = make_node(&p.ca, &pool, "eta", true).await;

    // A raw session, to read the reason it is closed with.
    let mut ws = raw_connect(&node, p.addr, PROTOCOL_VERSION).await.unwrap();
    read_welcome(&mut ws).await.unwrap();
    assert!(wait_for_status(&pool, node.id, "online", Duration::from_secs(3)).await);

    // Revoke exactly like the admin endpoint does: database first, then the open connection.
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        nodes::revoke_node(&mut tx, node.id).await.unwrap(),
        nodes::RevokeNodeOutcome::Revoked { certificates: 1 }
    );
    tx.commit().await.unwrap();
    assert!(p.gateway.disconnect_revoked(node.id));
    let mut reason = None;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_millis(500), ws.next()).await {
            Ok(Some(Ok(Message::Close(Some(frame))))) => {
                reason = Some(frame.reason.to_string());
                break;
            }
            Ok(None | Some(Err(_))) => break,
            _ => {}
        }
    }
    assert_eq!(reason.as_deref(), Some("node revoked"));
    assert!(!p.gateway.disconnect_revoked(node.id), "no session left");
    // The status stays revoked: the goodbye must not turn it into plain offline.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(status(&pool, node.id).await, "revoked");

    // The real agent keeps trying and is never let in.
    let (stop, task) = start_agent(node.params(p.addr));
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(status(&pool, node.id).await, "revoked");
    assert!(!p.gateway.is_connected(node.id));
    stop.send(true).unwrap();
    task.await.unwrap();
}
