//! Operations from the ledger to real agents and back: real TLS, the real
//! gateway, the real agent connection code, real PostgreSQL.
//! Ignored by default; run with DATABASE_URL (superuser) and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use builtpanel_database::nodes::{self, NewNode, NewNodeCertificate};
use builtpanel_dispatcher::{Dispatcher, Settings};
use builtpanel_node_agent::{
    ops::{Executor, Registry, cache::ResultCache},
    stream::{self, Params},
};
use builtpanel_node_gateway::{Gateway, NodeResult, Settings as GatewaySettings, tls};
use builtpanel_node_protocol::{
    AgentError, ErrorCode, Operation, OperationResult, Output, ResultStatus,
};
use builtpanel_operations::{Operations, Request, Row, Status};
use builtpanel_pki::CertificateAuthority;
use rcgen::{CertificateParams, KeyPair};
use secrecy::{ExposeSecret as _, SecretString};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::watch;
use uuid::Uuid;

struct Plane {
    ca: Arc<CertificateAuthority>,
    gateway: Gateway,
    addr: SocketAddr,
    ops: Operations,
    stop: watch::Sender<bool>,
}

fn dispatch_settings() -> Settings {
    Settings {
        poll: Duration::from_millis(50),
        batch: 50,
        operation_timeout: Duration::from_secs(1),
        retry_base: Duration::from_secs(1),
        retry_cap: Duration::from_secs(2),
        capabilities_check: Duration::from_millis(200),
        capabilities_max_age: Duration::from_secs(3600),
        purge_check: Duration::from_secs(3600),
        route_check: Duration::from_secs(3600),
        certificates_every: Duration::from_secs(3600),
        expiry_check: Duration::from_secs(3600),
        expiry_warning_days: 14,
    }
}

async fn plane(pool: &PgPool) -> Plane {
    let ca = Arc::new(
        CertificateAuthority::create(
            "Test CA",
            OffsetDateTime::now_utc(),
            time::Duration::days(3650),
        )
        .unwrap()
        .0,
    );
    let (server, key) = ca
        .issue_server_certificate(
            &["127.0.0.1".to_owned()],
            OffsetDateTime::now_utc(),
            time::Duration::days(30),
        )
        .unwrap();
    let config = tls::server_config(
        ca.certificate_pem(),
        server.cert_pem.as_bytes(),
        key.expose_secret().as_bytes(),
    )
    .unwrap();
    let gateway = Gateway::new(
        pool.clone(),
        config,
        GatewaySettings {
            ping_interval: Duration::from_millis(200),
            liveness_timeout: Duration::from_secs(30),
            handshake_timeout: Duration::from_secs(5),
            max_connections: 64,
        },
    );
    let listener = builtpanel_node_gateway::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = watch::channel(false);
    tokio::spawn(gateway.clone().serve(listener, stopped));
    Plane {
        ca,
        gateway,
        addr,
        ops: Operations::new(pool.clone()),
        stop,
    }
}

impl Plane {
    /// Runs the dispatcher the way control-api does.
    fn start_dispatcher(&self) -> Dispatcher {
        let d = Dispatcher::new(
            self.ops.clone(),
            self.gateway.clone(),
            None,
            dispatch_settings(),
        );
        let results = self.gateway.take_results().expect("results taken once");
        tokio::spawn(d.clone().run(results, self.stop.subscribe()));
        d
    }
}

struct TestNode {
    id: Uuid,
    ca_pem: String,
    cert_pem: String,
    key_pem: String,
    /// The agent's operation handlers and memory; kept across agent restarts.
    executor: Arc<Executor>,
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

    fn start_agent(&self, addr: SocketAddr) -> (watch::Sender<bool>, tokio::task::JoinHandle<()>) {
        let (stop, stopped) = watch::channel(false);
        let params = self.params(addr);
        let executor = self.executor.clone();
        let task =
            tokio::spawn(async move { stream::run(params, executor, stopped).await.unwrap() });
        (stop, task)
    }
}

async fn make_node(p: &Plane, pool: &PgPool, name: &str) -> TestNode {
    make_node_with(p, pool, name, Registry::builtin(&std::env::temp_dir())).await
}

