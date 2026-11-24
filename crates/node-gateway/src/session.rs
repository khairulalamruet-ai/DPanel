//! One agent connection, from TCP accept to goodbye.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use builtpanel_database::nodes;
use builtpanel_node_protocol::{
    AgentMessage, ControlMessage, MAX_FRAME_BYTES, STREAM_PATH, Welcome, decode, encode,
    negotiate_version,
};
use futures_util::{SinkExt as _, StreamExt as _};
use sha2::{Digest as _, Sha256};
use tokio::{
    net::TcpStream,
    sync::watch,
    time::{Instant, MissedTickBehavior, interval, timeout},
};
use tokio_tungstenite::{
    WebSocketStream, accept_hdr_async_with_config,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
        http::StatusCode,
        protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
    },
};
use tracing::Instrument as _;

use crate::{Inner, wait_for_shutdown};

type Ws = WebSocketStream<tokio_rustls::server::TlsStream<TcpStream>>;

/// Entry point of a connection task. Never panics on bad input; every failure
/// is logged at a level that does not let a stranger fill the log (`debug`
/// before the certificate is known, `info`/`warn` after).
pub(crate) async fn run(
    inner: Arc<Inner>,
    tcp: TcpStream,
    peer: SocketAddr,
    shutdown: watch::Receiver<bool>,
) {
    async {
        let _ = tcp.set_nodelay(true);
        if let Err(reason) = serve(&inner, tcp, peer, shutdown).await {
            tracing::debug!(%reason, "agent connection ended before a session started");
        }
    }
    .instrument(tracing::debug_span!("agent_conn", %peer))
    .await;
}

async fn serve(
    inner: &Arc<Inner>,
    tcp: TcpStream,
    peer: SocketAddr,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let settings = &inner.settings;
    let deadline = Instant::now() + settings.handshake_timeout;
    let left = || deadline.saturating_duration_since(Instant::now());

    // 1. mutual TLS: a certificate from our CA is already proven here.
    let tls = timeout(left(), inner.acceptor.accept(tcp))
        .await
        .map_err(|_| "TLS handshake timed out".to_owned())?
        .map_err(|e| format!("TLS handshake failed: {e}"))?;
    let fingerprint = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(<[_]>::first)
        .map(|der| hex(&Sha256::digest(der.as_ref())))
        .ok_or("no client certificate")?;

    // 2. the certificate must belong to a node that may connect.
    let node = nodes::node_for_certificate(&inner.db, &fingerprint)
        .await
        .map_err(|e| format!("certificate lookup failed: {e}"))?;
    let Some(node) = node else {
        tracing::warn!(%peer, certificate = &fingerprint[..12], "agent certificate refused: unknown, revoked or expired");
        return Err("certificate not accepted".into());
    };

    // 3. WebSocket upgrade on the one path we serve.
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME_BYTES))
        .max_frame_size(Some(MAX_FRAME_BYTES));
    // The error type is fixed by the WebSocket library's callback signature.
    #[allow(clippy::result_large_err)]
    let check_path = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if req.uri().path() == STREAM_PATH {
            Ok(resp)
        } else {
            let mut not_found = ErrorResponse::new(None);
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Err(not_found)
        }
    };
    let mut ws: Ws = timeout(
        left(),
        accept_hdr_async_with_config(tls, check_path, Some(config)),
    )
    .await
    .map_err(|_| "WebSocket upgrade timed out".to_owned())?
    .map_err(|e| format!("WebSocket upgrade failed: {e}"))?;

    // 4. Hello / Welcome.
    let hello = match timeout(left(), next_text(&mut ws)).await {
        Ok(Some(text)) => match decode::<AgentMessage>(&text) {
            Ok(AgentMessage::Hello(hello)) => hello,
            _ => {
                close(&mut ws, CloseCode::Protocol, "expected hello").await;
                return Err("first message was not hello".into());
            }
        },
        _ => return Err("no hello".into()),
    };
    let Some(version) = negotiate_version(hello.protocol_version) else {
        tracing::warn!(node = %node.name, agent_protocol = hello.protocol_version, "agent speaks an unsupported protocol version");
        close(&mut ws, CloseCode::Protocol, "unsupported protocol version").await;
        return Err("unsupported protocol version".into());
    };
    let welcome = ControlMessage::Welcome(Welcome {
        protocol_version: version,
        server_time: time_now(),
    });
    let frame = encode(&welcome).map_err(|e| e.to_string())?;
    timeout(left(), ws.send(Message::Text(frame.into())))
        .await
        .map_err(|_| "welcome timed out".to_owned())?
        .map_err(|e| e.to_string())?;

    // The node is now online: this process owns its session.
    let (session, stop, mut outbound) = inner.registry.register(node.id);
    let ip = peer.ip().to_string();
    if let Err(err) = nodes::mark_online(&inner.db, node.id, &hello.agent_version, Some(&ip)).await
    {
        inner.registry.release(node.id, session);
        tracing::error!(node = %node.name, error = %err, "could not record the node as online");
        return Err("database unavailable".into());
    }
    tracing::info!(node = %node.name, node_id = %node.id, agent = %hello.agent_version, protocol = version, "node online");

    let outcome = pump(inner, &mut ws, node.id, &stop, &mut outbound, &mut shutdown).await;

    if inner.registry.release(node.id, session)
        && let Err(err) = nodes::mark_offline(&inner.db, node.id).await
    {
        tracing::warn!(node = %node.name, error = %err, "could not record the node as offline");
    }
    tracing::info!(node = %node.name, node_id = %node.id, reason = outcome, "node offline");
    Ok(())
}

