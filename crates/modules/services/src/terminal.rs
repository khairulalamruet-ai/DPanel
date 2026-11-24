//! A terminal on a node, in the browser (Server: Terminal).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | POST | `/api/v1/admin/nodes/{id}/terminal/host-key` | `admin.server.manage`, sudo mode |
//! | GET (WebSocket) | `/api/v1/admin/nodes/{id}/terminal` | `admin.server.manage`, sudo mode |
//!
//! The shell is the node's own SSH server's: the control plane logs in over SSH with the login and the
//! password (or the key) the administrator types, and carries the terminal's bytes over the WebSocket.
//! Neither the node agent nor the panel runs a shell, and the panel's process never has more rights than
//! the SSH login it was given. The credential is used for that one connection and dropped.
//!
//! The server's host key is confirmed first (`host-key` shows it); a connection to a server that shows
//! another key is refused before anything is sent.
//!
//! WebSocket protocol. The first frame is text, JSON:
//! `{"user": "root", "password": "...", "host_key_sha256": "SHA256:...", "cols": 120, "rows": 30}`.
//! The server answers with a text frame `{"type":"ready"}` or `{"type":"error","code":"...","message":"..."}`
//! (and closes). After `ready`, binary frames carry the terminal's bytes both ways, and a text frame
//! `{"type":"resize","cols":N,"rows":N}` changes the terminal's size. A session ends when the shell ends, after
//! fifteen minutes without input, or after eight hours. Only the start and the end are audited (never what was
//! typed or shown).

use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{
        Path, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, header},
    response::{IntoResponse as _, Response},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_mod_migration::remote::{
    Credential, FetchError, Shell, ShellEvent, Source, probe_host_key,
};
use builtpanel_node_protocol::{Operation, Output};
use builtpanel_rbac::{RequireRecent, permissions::ServerManage};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{ServicesState, ask_node, internal};

/// Settings of the terminal.
#[derive(Clone, Debug, Default)]
pub struct TerminalSettings {
    /// Whether this machine's own addresses may be connected to (tests only).
    pub allow_local_ssh: bool,
    /// An SSH port to use instead of asking the node (tests only).
    pub ssh_port: Option<u16>,
}

/// Terminals open at once, in all.
const MAX_TERMINALS: usize = 8;
static TERMINALS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(MAX_TERMINALS);
/// How long the first frame may take.
const LOGIN_WAIT: Duration = Duration::from_secs(20);
/// A session with no input for this long ends.
const IDLE: Duration = Duration::from_secs(15 * 60);
/// The longest a session lasts.
const MAX_SESSION: Duration = Duration::from_secs(8 * 3600);
/// The biggest frame taken from the browser.
const MAX_FRAME: usize = 64 * 1024;

/// Where the terminal connects to, and the key the node shows.
#[derive(Debug, Serialize, ToSchema)]
pub struct TerminalTarget {
    /// The node's address.
    pub host: String,
    /// Its SSH port.
    pub port: u16,
    /// The host key's SHA-256 fingerprint (`SHA256:...`). Check it before sending a password.
    pub host_key_sha256: String,
}

/// The first frame of the WebSocket.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginFrame {
    #[serde(default = "root")]
    user: String,
    password: String,
    host_key_sha256: String,
    cols: u32,
    rows: u32,
}

fn root() -> String {
    "root".to_owned()
}

/// A text frame after the first.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Control {
    Resize { cols: u32, rows: u32 },
}

fn valid_user(user: &str) -> bool {
    let mut chars = user.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && user.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

fn valid_host_key(fp: &str) -> bool {
    fp.strip_prefix("SHA256:").is_some_and(|b| {
        b.len() == 43
            && b.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/'))
    })
}

fn valid_size(cols: u32, rows: u32) -> bool {
    (1..=500).contains(&cols) && (1..=200).contains(&rows)
}

/// What the page's `Origin` says must be this server (the browser always sends it on a WebSocket): a page of
/// another site cannot open a terminal with the administrator's cookie.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true; // not a browser
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_ascii_lowercase);
    let origin = origin
        .to_str()
        .ok()
        .and_then(|o| o.split_once("://"))
        .map(|(_, a)| a.trim_end_matches('/').to_ascii_lowercase());
    matches!((origin, host), (Some(o), Some(h)) if o == h)
}