async fn make_node_with(p: &Plane, pool: &PgPool, name: &str, registry: Registry) -> TestNode {
    let id = Uuid::now_v7();
    let key = KeyPair::generate().unwrap();
    let csr = CertificateParams::new(Vec::<String>::new())
        .unwrap()
        .serialize_request(&key)
        .and_then(|c| c.pem())
        .unwrap();
    let issued =
        p.ca.issue_node_certificate(
            &csr,
            id,
            OffsetDateTime::now_utc(),
            time::Duration::days(90),
        )
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    nodes::insert_node(
        &mut conn,
        &NewNode {
            id,
            name,
            hostname: None,
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
    TestNode {
        id,
        ca_pem: p.ca.certificate_pem().to_owned(),
        cert_pem: issued.cert_pem,
        key_pem: key.serialize_pem(),
        executor: Executor::new(registry, ResultCache::in_memory().unwrap(), 8),
    }
}

async fn wait_connected(p: &Plane, node: Uuid) {
    for _ in 0..100 {
        if p.gateway.is_connected(node) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("node never connected");
}

async fn wait_status(ops: &Operations, id: Uuid, wanted: Status, within: Duration) -> Row {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let row = ops.get(id).await.unwrap().unwrap();
        if row.status == wanted {
            return row;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "operation stayed {:?}, wanted {wanted:?}: {row:?}",
            row.status
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_ping_goes_to_the_agent_and_comes_back_succeeded(pool: PgPool) {
    let p = plane(&pool).await;
    p.start_dispatcher();
    let node = make_node(&p, &pool, "alpha").await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;

    let queued = p
        .ops
        .enqueue(&Request::new(
            node.id,
            Operation::Ping { nonce: 4242 },
            "ping-1",
        ))
        .await
        .unwrap();
    let id = queued.operation().id;
    let row = wait_status(&p.ops, id, Status::Succeeded, Duration::from_secs(5)).await;
    assert_eq!(row.attempts, 1);
    assert_eq!(
        row.output,
        Some(serde_json::json!({"type": "pong", "nonce": 4242}))
    );
    assert!(row.finished_at.is_some() && row.dispatched_at.is_some());

    // Asking again with the same key does not send a second ping.
    let again = p
        .ops
        .enqueue(&Request::new(
            node.id,
            Operation::Ping { nonce: 4242 },
            "ping-1",
        ))
        .await
        .unwrap();
    assert_eq!(again.operation().id, id);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(p.ops.get(id).await.unwrap().unwrap().attempts, 1);

    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_operation_for_an_offline_node_waits_and_is_sent_when_it_connects(pool: PgPool) {
    let p = plane(&pool).await;
    p.start_dispatcher();
    let node = make_node(&p, &pool, "beta").await;
    let id = p
        .ops
        .enqueue(&Request::new(
            node.id,
            Operation::Ping { nonce: 1 },
            "waiting",
        ))
        .await
        .unwrap()
        .operation()
        .id;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let row = p.ops.get(id).await.unwrap().unwrap();
    assert_eq!(
        (row.status, row.attempts),
        (Status::Pending, 0),
        "nothing is sent to nobody"
    );

    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_status(&p.ops, id, Status::Succeeded, Duration::from_secs(8)).await;
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_operation_the_agent_cannot_run_fails_for_good_without_retries(pool: PgPool) {
    let p = plane(&pool).await;
    p.start_dispatcher();
    // An agent build without the capability handler (like an older agent).
    let mut registry = Registry::new();
    registry.register("ping", |_| async { Ok(Output::Pong { nonce: 0 }) });
    let node = make_node_with(&p, &pool, "gamma", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let id = p
        .ops
        .enqueue(&Request::new(
            node.id,
            Operation::ReportCapabilities,
            "caps",
        ))
        .await
        .unwrap()
        .operation()
        .id;
    let row = wait_status(&p.ops, id, Status::Failed, Duration::from_secs(5)).await;
    assert_eq!(row.attempts, 1);
    assert_eq!(row.error.unwrap()["code"], "unsupported_operation");
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn results_are_only_accepted_from_the_node_the_operation_went_to(pool: PgPool) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let (a, b) = (
        make_node(&p, &pool, "node-a").await,
        make_node(&p, &pool, "node-b").await,
    );
    let id = p
        .ops
        .enqueue(&Request::new(a.id, Operation::Ping { nonce: 1 }, "k"))
        .await
        .unwrap()
        .operation()
        .id;
    assert_eq!(p.ops.claim_due(&[a.id], 10).await.unwrap().len(), 1);

    // B claims to have finished A's operation: ignored.
    d.handle_result(NodeResult {
        node_id: b.id,
        result: OperationResult::success(id, Output::Pong { nonce: 1 }),
    })
    .await;
    assert_eq!(
        p.ops.get(id).await.unwrap().unwrap().status,
        Status::Dispatched
    );
    // An id nobody knows is ignored too.
    d.handle_result(NodeResult {
        node_id: a.id,
        result: OperationResult::success(Uuid::now_v7(), Output::Pong { nonce: 1 }),
    })
    .await;

    // A's own answer counts, once.
    d.handle_result(NodeResult {
        node_id: a.id,
        result: OperationResult::success(id, Output::Pong { nonce: 1 }),
    })
    .await;
    assert_eq!(
        p.ops.get(id).await.unwrap().unwrap().status,
        Status::Succeeded
    );
    d.handle_result(NodeResult {
        node_id: a.id,
        result: OperationResult::failure(id, AgentError::new(ErrorCode::Internal, "late")),
    })
    .await;
    let row = p.ops.get(id).await.unwrap().unwrap();
    assert_eq!(row.status, Status::Succeeded);
    assert!(row.error.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn failed_and_retryable_results_follow_the_retry_policy(pool: PgPool) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let n = make_node(&p, &pool, "node-a").await;
    let mut request = Request::new(n.id, Operation::Ping { nonce: 1 }, "k");
    request.max_attempts = 2;
    let id = p.ops.enqueue(&request).await.unwrap().operation().id;
    let report = |status_error: AgentError| NodeResult {
        node_id: n.id,
        result: OperationResult::failure(id, status_error),
    };

    p.ops.claim_due(&[n.id], 10).await.unwrap();
    d.handle_result(report(
        AgentError::new(ErrorCode::Internal, "disk busy").retryable(),
    ))
    .await;
    let row = p.ops.get(id).await.unwrap().unwrap();
    assert_eq!(row.status, Status::Retryable);
    assert_eq!(row.error.as_ref().unwrap()["message"], "disk busy");
    assert!(
        row.next_attempt_at > row.created_at,
        "waits before the next try"
    );

    // Second attempt fails retryably again, but attempts are used up.
    sqlx::query("UPDATE operations SET next_attempt_at = now() WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    p.ops.claim_due(&[n.id], 10).await.unwrap();
    d.handle_result(report(
        AgentError::new(ErrorCode::Internal, "still busy").retryable(),
    ))
    .await;
    assert_eq!(p.ops.get(id).await.unwrap().unwrap().status, Status::Failed);

    // A permanent failure is final at once.
    let id2 = p
        .ops
        .enqueue(&Request::new(n.id, Operation::Ping { nonce: 2 }, "k2"))
        .await
        .unwrap()
        .operation()
        .id;
    p.ops.claim_due(&[n.id], 10).await.unwrap();
    d.handle_result(NodeResult {
        node_id: n.id,
        result: OperationResult {
            op_id: id2,
            status: ResultStatus::Failed,
            output: None,
            error: Some(AgentError::new(ErrorCode::InvalidArguments, "no")),
        },
    })
    .await;
    assert_eq!(
        p.ops.get(id2).await.unwrap().unwrap().status,
        Status::Failed
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_operation_a_node_never_answers_is_retried_and_then_fails_with_a_timeout(pool: PgPool) {
    let p = plane(&pool).await;
    p.start_dispatcher();
    let node = make_node(&p, &pool, "delta").await;
    // Connected, but silent about operations: a connection that is held open and never read.
    let params = node.params(p.addr);
    let config = stream::client_config(&params).unwrap();
    let tcp = tokio::net::TcpStream::connect(p.addr).await.unwrap();
    let tls = tokio_rustls::TlsConnector::from(config)
        .connect(
            rustls::pki_types::ServerName::try_from("127.0.0.1".to_owned()).unwrap(),
            tcp,
        )
        .await
        .unwrap();
    let (mut ws, _) = tokio_tungstenite::client_async(
        format!("wss://{}{}", p.addr, builtpanel_node_protocol::STREAM_PATH),
        tls,
    )
    .await
    .unwrap();
    use futures_util::SinkExt as _;
    let hello = builtpanel_node_protocol::encode(&builtpanel_node_protocol::AgentMessage::Hello(
        builtpanel_node_protocol::Hello {
            protocol_version: 1,
            agent_version: "mute".into(),
        },
    ))
    .unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(hello.into()))
        .await
        .unwrap();
    wait_connected(&p, node.id).await;

    let mut request = Request::new(node.id, Operation::Ping { nonce: 1 }, "mute");
    request.max_attempts = 2;
    let id = p.ops.enqueue(&request).await.unwrap().operation().id;
    let row = wait_status(&p.ops, id, Status::Failed, Duration::from_secs(10)).await;
    assert_eq!(row.attempts, 2, "tried twice, never answered");
    assert_eq!(row.error.unwrap()["code"], "timeout");
    drop(ws);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_slow_operation_that_is_sent_again_still_runs_only_once(pool: PgPool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let p = plane(&pool).await;
    p.start_dispatcher();
    // A handler slower than the dispatcher's patience (1 s in these tests).
    let runs = Arc::new(AtomicUsize::new(0));
    let mut registry = Registry::new();
    let counter = runs.clone();
    registry.register("ping", move |op| {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(2500)).await;
            match op {
                Operation::Ping { nonce } => Ok(Output::Pong { nonce }),
                _ => Err(builtpanel_node_agent::ops::wrong_operation()),
            }
        }
    });
    let node = make_node_with(&p, &pool, "slow", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;

    let id = p
        .ops
        .enqueue(&Request::new(
            node.id,
            Operation::Ping { nonce: 77 },
            "slow-ping",
        ))
        .await
        .unwrap()
        .operation()
        .id;
    // The control plane gives up waiting, sends it again, and the first run's
    // answer then completes the operation.
    let row = wait_status(&p.ops, id, Status::Succeeded, Duration::from_secs(10)).await;
    assert!(row.attempts >= 2, "it was sent again: {row:?}");
    assert_eq!(
        row.output,
        Some(serde_json::json!({"type": "pong", "nonce": 77}))
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1, "sent twice, run once");

    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_connected_node_is_asked_for_its_capabilities_and_they_are_stored(pool: PgPool) {
    let p = plane(&pool).await;
    p.start_dispatcher();
    let node = make_node(&p, &pool, "caps").await;
    let before = nodes::list_nodes(&pool, 10, 60).await.unwrap();
    assert_eq!(before[0].capabilities, serde_json::json!({}));
    assert!(before[0].capabilities_at.is_none());

    let (agent_stop, agent) = node.start_agent(p.addr);
    // Nobody asks by hand: connecting is enough.
    let mut stored = None;
    for _ in 0..100 {
        let listed = nodes::list_nodes(&pool, 10, 60).await.unwrap();
        if listed[0].capabilities_at.is_some() {
            stored = Some(listed[0].clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let stored = stored.expect("capabilities stored within 10 s");
    let caps = &stored.capabilities;
    assert_eq!(caps["agent_version"], builtpanel_node_agent::AGENT_VERSION);
    assert!(caps["cpu_cores"].as_u64().unwrap() >= 1);
    assert!(caps["memory_bytes"].as_u64().unwrap() > 0);
    assert!(
        caps["disk_total_bytes"].as_u64().unwrap() >= caps["disk_free_bytes"].as_u64().unwrap()
    );
    assert!(!caps["kernel"].as_str().unwrap().is_empty());
    assert!(caps["cgroup_v2"].is_boolean() && caps["seccomp"].is_boolean());

    // Asked once per time slot, however often the check runs.
    tokio::time::sleep(Duration::from_millis(900)).await;
    let asked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE node_id = $1 AND kind = 'report_capabilities'",
    )
    .bind(node.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(asked, 1);

    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn data_of_deleted_tenants_is_purged_when_due_and_the_tenant_goes_when_the_node_says_so(
    pool: PgPool,
) {
    let p = plane(&pool).await;
    // Not running: the test asks for purges itself, at the moment it wants to.
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let node = make_node(&p, &pool, "purger").await;
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'zed', 'zed@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant = |name: &'static str, uid: i32, days: i32| {
        let (pool, node_id) = (pool.clone(), node.id);
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status, deleted_at, purge_after)
                 VALUES ($1, $2, $3, $4, $5, 'deleting', now() - interval '10 days', now() + make_interval(days => $6)) RETURNING id",
            )
            .bind(account)
            .bind(node_id)
            .bind(plan)
            .bind(name)
            .bind(uid)
            .bind(days)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let due = tenant("duetenant", 10_000, -1).await;
    let later = tenant("latertenant", 10_001, 3).await;

    // Only the one whose time has come is asked for; asking again changes nothing.
    assert_eq!(d.request_purges().await, 1);
    assert_eq!(d.request_purges().await, 0);
    let rows: Vec<(Option<Uuid>, String, serde_json::Value)> = sqlx::query_as(
        "SELECT tenant_id, kind, payload FROM operations WHERE kind = 'delete_tenant'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, Some(due));
    assert_eq!(rows[0].2["purge_data"], true);
    assert_eq!(rows[0].2["username"], "duetenant");
    assert_ne!(rows[0].0, Some(later));

    // The node's success removes the tenant row; the other one stays.
    let op: Uuid = sqlx::query_scalar("SELECT id FROM operations WHERE kind = 'delete_tenant'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        builtpanel_database::operations::succeed(&pool, op, &serde_json::json!({}))
            .await
            .unwrap()
    );
    let left: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM tenants ORDER BY username")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, [later]);
    assert_eq!(d.request_purges().await, 0, "nothing more is due");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_tenants_routes_are_sent_when_anything_they_are_made_of_changes(pool: PgPool) {
    use builtpanel_node_protocol::Operation as Op;
    let p = plane(&pool).await;
    // Not running: the test asks for the sync itself, at the moment it wants to.
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let node = make_node(&p, &pool, "web").await;
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'zed', 'zed@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'zedhost', 10000, 'provisioning') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let ops = |pool: PgPool| async move {
        sqlx::query_as::<_, (Uuid, serde_json::Value, String)>(
            "SELECT id, payload, status FROM operations WHERE kind = 'set_site_routes' ORDER BY created_at, id",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
    };

    // A new tenant is pending; while it is being set up it has no routes, but the
    // empty list is still sent (the agent removes what it may hold from before).
    assert_eq!(d.request_route_syncs().await, 1);
    let sent = ops(pool.clone()).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1["username"], "zedhost");
    assert_eq!(sent[0].1["routes"], serde_json::json!([]));
    assert_eq!(
        d.request_route_syncs().await,
        0,
        "nothing changed: nothing sent"
    );

    // Active, with a domain, an alias and a redirect: one site, both names.
    sqlx::query("UPDATE tenants SET status = 'active' WHERE id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    let main: Uuid = sqlx::query_scalar("INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, 'example.com', 'primary') RETURNING id")
        .bind(tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO domains (tenant_id, fqdn, kind, parent_domain_id) VALUES ($1, 'example.net', 'alias', $2)")
        .bind(tenant)
        .bind(main)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO redirects (domain_id, source_path, target) VALUES ($1, '/old', '/new')",
    )
    .bind(main)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        d.request_route_syncs().await,
        1,
        "several changes, one operation"
    );
    let sent = ops(pool.clone()).await;
    assert_eq!(sent.len(), 2);
    let routes = &sent[1].1["routes"];
    assert_eq!(
        routes[0]["hosts"],
        serde_json::json!(["example.com", "example.net"])
    );
    assert_eq!(routes[0]["upstream"], "bp-zedhost-web:8080");
    assert_eq!(routes[0]["redirects"][0]["path"], "/old");
    assert_eq!(routes[0]["redirects"][0]["code"], 301);

    // A domain suspended by the administrator, then the whole tenant: the page, not the files.
    sqlx::query("UPDATE domains SET status = 'suspended', suspended_at = now() WHERE id = $1")
        .bind(main)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 1);
    let sent = ops(pool.clone()).await;
    assert!(
        sent[2].1["routes"][0].get("upstream").is_none()
            || sent[2].1["routes"][0]["upstream"].is_null()
    );
    sqlx::query("UPDATE domains SET status = 'active', suspended_at = NULL WHERE id = $1")
        .bind(main)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tenants SET status = 'suspended' WHERE id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 1);
    assert!(
        ops(pool.clone()).await[3].1["routes"][0]
            .get("upstream")
            .is_none()
    );

    // A delete of the domain: the empty list again.
    sqlx::query("DELETE FROM domains WHERE tenant_id = $1 AND kind = 'alias'")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM domains WHERE id = $1")
        .bind(main)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 1);
    assert_eq!(
        ops(pool.clone()).await[4].1["routes"],
        serde_json::json!([])
    );

    // An operation that fails for good puts the tenant back on the list; it is asked
    // again in the next five-minute slot, not at once.
    let last = ops(pool.clone()).await.last().unwrap().0;
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(last)
        .execute(&pool)
        .await
        .unwrap();
    builtpanel_database::operations::fail(&pool, last, &serde_json::json!({"code": "boom"}), None)
        .await
        .unwrap();
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenants WHERE routes_version <> routes_synced_version",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 1, "the failure queued the work again");
    assert_eq!(
        d.request_route_syncs().await,
        0,
        "in this slot the failed request is not repeated"
    );
    let _ = Op::ReportCapabilities;
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn reported_certificates_are_kept_and_an_ending_one_is_announced_once(pool: PgPool) {
    use builtpanel_node_protocol::{CertificateInfo, Output};
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let node = make_node(&p, &pool, "certs").await;
    let other = make_node(&p, &pool, "certs-other").await;
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'zed', 'zed@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'zedhost', 10000, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    for name in ["soon.com", "later.com", "custom.com"] {
        sqlx::query("INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, $2, 'addon')")
            .bind(tenant)
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let day = 86_400;
    let cert = |host: &str, ends_in_days: i64| CertificateInfo {
        host: host.into(),
        issuer: "R11".into(),
        not_before: now - 30 * day,
        not_after: now + ends_in_days * day,
    };
    let ops = p.ops.clone();
    let report = |node_id: Uuid, items: Vec<CertificateInfo>| {
        let (d, ops) = (d.clone(), ops.clone());
        async move {
            let op = p_enqueue(&ops, node_id).await;
            d.handle_result(NodeResult {
                node_id,
                result: builtpanel_node_protocol::OperationResult {
                    op_id: op,
                    status: builtpanel_node_protocol::ResultStatus::Succeeded,
                    output: Some(Output::Certificates { items }),
                    error: None,
                },
            })
            .await;
        }
    };
    async fn p_enqueue(ops: &Operations, node: Uuid) -> Uuid {
        let r = Request::new(
            node,
            builtpanel_node_protocol::Operation::ReportCertificates,
            format!("c-{}", Uuid::now_v7().simple()),
        );
        let row = ops.enqueue(&r).await.unwrap();
        let id = row.operation().id;
        sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
            .bind(id)
            .execute(ops.db())
            .await
            .unwrap();
        id
    }
    report(
        node.id,
        vec![
            cert("soon.com", 5),
            cert("later.com", 60),
            cert("unknown.org", 30),
            cert("custom.com", 90),
        ],
    )
    .await;

    // Only names that are domains of tenants on that node are kept.
    let kept: Vec<(String, String)> = sqlx::query_as(
        "SELECT d.fqdn, c.issuer FROM certificates c JOIN domains d ON d.id = c.domain_id ORDER BY d.fqdn",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        kept.iter().map(|k| k.0.as_str()).collect::<Vec<_>>(),
        ["custom.com", "later.com", "soon.com"]
    );
    // A report from a node that does not host them changes nothing.
    report(other.id, vec![cert("soon.com", 400)]).await;
    let days: f64 = sqlx::query_scalar("SELECT (extract(epoch FROM c.not_after - now()) / 86400)::float8 FROM certificates c JOIN domains d ON d.id = c.domain_id WHERE d.fqdn = 'soon.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!((4.0..6.0).contains(&days), "{days}");

    // A custom certificate is never replaced by a report.
    sqlx::query("UPDATE certificates SET kind = 'custom', issuer = 'My CA', chain_pem = 'x', private_key_enc = '\\x00'::bytea WHERE domain_id = (SELECT id FROM domains WHERE fqdn = 'custom.com')")
        .execute(&pool)
        .await
        .unwrap();
    report(node.id, vec![cert("custom.com", 10)]).await;
    let issuer: String = sqlx::query_scalar("SELECT issuer FROM certificates c JOIN domains d ON d.id = c.domain_id WHERE d.fqdn = 'custom.com'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(issuer, "My CA");

    // The one ending in 5 days is announced, once, and only that one.
    assert_eq!(d.announce_expiring().await, 1);
    assert_eq!(d.announce_expiring().await, 0, "not twice");
    let events: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT payload FROM outbox WHERE event_type = 'certificate_expiring'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["name"], "soon.com");
    assert_eq!(events[0]["tenant_id"], tenant.to_string());

    // The certificate is renewed: it is announced again when that one nears its end.
    report(node.id, vec![cert("soon.com", 3)]).await;
    assert_eq!(
        d.announce_expiring().await,
        1,
        "another end date, another notice"
    );
    report(node.id, vec![cert("soon.com", 3)]).await;
    assert_eq!(
        d.announce_expiring().await,
        0,
        "the same certificate reported again is not news"
    );

    // Tenants that are on their way out are not told.
    sqlx::query("UPDATE tenants SET status = 'deleting', deleted_at = now(), purge_after = now() + interval '7 days' WHERE id = $1").bind(tenant).execute(&pool).await.unwrap();
    sqlx::query("UPDATE certificates SET expiry_notified_at = NULL")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.announce_expiring().await, 0);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_uploaded_certificate_reaches_the_node_with_its_key_and_the_ledger_never_has_it(
    pool: PgPool,
) {
    use std::sync::Mutex;
    let p = plane(&pool).await;
    // The hook plays the sealed-key lookup: whatever it is given gets this key.
    let filler: builtpanel_dispatcher::SecretFiller = Arc::new(|op| {
        Box::pin(async move {
            match op {
                Operation::SetCustomCertificates {
                    username,
                    mut items,
                } => {
                    for item in &mut items {
                        item.private_key = Some("KEY-OPENED-AT-SEND-TIME".to_owned());
                    }
                    Ok(Operation::SetCustomCertificates { username, items })
                }
                other => Ok(other),
            }
        })
    });
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings())
        .with_secrets(filler);
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));

    let got = Arc::new(Mutex::new(Vec::new()));
    let mut registry = Registry::new();
    let seen = got.clone();
    registry.register("set_custom_certificates", move |op| {
        let seen = seen.clone();
        async move {
            match op {
                Operation::SetCustomCertificates { items, .. } => {
                    let count = items.len();
                    seen.lock().unwrap().extend(items);
                    Ok(Output::CustomCertificatesSet {
                        count,
                        applied: true,
                    })
                }
                _ => Err(builtpanel_node_agent::ops::wrong_operation()),
            }
        }
    });
    registry.register("attach_domain", |_| async move {
        Ok(Output::DomainAttached { applied: true })
    });
    registry.register("set_php_ini", |_| async move {
        Ok(Output::PhpIniSet { reloaded: 0 })
    });
    registry.register("keep_domains", |_| async move {
        Ok(Output::DomainsKept {
            removed: 0,
            applied: true,
        })
    });
    registry.register("set_site_routes", |op| async move {
        match op {
            Operation::SetSiteRoutes { routes, .. } => Ok(Output::SiteRoutesSet {
                sites: routes.len(),
                hosts: 0,
                applied: true,
            }),
            _ => Err(builtpanel_node_agent::ops::wrong_operation()),
        }
    });
    let node = make_node_with(&p, &pool, "certs", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;

    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'cora', 'cora@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'corahost', 10001, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let domain: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, 'cora.example.com', 'primary') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    // The first pass of the running dispatcher may have taken the new tenant already:
    // either way, everything so far is synced after this. The certificate alone then
    // makes the tenant pending again.
    let _ = d.request_route_syncs().await;
    sqlx::query("UPDATE tenants SET routes_synced_version = routes_version WHERE id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 0);
    let mut conn = pool.acquire().await.unwrap();
    builtpanel_database::certificates::set_custom(
        &mut conn,
        &builtpanel_database::certificates::NewCustom {
            domain_id: domain,
            issuer: "Test CA",
            not_before: 1_700_000_000,
            not_after: 4_000_000_000,
            chain_pem: "CHAIN-PEM",
            private_key_enc: b"sealed-bytes",
            uploaded_by: account,
        },
    )
    .await
    .unwrap();
    drop(conn);
    assert_eq!(
        d.request_route_syncs().await,
        1,
        "an uploaded certificate is a change of the tenant's routes"
    );

    let mut waited = 0;
    while got.lock().unwrap().is_empty() && waited < 100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        waited += 1;
    }
    {
        let items = got.lock().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].host, "cora.example.com");
        assert_eq!(items[0].certificate, "CHAIN-PEM");
        assert_eq!(
            items[0].private_key.as_deref(),
            Some("KEY-OPENED-AT-SEND-TIME")
        );
    }
    // The ledger has the chain and never the key.
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'set_custom_certificates' AND payload @> '{\"username\":\"corahost\"}'
         ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payload["items"][0]["certificate"], "CHAIN-PEM");
    assert!(
        payload["items"][0].get("private_key").is_none(),
        "{payload}"
    );
    assert!(!payload.to_string().contains("KEY-OPENED"));

    // Removing the certificate makes the tenant pending again, with an empty list.
    sqlx::query("UPDATE tenants SET routes_synced_version = routes_version WHERE id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    assert!(
        builtpanel_database::certificates::remove_custom(&mut conn, domain)
            .await
            .unwrap()
    );
    drop(conn);
    assert_eq!(d.request_route_syncs().await, 1);

    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_tenants_web_server_files_are_sent_with_its_routes_and_what_is_gone_is_pruned(
    pool: PgPool,
) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let node = make_node(&p, &pool, "vhosts").await;
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'vera', 'vera@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'verahost', 10002, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let main: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind, docroot) VALUES ($1, 'vera.example.com', 'primary', 'public_html') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO domains (tenant_id, fqdn, kind, parent_domain_id) VALUES ($1, 'www.vera.example.com', 'alias', $2)")
        .bind(tenant)
        .bind(main)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO domains (tenant_id, fqdn, kind, docroot) VALUES ($1, 'shop.example.org', 'addon', 'shop')")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 1);
    let attached: Vec<(String, String)> = sqlx::query_as(
        "SELECT payload ->> 'domain', payload ->> 'config' FROM operations
         WHERE kind = 'attach_domain' ORDER BY payload ->> 'domain'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let names: Vec<&str> = attached.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["shop.example.org", "vera.example.com"]);
    assert!(
        attached[1]
            .1
            .contains("server_name vera.example.com www.vera.example.com;")
    );
    assert!(attached[1].1.contains("root /var/www/html/public_html;"));
    assert!(attached[1].1.contains("fastcgi_pass bp-verahost-php:9000;"));
    let keep: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'keep_domains' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        keep["domains"],
        serde_json::json!(["shop.example.org", "vera.example.com"])
    );

    // The account's php.ini options go with them: none chosen, none sent.
    let ini: String = sqlx::query_scalar(
        "SELECT payload ->> 'ini' FROM operations WHERE kind = 'set_php_ini' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ini, "");
    sqlx::query("INSERT INTO php_settings (tenant_id, options) VALUES ($1, $2)")
        .bind(tenant)
        .bind(serde_json::json!({"memory_limit": "512M", "max_execution_time": "60", "display_errors": "On"}))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 1);
    let ini: String = sqlx::query_scalar(
        "SELECT payload ->> 'ini' FROM operations WHERE kind = 'set_php_ini' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    // 512M is above what the default plan allows (256M): the node gets the ceiling.
    assert!(ini.contains("memory_limit = 256M\n"), "{ini}");
    assert!(ini.contains("max_execution_time = 60\n") && ini.contains("display_errors = On\n"));
    sqlx::query("DELETE FROM php_settings WHERE tenant_id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 1);

    // A domain deleted: the next keep list no longer has it.
    sqlx::query("DELETE FROM domains WHERE fqdn = 'shop.example.org'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 1);
    let keep: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'keep_domains' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(keep["domains"], serde_json::json!(["vera.example.com"]));

    // A configuration the web server refused puts the tenant on the list again,
    // and does not stop the routes from being sent.
    sqlx::query(
        "UPDATE operations SET status = 'dispatched', attempts = 1 WHERE kind = 'attach_domain'
           AND payload ->> 'domain' = 'vera.example.com'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let op: Uuid = sqlx::query_scalar(
        "SELECT id FROM operations WHERE kind = 'attach_domain' AND status = 'dispatched'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        builtpanel_database::operations::fail(
            &pool,
            op,
            &serde_json::json!({"code": "invalid_arguments", "message": "refused", "retryable": false}),
            None,
        )
        .await
        .unwrap(),
        Some(builtpanel_operations::Status::Failed)
    );
    assert_eq!(
        d.request_route_syncs().await,
        1,
        "a failed web server file means the tenant is looked at again"
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn php_containers_are_set_before_the_files_that_point_to_them(pool: PgPool) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let node = make_node(&p, &pool, "php").await;
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'phil', 'phil@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'philhost', 10003, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let main: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, 'phil.example.com', 'primary') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO domains (tenant_id, fqdn, kind, php_version) VALUES ($1, 'old.example.org', 'addon', '8.2')")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    // An account that runs the version it started with needs no PHP step; with a
    // domain on another version it does: the PHP operation goes out, the files wait.
    let php_ops = || async {
        sqlx::query_as::<_, (Uuid, serde_json::Value, String, String)>(
            "SELECT id, payload, status, idempotency_key FROM operations WHERE kind = 'set_php_versions' ORDER BY created_at",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
    };
    let attached = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM operations WHERE kind = 'attach_domain'")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    assert_eq!(d.request_route_syncs().await, 0, "waiting for PHP");
    let sent = php_ops().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1["php_version"], "8.4");
    assert_eq!(sent[0].1["extra_php"], serde_json::json!(["8.2"]));
    assert_eq!(sent[0].1["username"], "philhost");
    assert!(sent[0].3.starts_with(&format!("php:{}:", tenant.simple())));
    assert_eq!(
        attached().await,
        0,
        "no file points at a container that is not there yet"
    );
    // Asking again before the node answers sends nothing new and still waits.
    assert_eq!(d.request_route_syncs().await, 0);
    assert_eq!(php_ops().await.len(), 1);

    // The node says done: the account is up to date and the files go out.
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1 WHERE id = $1")
        .bind(sent[0].0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        builtpanel_database::operations::succeed(&pool, sent[0].0, &serde_json::json!({}))
            .await
            .unwrap()
    );
    let state = builtpanel_database::php::state(&pool, tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.state_version, state.applied_version);
    assert_eq!(d.request_route_syncs().await, 1);
    let files: Vec<(String, String)> = sqlx::query_as(
        "SELECT payload ->> 'domain', payload ->> 'config' FROM operations
         WHERE kind = 'attach_domain' ORDER BY payload ->> 'domain'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(files.len(), 2);
    assert!(
        files[0]
            .1
            .contains("fastcgi_pass bp-philhost-php-8-2:9000;"),
        "{}",
        files[0].1
    );
    assert!(
        files[1].1.contains("fastcgi_pass bp-philhost-php:9000;"),
        "{}",
        files[1].1
    );

    // The account moves to 8.2: the domain that asked for it follows the main
    // container, so no extra remains; the PHP step comes first again.
    sqlx::query("UPDATE tenants SET php_version = '8.2' WHERE id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 0);
    let sent = php_ops().await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].1["php_version"], "8.2");
    assert_eq!(sent[1].1["extra_php"], serde_json::json!([]));
    let _ = main;

    // A suspended account has nothing running: it does not wait for PHP.
    sqlx::query("UPDATE tenants SET status = 'suspended' WHERE id = $1")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_route_syncs().await, 1);
    assert_eq!(
        php_ops().await.len(),
        2,
        "nothing more is asked of a suspended account"
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn error_pages_go_to_every_connected_node_once_per_version(pool: PgPool) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let node = make_node(&p, &pool, "pages").await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    // Nothing set: the node is still told once (an empty list), then left alone.
    assert_eq!(d.request_error_pages().await, 1);
    assert_eq!(d.request_error_pages().await, 0);
    let mut conn = pool.acquire().await.unwrap();
    builtpanel_database::error_pages::set(&mut conn, "not_found", "<p>gone</p>", None)
        .await
        .unwrap();
    drop(conn);
    assert_eq!(
        d.request_error_pages().await,
        1,
        "a change is a new version"
    );
    assert_eq!(d.request_error_pages().await, 0);
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'set_error_pages' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payload["pages"][0]["kind"], "not_found");
    assert_eq!(payload["pages"][0]["html"], "<p>gone</p>");
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn database_sizes_are_asked_for_only_where_there_are_databases_and_kept(pool: PgPool) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let mut registry = Registry::new();
    registry.register("report_db_sizes", |_| async move {
        Ok(Output::DbSizes {
            items: vec![
                builtpanel_node_protocol::DbSize {
                    name: "dina_shop".into(),
                    bytes: 123_456,
                },
                builtpanel_node_protocol::DbSize {
                    name: "someone_elses".into(),
                    bytes: 9,
                },
            ],
        })
    });
    let node = make_node_with(&p, &pool, "sizes", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));

    // No databases on the node: nobody is asked.
    assert_eq!(d.request_db_sizes().await, 0);
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'dina', 'dina@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'dina', 10004, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO databases (tenant_id, name, status) VALUES ($1, 'dina_shop', 'ready')",
    )
    .bind(tenant)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(d.request_db_sizes().await, 1);
    assert_eq!(d.request_db_sizes().await, 0, "once per time slot");
    let mut size = 0_i64;
    for _ in 0..100 {
        size = sqlx::query_scalar("SELECT size_bytes FROM databases WHERE name = 'dina_shop'")
            .fetch_one(&pool)
            .await
            .unwrap();
        if size > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(size, 123_456);
    let checked: bool = sqlx::query_scalar(
        "SELECT size_checked_at IS NOT NULL FROM databases WHERE name = 'dina_shop'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(checked);
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn running_imports_are_asked_how_far_they_are_and_the_answer_is_kept(pool: PgPool) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let mut registry = Registry::new();
    registry.register("report_import_progress", |_| async move {
        Ok(Output::ImportProgress {
            stage: "importing".into(),
            bytes_done: 4_000,
            bytes_total: 10_000,
            statements: 77,
            error: None,
        })
    });
    let node = make_node_with(&p, &pool, "imports", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));

    assert_eq!(
        d.request_import_progress().await,
        0,
        "no import, nobody asked"
    );
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'eve', 'eve@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'eve', 10005, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let database: Uuid = sqlx::query_scalar(
        "INSERT INTO databases (tenant_id, name, status) VALUES ($1, 'eve_shop', 'ready') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let import: Uuid = sqlx::query_scalar(
        "INSERT INTO db_imports (tenant_id, database_id, gzip, size_bytes, sha256, status)
         VALUES ($1, $2, false, 10000, repeat('a', 64), 'queued') RETURNING id",
    )
    .bind(tenant)
    .bind(database)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        d.request_import_progress().await,
        0,
        "only running imports are asked about"
    );
    sqlx::query("UPDATE db_imports SET status = 'running' WHERE id = $1")
        .bind(import)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.request_import_progress().await, 1);
    assert_eq!(d.request_import_progress().await, 0, "once per time slot");
    let mut statements = 0_i64;
    for _ in 0..100 {
        statements = sqlx::query_scalar("SELECT statements FROM db_imports WHERE id = $1")
            .bind(import)
            .fetch_one(&pool)
            .await
            .unwrap();
        if statements > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(statements, 77);
    let (stage, done): (String, i64) =
        sqlx::query_as("SELECT stage, bytes_done FROM db_imports WHERE id = $1")
            .bind(import)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((stage.as_str(), done), ("importing", 4_000));

    // A node that has lost the import (it restarted) ends it, but not at once.
    builtpanel_database::imports::lost(&pool, import)
        .await
        .unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM db_imports WHERE id = $1")
        .bind(import)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "running");
    sqlx::query("UPDATE db_imports SET updated_at = now() - interval '2 minutes' WHERE id = $1")
        .bind(import)
        .execute(&pool)
        .await
        .unwrap();
    builtpanel_database::imports::lost(&pool, import)
        .await
        .unwrap();
    let (status, error): (String, Option<String>) =
        sqlx::query_as("SELECT status, error FROM db_imports WHERE id = $1")
            .bind(import)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    assert!(error.unwrap().contains("restarted"));
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_disk_usage_answer_is_kept_for_the_account(pool: PgPool) {
    use builtpanel_node_protocol::files::{FileRequest, FileResult, FolderUsage};
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let mut registry = Registry::new();
    registry.register("file_op", |_| async move {
        Ok(Output::File {
            result: FileResult::Usage {
                total_bytes: 123_456,
                inodes: 77,
                folders: vec![FolderUsage {
                    path: "www".into(),
                    bytes: 100_000,
                    inodes: 70,
                }],
                partial: false,
            },
        })
    });
    let node = make_node_with(&p, &pool, "usage", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'fay', 'fay@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'fay', 10006, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut request = builtpanel_operations::Request::new(
        node.id,
        builtpanel_node_protocol::Operation::FileOp {
            tenant_id: tenant,
            uid: 10_006,
            request: FileRequest::DiskUsage { top: 5 },
        },
        "du:test",
    );
    request.tenant_id = Some(tenant);
    p.ops.enqueue(&request).await.unwrap();
    let mut found = None;
    for _ in 0..100 {
        found = builtpanel_database::file_transfers::usage_of(&pool, tenant)
            .await
            .unwrap();
        if found.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let found = found.expect("the measurement was kept");
    assert_eq!(
        (found.total_bytes, found.inodes, found.partial),
        (123_456, 77, false)
    );
    assert_eq!(found.folders[0]["path"], "www");
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_wordpress_install_moves_from_the_database_to_the_grant_to_the_installer_to_active(
    pool: PgPool,
) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let mut registry = Registry::new();
    registry.register("grant_db", |_| async move { Ok(Output::DbGranted) });
    registry.register("install_wordpress", |op| async move {
        // The passwords arrive only on the wire; the node would refuse without them.
        let Operation::InstallWordPress {
            database_password,
            admin_password,
            path,
            ..
        } = op
        else {
            return Err(AgentError::new(ErrorCode::Internal, "wrong operation"));
        };
        if database_password.is_some() || admin_password.is_some() {
            // (No secret filler in this test: they stay absent.)
            return Err(AgentError::new(ErrorCode::Internal, "unexpected secrets"));
        }
        assert_eq!(path, "blog");
        Ok(Output::WordPressInstalled {
            version: "7.1.4".into(),
        })
    });
    let node = make_node_with(&p, &pool, "wp", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'eva', 'eva@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'eva', 10005, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let database: Uuid = sqlx::query_scalar(
        "INSERT INTO databases (tenant_id, name, status) VALUES ($1, 'eva_wp1', 'pending') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO db_users (tenant_id, name, status) VALUES ($1, 'eva_wp1', 'pending') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let domain: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, 'eva.example', 'primary') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let settings = serde_json::json!({
        "stage": "setup", "folder": "blog", "url": "https://eva.example/blog", "title": "Eva",
        "admin_user": "eva", "admin_email": "eva@example.com", "database": "eva_wp1",
        "db_id": database.to_string(), "user_id": user.to_string(), "plugins": []
    });
    let app: Uuid = sqlx::query_scalar(
        "INSERT INTO apps (tenant_id, domain_id, path, kind, name, database_id, settings)
         VALUES ($1, $2, '/blog', 'wordpress', 'Eva', $3, $4) RETURNING id",
    )
    .bind(tenant)
    .bind(domain)
    .bind(database)
    .bind(settings)
    .fetch_one(&pool)
    .await
    .unwrap();
    let status = |pool: PgPool| async move {
        sqlx::query_scalar::<_, String>("SELECT status FROM apps WHERE id = $1")
            .bind(app)
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    // Nothing happens while the database and its user are not ready.
    assert_eq!(d.request_app_installs().await, 0);
    assert_eq!(status(pool.clone()).await, "installing");
    sqlx::query("UPDATE databases SET status = 'ready' WHERE id = $1")
        .bind(database)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        d.request_app_installs().await,
        0,
        "the user is not ready yet"
    );
    sqlx::query("UPDATE db_users SET status = 'ready' WHERE id = $1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    // Ready: the user is given the database.
    assert_eq!(d.request_app_installs().await, 1);
    let grant: String =
        sqlx::query_scalar("SELECT access FROM db_grants WHERE database_id = $1 AND user_id = $2")
            .bind(database)
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(grant, "all");
    // Once the node has done it, the installer goes; then the app turns active.
    let mut moved = 0;
    for _ in 0..100 {
        moved += d.request_app_installs().await;
        if status(pool.clone()).await == "active" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(status(pool.clone()).await, "active");
    assert_eq!(moved, 1, "the installer was sent once");
    let version: Option<String> = sqlx::query_scalar("SELECT version FROM apps WHERE id = $1")
        .bind(app)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(version.as_deref(), Some("7.1.4"));
    // A failed database fails the app and forgets its secrets.
    let failing: Uuid = sqlx::query_scalar(
        "INSERT INTO apps (tenant_id, domain_id, path, kind, name, settings)
         VALUES ($1, $2, '/other', 'wordpress', 'Other', $3) RETURNING id",
    )
    .bind(tenant)
    .bind(domain)
    .bind(serde_json::json!({"stage": "setup", "db_id": database.to_string(), "user_id": user.to_string()}))
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE databases SET status = 'failed' WHERE id = $1")
        .bind(database)
        .execute(&pool)
        .await
        .unwrap();
    d.request_app_installs().await;
    let (s, e): (String, Option<String>) =
        sqlx::query_as("SELECT status, error FROM apps WHERE id = $1")
            .bind(failing)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(s, "failed");
    assert!(e.unwrap().contains("database"));
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_application_without_a_database_goes_straight_to_the_installer(pool: PgPool) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let mut registry = Registry::new();
    registry.register("install_app", |op| async move {
        let Operation::InstallApp {
            definition,
            database,
            path,
            ..
        } = op
        else {
            return Err(AgentError::new(ErrorCode::Internal, "wrong operation"));
        };
        assert_eq!(
            (definition.slug.as_str(), database.is_none(), path.as_str()),
            ("static-site", true, "promo")
        );
        Ok(Output::AppInstalled {
            version: "1.0".into(),
        })
    });
    let node = make_node_with(&p, &pool, "apps", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'fay', 'fay@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'fay', 10006, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let definition = builtpanel_catalog::Catalog::embedded()
        .unwrap()
        .get("static-site")
        .unwrap()
        .clone();
    let settings = serde_json::json!({
        "stage": "grant", "folder": "promo", "url": "https://fay.example/promo", "title": "Promo",
        "definition": definition
    });
    let app: Uuid = sqlx::query_scalar(
        "INSERT INTO apps (tenant_id, path, kind, name, settings)
         VALUES ($1, '/promo', 'static-site', 'Promo', $2) RETURNING id",
    )
    .bind(tenant)
    .bind(settings)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(d.request_app_installs().await, 1);
    let mut status = String::new();
    for _ in 0..100 {
        d.request_app_installs().await;
        status = sqlx::query_scalar("SELECT status FROM apps WHERE id = $1")
            .bind(app)
            .fetch_one(&pool)
            .await
            .unwrap();
        if status == "active" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(status, "active");
    let version: Option<String> = sqlx::query_scalar("SELECT version FROM apps WHERE id = $1")
        .bind(app)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(version.as_deref(), Some("1.0"));
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_cache_server_is_ready_when_the_node_says_so_and_gone_when_it_is_removed(pool: PgPool) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let mut registry = Registry::new();
    registry.register("set_cache_instance", |_| async move {
        Ok(Output::CacheInstanceSet {
            host: "bp-gus-redis".into(),
            port: 6379,
        })
    });
    registry.register("remove_cache_instance", |_| async move {
        Ok(Output::CacheInstanceRemoved { existed: true })
    });
    let node = make_node_with(&p, &pool, "cache", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'gus', 'gus@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'gus', 10007, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let instance: Uuid = sqlx::query_scalar(
        "INSERT INTO cache_instances (tenant_id, kind, memory_mb) VALUES ($1, 'redis', 32) RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let ask = |op: Operation, key: &str| {
        let request = Request::new(node.id, op, key.to_owned());
        let ops = p.ops.clone();
        async move {
            match ops.enqueue(&request).await.unwrap() {
                builtpanel_operations::Enqueued::Created(op)
                | builtpanel_operations::Enqueued::Existing(op) => op.id,
            }
        }
    };
    let op = ask(
        Operation::SetCacheInstance {
            instance_id: instance,
            tenant_id: tenant,
            username: "gus".into(),
            uid: 10_007,
            kind: "redis".into(),
            memory_mb: 32,
            password: None,
        },
        "cache-test-1",
    )
    .await;
    sqlx::query("UPDATE cache_instances SET op_id = $2 WHERE id = $1")
        .bind(instance)
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
    let status = |pool: PgPool| async move {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, host FROM cache_instances WHERE kind = 'redis'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap()
    };
    let mut now = None;
    for _ in 0..100 {
        d.settle_cache_instances().await;
        now = status(pool.clone()).await;
        if now.as_ref().is_some_and(|(s, _)| s == "ready") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(now, Some(("ready".into(), Some("bp-gus-redis".into()))));
    // Removing: the row goes when the node has removed the server.
    let op = ask(
        Operation::RemoveCacheInstance {
            tenant_id: tenant,
            username: "gus".into(),
            kind: "redis".into(),
        },
        "cache-test-2",
    )
    .await;
    sqlx::query("UPDATE cache_instances SET op_id = $2, status = 'removing' WHERE id = $1")
        .bind(instance)
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
    for _ in 0..100 {
        d.settle_cache_instances().await;
        now = status(pool.clone()).await;
        if now.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(now, None);
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_application_is_ready_when_the_node_says_so_failed_with_its_reason_and_gone_when_removed(
    pool: PgPool,
) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let mut registry = Registry::new();
    registry.register("set_runtime_app", |op| async move {
        let Operation::SetRuntimeApp { name, port, .. } = op else {
            unreachable!()
        };
        if name == "broken" {
            return Err(AgentError::new(
                ErrorCode::Internal,
                "the application stopped right after starting (exit code 1): boom",
            ));
        }
        Ok(Output::RuntimeAppSet {
            host: format!("bp-gus-app-{name}:{port}"),
        })
    });
    registry.register("remove_runtime_app", |_| async move {
        Ok(Output::RuntimeAppRemoved { existed: true })
    });
    let node = make_node_with(&p, &pool, "apps", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'gus', 'gus@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'gus', 10007, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut ids = Vec::new();
    for name in ["api", "broken"] {
        let app: Uuid = sqlx::query_scalar(
            "INSERT INTO runtime_apps (tenant_id, name, kind, version, start_command, port)
             VALUES ($1, $2, 'node', '24', 'node a.js', 3000) RETURNING id",
        )
        .bind(tenant)
        .bind(name)
        .fetch_one(&pool)
        .await
        .unwrap();
        let op = match p
            .ops
            .enqueue(&Request::new(
                node.id,
                Operation::SetRuntimeApp {
                    app_id: app,
                    tenant_id: tenant,
                    username: "gus".into(),
                    uid: 10_007,
                    kind: "node".into(),
                    version: "24".into(),
                    name: name.into(),
                    dir: String::new(),
                    command: vec!["node".into(), "a.js".into()],
                    install: false,
                    port: 3000,
                    memory_mb: 256,
                    env: None,
                    image: None,
                },
                format!("rt-{name}"),
            ))
            .await
            .unwrap()
        {
            builtpanel_operations::Enqueued::Created(op)
            | builtpanel_operations::Enqueued::Existing(op) => op.id,
        };
        sqlx::query("UPDATE runtime_apps SET op_id = $2 WHERE id = $1")
            .bind(app)
            .bind(op)
            .execute(&pool)
            .await
            .unwrap();
        ids.push(app);
    }
    let state = |pool: PgPool, name: &'static str| async move {
        sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT status, host, error FROM runtime_apps WHERE name = $1",
        )
        .bind(name)
        .fetch_optional(&pool)
        .await
        .unwrap()
    };
    let mut api = None;
    let mut broken = None;
    for _ in 0..100 {
        d.settle_runtime_apps().await;
        api = state(pool.clone(), "api").await;
        broken = state(pool.clone(), "broken").await;
        if api.as_ref().is_some_and(|(s, _, _)| s == "ready")
            && broken.as_ref().is_some_and(|(s, _, _)| s == "failed")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        api,
        Some(("ready".into(), Some("bp-gus-app-api:3000".into()), None))
    );
    let (status, host, error) = broken.unwrap();
    assert_eq!((status.as_str(), host), ("failed", None));
    assert!(error.unwrap().contains("exit code 1"));
    // Removed: the row goes when the node has removed the container.
    let op = match p
        .ops
        .enqueue(&Request::new(
            node.id,
            Operation::RemoveRuntimeApp {
                tenant_id: tenant,
                username: "gus".into(),
                name: "api".into(),
            },
            "rt-remove".to_owned(),
        ))
        .await
        .unwrap()
    {
        builtpanel_operations::Enqueued::Created(op)
        | builtpanel_operations::Enqueued::Existing(op) => op.id,
    };
    sqlx::query("UPDATE runtime_apps SET op_id = $2, status = 'removing' WHERE id = $1")
        .bind(ids[0])
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
    for _ in 0..100 {
        d.settle_runtime_apps().await;
        api = state(pool.clone(), "api").await;
        if api.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(api, None);
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_git_deploy_is_live_when_the_node_says_so_failed_with_its_reason_and_gone_when_removed(
    pool: PgPool,
) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let mut registry = Registry::new();
    registry.register("run_git_deploy", |op| async move {
        let Operation::RunGitDeploy { name, .. } = op else {
            unreachable!()
        };
        if name == "broken" {
            return Err(AgentError::new(
                ErrorCode::Internal,
                "fetching the repository failed (exit code 128): not found",
            ));
        }
        Ok(Output::GitDeployed {
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            log: "$ git clone main\nok".into(),
        })
    });
    registry.register("remove_git_deploy", |_| async move {
        Ok(Output::GitDeployRemoved)
    });
    let node = make_node_with(&p, &pool, "gitdeploy", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'gus', 'gus@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'gus', 10007, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut ids = Vec::new();
    for name in ["site", "broken"] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO git_deploys (tenant_id, name, repo_url, dest, sealed_webhook, status)
             VALUES ($1, $2, 'https://git.example.com/a/b.git', $2, '\\x00', 'deploying') RETURNING id",
        )
        .bind(tenant)
        .bind(name)
        .fetch_one(&pool)
        .await
        .unwrap();
        let op = match p
            .ops
            .enqueue(&Request::new(
                node.id,
                Operation::RunGitDeploy {
                    deploy_id: id,
                    tenant_id: tenant,
                    username: "gus".into(),
                    uid: 10_007,
                    name: name.into(),
                    repo_url: "https://git.example.com/a/b.git".into(),
                    branch: "main".into(),
                    dest: name.into(),
                    build_kind: None,
                    build_version: None,
                    build_steps: Vec::new(),
                    restart_app: None,
                    token: None,
                },
                format!("gd-{name}"),
            ))
            .await
            .unwrap()
        {
            builtpanel_operations::Enqueued::Created(op)
            | builtpanel_operations::Enqueued::Existing(op) => op.id,
        };
        sqlx::query("UPDATE git_deploys SET op_id = $2 WHERE id = $1")
            .bind(id)
            .bind(op)
            .execute(&pool)
            .await
            .unwrap();
        ids.push(id);
    }
    let state = |pool: PgPool, name: &'static str| async move {
        sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(
            "SELECT status, last_commit, last_log, last_error FROM git_deploys WHERE name = $1",
        )
        .bind(name)
        .fetch_optional(&pool)
        .await
        .unwrap()
    };
    let (mut site, mut broken) = (None, None);
    for _ in 0..100 {
        d.settle_git_deploys().await;
        site = state(pool.clone(), "site").await;
        broken = state(pool.clone(), "broken").await;
        if site.as_ref().is_some_and(|s| s.0 == "ok")
            && broken.as_ref().is_some_and(|s| s.0 == "failed")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let site = site.unwrap();
    assert_eq!(site.0, "ok");
    assert_eq!(
        site.1.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert!(site.2.unwrap().contains("git clone"));
    let broken = broken.unwrap();
    assert_eq!((broken.0.as_str(), broken.1), ("failed", None));
    assert!(broken.3.unwrap().contains("exit code 128"));
    // Removing: the row goes.
    let op = match p
        .ops
        .enqueue(&Request::new(
            node.id,
            Operation::RemoveGitDeploy {
                tenant_id: tenant,
                username: "gus".into(),
                name: "site".into(),
            },
            "gd-remove".to_owned(),
        ))
        .await
        .unwrap()
    {
        builtpanel_operations::Enqueued::Created(op)
        | builtpanel_operations::Enqueued::Existing(op) => op.id,
    };
    sqlx::query("UPDATE git_deploys SET op_id = $2, status = 'removing' WHERE id = $1")
        .bind(ids[0])
        .bind(op)
        .execute(&pool)
        .await
        .unwrap();
    let mut gone = None;
    for _ in 0..100 {
        d.settle_git_deploys().await;
        gone = state(pool.clone(), "site").await;
        if gone.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(gone, None);
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_plan_with_dedicated_databases_moves_a_tenants_databases_and_back(pool: PgPool) {
    let p = plane(&pool).await;
    let d = Dispatcher::new(p.ops.clone(), p.gateway.clone(), None, dispatch_settings());
    let mut registry = Registry::new();
    registry.register("migrate_tenant_databases", |_| async move {
        Ok(Output::DbMigrated {
            databases: 1,
            users: 1,
        })
    });
    registry.register("drop_database", |_| async move {
        Ok(Output::DatabaseDropped { existed: true })
    });
    registry.register("drop_db_user", |_| async move {
        Ok(Output::DbUserDropped { existed: true })
    });
    registry.register("remove_dedicated_db", |_| async move {
        Ok(Output::DedicatedDbRemoved { existed: true })
    });
    let node = make_node_with(&p, &pool, "moves", registry).await;
    let (agent_stop, agent) = node.start_agent(p.addr);
    wait_connected(&p, node.id).await;
    let results = p.gateway.take_results().expect("results taken once");
    tokio::spawn(d.clone().run(results, p.stop.subscribe()));

    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', 'dina', 'dina@example.com',
                 '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, 'dina', 10004, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node.id)
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();
    let database: Uuid = sqlx::query_scalar(
        "INSERT INTO databases (tenant_id, name, status) VALUES ($1, 'dina_shop', 'ready') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO db_users (tenant_id, name, status) VALUES ($1, 'dina_web', 'ready') RETURNING id",
    )
    .bind(tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO db_grants (tenant_id, database_id, user_id, access) VALUES ($1, $2, $3, 'all')")
        .bind(tenant)
        .bind(database)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO db_remote_hosts (user_id, cidr) VALUES ($1, '203.0.113.7/32')")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    let state = || {
        let pool = pool.clone();
        async move {
            builtpanel_database::tenant_db::state(&pool, tenant, "mysql")
                .await
                .unwrap()
        }
    };

    // The plan has no such feature: nothing moves.
    d.reconcile_db_isolation().await;
    assert_eq!(state().await, "shared");

    // The plan gets it: the databases are moved by one operation carrying all there is.
    sqlx::query(
        "UPDATE feature_sets SET all_features = false, features = ARRAY['databases', 'db_dedicated']::feature_key[]
         WHERE id = (SELECT feature_set_id FROM plans WHERE id = $1)",
    )
    .bind(plan)
    .execute(&pool)
    .await
    .unwrap();
    assert!(d.reconcile_db_isolation().await >= 1);
    assert_eq!(state().await, "moving_to_dedicated");
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM operations WHERE kind = 'migrate_tenant_databases'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payload["to_dedicated"], true);
    assert_eq!(payload["databases"], serde_json::json!(["dina_shop"]));
    assert_eq!(payload["users"], serde_json::json!(["dina_web"]));
    assert_eq!(payload["grants"][0]["access"], "all");
    assert_eq!(
        payload["remote_hosts"][0]["cidrs"],
        serde_json::json!(["203.0.113.7/32"])
    );
    assert!(payload.get("password").is_none());
    // While it runs, new work for the tenant is told to wait.
    assert!(
        builtpanel_database::tenant_db::dedicated_for(&pool, tenant, "dina", "mysql")
            .await
            .unwrap()
            .is_err()
    );
    // The node did it: the tenant is switched, and the old copies are asked to go.
    let mut switched = String::new();
    for _ in 0..100 {
        d.reconcile_db_isolation().await;
        switched = state().await;
        if switched == "dedicated" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(switched, "dedicated");
    assert_eq!(
        builtpanel_database::tenant_db::dedicated_for(&pool, tenant, "dina", "mysql")
            .await
            .unwrap(),
        Ok(Some("dina".to_owned()))
    );
    let cleanup: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM operations WHERE kind IN ('drop_database', 'drop_db_user') ORDER BY kind",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(cleanup, ["drop_database", "drop_db_user"]);
    let dropped: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM operations WHERE kind = 'drop_database'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        dropped.get("dedicated").is_none(),
        "the old copy is on the shared server: {dropped}"
    );
    // PostgreSQL has nothing: it was switched too, with nothing to move.
    assert_eq!(
        builtpanel_database::tenant_db::state(&pool, tenant, "postgres")
            .await
            .unwrap(),
        "dedicated"
    );

    // The plan loses it: a short while later the databases come back.
    sqlx::query("UPDATE feature_sets SET features = ARRAY['databases']::feature_key[] WHERE id = (SELECT feature_set_id FROM plans WHERE id = $1)")
        .bind(plan)
        .execute(&pool)
        .await
        .unwrap();
    d.reconcile_db_isolation().await;
    assert_eq!(state().await, "dedicated", "not at once: moves are spaced");
    sqlx::query("UPDATE tenant_db_servers SET updated_at = now() - interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();
    d.reconcile_db_isolation().await;
    assert_eq!(state().await, "moving_to_shared");
    for _ in 0..100 {
        d.reconcile_db_isolation().await;
        if state().await == "shared" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(state().await, "shared");
    let removals: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE kind = 'remove_dedicated_db' AND payload ->> 'engine' = 'mysql'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(removals >= 1);
    agent_stop.send(true).unwrap();
    agent.await.unwrap();
}