/// The steady state: ping, read, refresh `last_seen_at`. Returns why it ended.
async fn pump(
    inner: &Arc<Inner>,
    ws: &mut Ws,
    node: uuid::Uuid,
    stop: &crate::registry::Stop,
    outbound: &mut tokio::sync::mpsc::Receiver<String>,
    shutdown: &mut watch::Receiver<bool>,
) -> &'static str {
    let settings = &inner.settings;
    let mut ticker = interval(settings.ping_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_heard = Instant::now();
    loop {
        tokio::select! {
            () = wait_for_shutdown(shutdown) => {
                close(ws, CloseCode::Away, "control plane is stopping").await;
                return "control plane stopping";
            }
            () = stop.requested() => {
                if stop.was_revoked() {
                    close(ws, CloseCode::Policy, "node revoked").await;
                    return "node revoked";
                }
                close(ws, CloseCode::Policy, "replaced by a newer connection").await;
                return "replaced by a newer connection";
            }
            Some(frame) = outbound.recv() => {
                let sent = timeout(settings.liveness_timeout, ws.send(Message::Text(frame.into()))).await;
                if !matches!(sent, Ok(Ok(()))) {
                    return "could not send a frame";
                }
            }
            _ = ticker.tick() => {
                if last_heard.elapsed() >= settings.liveness_timeout {
                    close(ws, CloseCode::Policy, "heartbeat timeout").await;
                    return "heartbeat timeout";
                }
                // A peer that does not read cannot block us for long.
                let sent = timeout(settings.liveness_timeout, ws.send(Message::Ping(Vec::new().into()))).await;
                if !matches!(sent, Ok(Ok(()))) {
                    return "could not send ping";
                }
            }
            frame = ws.next() => {
                let Some(Ok(message)) = frame else { return "connection closed" };
                last_heard = Instant::now();
                match message {
                    Message::Pong(_) => {
                        if let Err(err) = nodes::touch_seen(&inner.db, node).await {
                            tracing::warn!(%node, error = %err, "could not refresh last_seen_at");
                        }
                    }
                    Message::Ping(_) | Message::Frame(_) => {}
                    Message::Text(text) => match decode::<AgentMessage>(text.as_str()) {
                        Ok(AgentMessage::Result(result)) => {
                            tracing::debug!(%node, op_id = %result.op_id, status = ?result.status, "operation result");
                            let report = crate::NodeResult { node_id: node, result };
                            if inner.results.try_send(report).is_err() {
                                // Nobody is listening or the queue is full. The operation
                                // times out and is asked again; the agent answers from memory.
                                tracing::warn!(%node, "operation result dropped: dispatcher not keeping up");
                            }
                        }
                        Ok(AgentMessage::Hello(_)) | Err(_) => {
                            close(ws, CloseCode::Invalid, "unexpected or malformed message").await;
                            return "protocol violation";
                        }
                    },
                    Message::Binary(_) => {
                        close(ws, CloseCode::Unsupported, "text frames only").await;
                        return "binary frame";
                    }
                    Message::Close(_) => return "agent said goodbye",
                }
            }
        }
    }
}

/// The next text message, skipping control frames; `None` if the connection ended.
async fn next_text(ws: &mut Ws) -> Option<String> {
    while let Some(Ok(message)) = ws.next().await {
        match message {
            Message::Text(text) => return Some(text.as_str().to_owned()),
            Message::Ping(_) | Message::Pong(_) => {}
            _ => return None,
        }
    }
    None
}

async fn close(ws: &mut Ws, code: CloseCode, reason: &'static str) {
    let frame = CloseFrame {
        code,
        reason: reason.into(),
    };
    let _ = timeout(Duration::from_secs(2), ws.close(Some(frame))).await;
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn time_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