async fn target_of(
    state: &ServicesState,
    who: &builtpanel_rbac::Principal,
    id: Uuid,
) -> Result<(String, u16), AppError> {
    let row: Option<(Option<String>, Option<String>)> = builtpanel_database::sqlx::query_as(
        "SELECT host(public_ip), hostname FROM nodes WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;
    let (ip, name) = row.ok_or_else(|| AppError::not_found("node"))?;
    let host = ip.or(name).ok_or_else(|| {
        AppError::conflict(
            "node_address_unknown",
            "The node has not told its address yet.",
        )
    })?;
    // The port the SSH server listens on (the first, when there are several).
    if let Some(port) = state.terminal.ssh_port {
        return Ok((host, port));
    }
    let port = match ask_node(state, id, who, &Operation::ReadSshAccess).await {
        Ok(Ok(Output::SshAccess { access })) => access.ports.first().copied().unwrap_or(22),
        _ => 22,
    };
    Ok((host, port))
}

fn ssh_error(e: &FetchError) -> (&'static str, String) {
    match e {
        FetchError::Login => (
            "ssh_login_refused",
            "The server refused the login (check the user and the password, and that password login is allowed)."
                .to_owned(),
        ),
        FetchError::HostKeyChanged { .. } | FetchError::UnknownHostKey(_) => (
            "ssh_host_key_changed",
            "The server shows another host key than the one confirmed: the connection was refused.".to_owned(),
        ),
        FetchError::Refused(_) => (
            "address_refused",
            "The panel may not connect to this address.".to_owned(),
        ),
        other => ("ssh_unreachable", other.to_string()),
    }
}

/// Shows the host key of the node's SSH server (nothing is sent to it, no password).
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/terminal/host-key",
    tag = "services",
    operation_id = "adminTerminalHostKey",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 200, description = "Where the terminal connects and the key shown", body = TerminalTarget),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`node_address_unknown` or `ssh_unreachable`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn host_key(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<TerminalTarget>, AppError> {
    let (host, port) = target_of(&state, &who.0, id).await?;
    let host_key_sha256 = probe_host_key(&host, port, state.terminal.allow_local_ssh)
        .await
        .map_err(|e| {
            let (code, message) = ssh_error(&e);
            AppError::conflict(code, message)
        })?;
    Ok(Json(TerminalTarget {
        host,
        port,
        host_key_sha256,
    }))
}

/// Opens a terminal on the node: a WebSocket (see the module's description for its frames). The answer to the
/// request is `101 Switching Protocols`.
#[utoipa::path(
    get,
    path = "/api/v1/admin/nodes/{id}/terminal",
    tag = "services",
    operation_id = "adminTerminal",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id")),
    responses(
        (status = 101, description = "The WebSocket opens; send the login frame first"),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `origin_not_allowed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`too_many_terminals` or `node_address_unknown`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn terminal(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    if !same_origin(&headers) {
        return Err(AppError::forbidden(
            "origin_not_allowed",
            "This page is not served by this panel.",
        ));
    }
    let (host, port) = target_of(&state, &who.0, id).await?;
    let permit = TERMINALS.try_acquire().map_err(|_| {
        AppError::conflict(
            "too_many_terminals",
            "Too many terminals are open: close one first.",
        )
    })?;
    let account = who.0.account_id();
    Ok(ws
        .max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| async move {
            session(state, account, id, host, port, socket).await;
            drop(permit);
        })
        .into_response())
}

async fn say(socket: &mut WebSocket, value: serde_json::Value) {
    let _ = socket.send(Message::Text(value.to_string().into())).await;
}

async fn refuse(socket: &mut WebSocket, code: &str, message: &str) {
    say(
        socket,
        serde_json::json!({"type": "error", "code": code, "message": message}),
    )
    .await;
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: 1008,
            reason: "".into(),
        })))
        .await;
}

async fn session(
    state: ServicesState,
    account: Uuid,
    node: Uuid,
    host: String,
    port: u16,
    mut socket: WebSocket,
) {
    let first = tokio::time::timeout(LOGIN_WAIT, socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = first else {
        refuse(&mut socket, "login_expected", "Send the login frame first.").await;
        return;
    };
    let Ok(login) = serde_json::from_str::<LoginFrame>(&text) else {
        refuse(
            &mut socket,
            "invalid_login_frame",
            "The login frame is not valid.",
        )
        .await;
        return;
    };
    if !valid_user(&login.user)
        || !valid_host_key(&login.host_key_sha256)
        || !valid_size(login.cols, login.rows)
        || login.password.is_empty()
        || login.password.len() > 1024
    {
        refuse(
            &mut socket,
            "invalid_login_frame",
            "The login frame is not valid.",
        )
        .await;
        return;
    }
    let user = login.user.clone();
    let source = Source {
        host: host.clone(),
        port,
        user: login.user,
        credential: Credential::Password(SecretString::from(login.password)),
        host_key: Some(login.host_key_sha256),
        path: String::new(),
        allow_local: state.terminal.allow_local_ssh,
    };
    let mut shell = match Shell::open(source, login.cols, login.rows).await {
        Ok(shell) => shell,
        Err(error) => {
            let (code, message) = ssh_error(&error);
            let _ = builtpanel_audit::insert(
                &state.db,
                &AuditEntry::new("node_terminal.login_failed", Outcome::Failure)
                    .actor(account)
                    .target("node", node)
                    .metadata(serde_json::json!({"user": user, "host": host, "port": port, "reason": code})),
            )
            .await;
            refuse(&mut socket, code, &message).await;
            return;
        }
    };
    let _ = builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("node_terminal.opened", Outcome::Success)
            .actor(account)
            .target("node", node)
            .metadata(serde_json::json!({"user": user, "host": host, "port": port})),
    )
    .await;
    say(&mut socket, serde_json::json!({"type": "ready"})).await;

    let started = Instant::now();
    let mut last_input = Instant::now();
    let (mut bytes_in, mut bytes_out) = (0_u64, 0_u64);
    let reason = loop {
        let idle_left = IDLE.saturating_sub(last_input.elapsed());
        let life_left = MAX_SESSION.saturating_sub(started.elapsed());
        let wait = idle_left.min(life_left);
        tokio::select! {
            msg = socket.recv() => match msg {
                Some(Ok(Message::Binary(data))) => {
                    last_input = Instant::now();
                    bytes_in += data.len() as u64;
                    if shell.write(&data).await.is_err() {
                        break "connection_lost";
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    last_input = Instant::now();
                    if let Ok(Control::Resize { cols, rows }) = serde_json::from_str(&text)
                        && valid_size(cols, rows)
                    {
                        let _ = shell.resize(cols, rows).await;
                    }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => break "browser_closed",
            },
            event = shell.next() => match event {
                ShellEvent::Data(data) => {
                    bytes_out += data.len() as u64;
                    if socket.send(Message::Binary(data.into())).await.is_err() {
                        break "browser_closed";
                    }
                }
                ShellEvent::Ended(_) => break "shell_ended",
            },
            () = tokio::time::sleep(wait) => {
                break if last_input.elapsed() >= IDLE { "idle" } else { "time_limit" };
            }
        }
    };
    shell.close().await;
    say(
        &mut socket,
        serde_json::json!({"type": "closed", "reason": reason}),
    )
    .await;
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: 1000,
            reason: "".into(),
        })))
        .await;
    let _ = builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("node_terminal.closed", Outcome::Success)
            .actor(account)
            .target("node", node)
            .metadata(serde_json::json!({
                "user": user, "reason": reason, "seconds": started.elapsed().as_secs(),
                "bytes_in": bytes_in, "bytes_out": bytes_out
            })),
    )
    .await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_page_of_another_site_cannot_open_a_terminal() {
        let mut h = HeaderMap::new();
        assert!(same_origin(&h), "no Origin: not a browser");
        h.insert(header::HOST, "panel.example.com:2087".parse().unwrap());
        h.insert(
            header::ORIGIN,
            "https://panel.example.com:2087".parse().unwrap(),
        );
        assert!(same_origin(&h));
        h.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!same_origin(&h));
        h.insert(header::ORIGIN, "null".parse().unwrap());
        assert!(!same_origin(&h));
    }

    #[test]
    fn what_the_login_frame_may_hold_is_checked() {
        assert!(valid_user("root") && !valid_user("Root") && !valid_user("a b"));
        assert!(
            valid_host_key(&format!("SHA256:{}", "A".repeat(43))) && !valid_host_key("SHA256:x")
        );
        assert!(
            valid_size(80, 24) && !valid_size(0, 24) && !valid_size(80, 201) && !valid_size(501, 5)
        );
        assert!(
            serde_json::from_str::<LoginFrame>(
                r#"{"password":"p","host_key_sha256":"k","cols":1,"rows":1,"x":1}"#
            )
            .is_err()
        );
    }
}
