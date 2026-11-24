//! The control plane's agent port.
//!
//! Agents dial in; nothing ever dials out to them (ADR 0004). A connection is
//! accepted in four steps, and any failure just closes it:
//!
//! 1. **Mutual TLS 1.3.** The client must present a certificate signed by the
//!    node CA with client-authentication usage ([`tls`]).
//! 2. **Known certificate.** The certificate's fingerprint must belong to a
//!    node that is not revoked, and the certificate itself must be unrevoked and
//!    inside its validity window ([`builtpanel_database::nodes::node_for_certificate`]).
//!    Revoking a node or a certificate therefore stops it at its next connect.
//! 3. **WebSocket upgrade** on [`STREAM_PATH`] only.
//! 4. **`Hello` / `Welcome`**: the protocol version is negotiated.
//!
//! A node is `online` while its session lives. The control plane pings every
//! [`HEARTBEAT_INTERVAL_SECS`]; every answer refreshes `last_seen_at`. A session
//! that stays silent for [`LIVENESS_TIMEOUT_SECS`] is closed, and a background
//! sweep also marks nodes `offline` whose `last_seen_at` is that old (sessions
//! lost without a goodbye, or `online` rows left by a control plane that was
//! stopped). One node has one session: a new connection replaces the old one.
//!
//! Frames are JSON text ([`builtpanel_node_protocol`]); anything else, or a frame
//! over [`MAX_FRAME_BYTES`], ends the session.

mod registry;
mod session;
pub mod tls;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use builtpanel_database::{nodes, sqlx::PgPool};
pub use builtpanel_node_protocol::{
    HEARTBEAT_INTERVAL_SECS, LIVENESS_TIMEOUT_SECS, MAX_FRAME_BYTES, STREAM_PATH,
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, watch},
};
use tokio_rustls::TlsAcceptor;
use uuid::Uuid;

use crate::registry::Registry;
pub use crate::registry::SendError;
use builtpanel_node_protocol::OperationResult;
use std::sync::Mutex;

/// Timing and limits. The defaults are what production uses; tests shorten them.
#[derive(Debug, Clone)]
pub struct Settings {
    /// How often an agent is pinged.
    pub ping_interval: Duration,
    /// How long a session may stay silent before it is closed.
    pub liveness_timeout: Duration,
    /// How long TLS, the upgrade and `Hello` may take together.
    pub handshake_timeout: Duration,
    /// Most connections served at once; more are dropped at accept.
    pub max_connections: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ping_interval: Duration::from_secs(HEARTBEAT_INTERVAL_SECS),
            liveness_timeout: Duration::from_secs(LIVENESS_TIMEOUT_SECS),
            handshake_timeout: Duration::from_secs(10),
            max_connections: 1024,
        }
    }
}

/// An agent's answer to an operation, with the node that sent it. The receiver
/// must check that the operation belongs to that node.
#[derive(Debug, Clone)]
pub struct NodeResult {
    /// The node whose certificate the connection was authenticated with.
    pub node_id: Uuid,
    /// What it reported.
    pub result: OperationResult,
}

/// Results waiting for the dispatcher.
const RESULT_QUEUE: usize = 1024;

struct Inner {
    results: tokio::sync::mpsc::Sender<NodeResult>,
    results_rx: Mutex<Option<tokio::sync::mpsc::Receiver<NodeResult>>>,
    db: PgPool,
    acceptor: TlsAcceptor,
    registry: Registry,
    settings: Settings,
}

/// The agent port. Cheap to clone.
#[derive(Clone)]
pub struct Gateway {
    inner: Arc<Inner>,
}

impl Gateway {
    /// Builds a gateway from a TLS configuration (see [`tls::server_config`]).
    #[must_use]
    pub fn new(db: PgPool, tls: Arc<rustls::ServerConfig>, settings: Settings) -> Self {
        let (results, results_rx) = tokio::sync::mpsc::channel(RESULT_QUEUE);
        Self {
            inner: Arc::new(Inner {
                results,
                results_rx: Mutex::new(Some(results_rx)),
                db,
                acceptor: TlsAcceptor::from(tls),
                registry: Registry::default(),
                settings,
            }),
        }
    }

    /// Sends a text frame (an encoded operation) to the node's live session.
    ///
    /// # Errors
    /// [`SendError`] when the node is not connected or its queue is full.
    pub fn send(&self, node: Uuid, frame: String) -> Result<(), SendError> {
        self.inner.registry.send(node, frame)
    }

    /// Nodes that have a live session right now.
    #[must_use]
    pub fn connected_nodes(&self) -> Vec<Uuid> {
        self.inner.registry.connected()
    }

    /// The stream of operation results from all nodes. There is one stream:
    /// the first caller gets it, later calls get `None`.
    #[must_use]
    pub fn take_results(&self) -> Option<tokio::sync::mpsc::Receiver<NodeResult>> {
        self.inner
            .results_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Whether this process holds a live session for the node.
    #[must_use]
    pub fn is_connected(&self, node: Uuid) -> bool {
        self.inner.registry.contains(node)
    }

    /// Closes the node's live session because the node was revoked. Returns
    /// whether it had one. (The database already refuses its certificates; this
    /// ends the connection that is open right now.)
    pub fn disconnect_revoked(&self, node: Uuid) -> bool {
        self.inner.registry.revoke(node)
    }

    /// Accepts connections until `shutdown` turns `true`, then closes every
    /// session. Also runs the sweep that marks silent nodes offline.
    pub async fn serve(self, listener: TcpListener, mut shutdown: watch::Receiver<bool>) {
        let limit = Arc::new(Semaphore::new(self.inner.settings.max_connections));
        let sweeper = tokio::spawn(sweep(self.inner.clone(), shutdown.clone()));
        loop {
            tokio::select! {
                () = wait_for_shutdown(&mut shutdown) => break,
                accepted = listener.accept() => match accepted {
                    Ok((tcp, peer)) => {
                        let Ok(permit) = limit.clone().try_acquire_owned() else {
                            tracing::warn!(%peer, "agent port is full: connection dropped");
                            continue;
                        };
                        let inner = self.inner.clone();
                        let shutdown = shutdown.clone();
                        tokio::spawn(async move {
                            session::run(inner, tcp, peer, shutdown).await;
                            drop(permit);
                        });
                    }
                    Err(err) => {
                        // Out of file descriptors and the like: do not spin.
                        tracing::warn!(error = %err, "accept failed on the agent port");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
            }
        }
        // Sessions see the same signal and close themselves; give them a moment.
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            let _ = limit
                .acquire_many(
                    u32::try_from(self.inner.settings.max_connections).unwrap_or(u32::MAX),
                )
                .await;
        })
        .await;
        let _ = sweeper.await;
    }
}

/// Binds the agent port.
///
/// # Errors
/// The socket could not be bound.
pub async fn bind(addr: SocketAddr) -> std::io::Result<TcpListener> {
    TcpListener::bind(addr).await
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

/// Marks nodes offline that nobody has heard from for the liveness timeout.
async fn sweep(inner: Arc<Inner>, mut shutdown: watch::Receiver<bool>) {
    let silent = i32::try_from(inner.settings.liveness_timeout.as_secs()).unwrap_or(i32::MAX);
    let mut tick = tokio::time::interval(inner.settings.ping_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = wait_for_shutdown(&mut shutdown) => return,
            _ = tick.tick() => match nodes::mark_silent_offline(&inner.db, silent).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(nodes = n, "nodes marked offline: no heartbeat"),
                Err(err) => tracing::warn!(error = %err, "offline sweep failed"),
            },
        }
    }
}
