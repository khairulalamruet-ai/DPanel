//! `fetch_file` and `send_file` (T08.07): big files between the control plane and a
//! tenant's files. The bytes wait in the agent's own folder (`<state>/transfers`)
//! between the network and the tenant's files; the privileged helper moves them across
//! as the tenant's own user (see `builtpanel_priv_helper::files`).

use std::{path::PathBuf, sync::Arc};

use super::{
    sqlexport::upload,
    sqlimport::{download, failed, invalid},
    tenant::{TenantContext, ask_helper},
};
use builtpanel_node_protocol::{AgentError, Operation, Output};
use builtpanel_priv_helper::protocol::Action;

/// The folder files wait in (`BUILTPANEL_AGENT_TRANSFERS`; the helper's `--transfers-dir`
/// is the same place). Without it, a folder in the state directory.
fn transfers_dir(context: &TenantContext) -> PathBuf {
    std::env::var_os("BUILTPANEL_AGENT_TRANSFERS").map_or_else(
        || context.vhosts_root.with_file_name("transfers"),
        PathBuf::from,
    )
}

fn plain_url(url: &str) -> bool {
    let local = url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost")
        || url.starts_with("http://[::1]");
    url.len() <= 400
        && url.is_ascii()
        && !url.chars().any(|c| c.is_control() || c == ' ')
        && !url.ends_with('/')
        && (url.starts_with("https://") || local)
}

/// `fetch_file`.
///
/// # Errors
/// [`AgentError`]: bad arguments or a refusal (a file is already there; not
/// retryable); the download breaking off or the helper being away (retryable).
pub async fn fetch_file(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::FetchFile {
        tenant_id,
        uid,
        transfer_id,
        dest,
        url,
        size_bytes,
        overwrite,
        token,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    if !plain_url(&url) {
        return Err(invalid("the download address must be https"));
    }
    if size_bytes == 0 {
        return Err(invalid("the file is empty"));
    }
    let token = token
        .ok_or_else(|| invalid("no credential for the download"))?
        .0;
    let dir = transfers_dir(&context);
    std::fs::create_dir_all(&dir)
        .map_err(|e| failed(&format!("cannot make the folder: {}", e.kind())).retryable())?;
    let name = format!("{}.in", transfer_id.simple());
    let path = dir.join(&name);
    let (url2, path2) = (url.clone(), path.clone());
    let fetched = tokio::task::spawn_blocking(move || {
        download(&url2, &token, size_bytes, &path2, transfer_id)
    })
    .await
    .map_err(|_| failed("the download was cancelled"))?;
    let placed = match fetched {
        Ok(()) => {
            let socket = context.helper_socket.clone();
            let action = Action::PlaceFile {
                tenant_id: tenant_id.to_string(),
                uid,
                staged: name.clone(),
                dest,
                overwrite,
                max_bytes: size_bytes,
            };
            tokio::task::spawn_blocking(move || ask_helper(&socket, action))
                .await
                .map_err(|_| failed("the placing was cancelled"))?
        }
        Err(e) => Err(e),
    };
    // A refusal ends it; a failure that may pass keeps the download for the next try.
    if placed.is_ok() || placed.as_ref().is_err_and(|e| !e.retryable) {
        let _ = std::fs::remove_file(&path);
    }
    placed?;
    Ok(Output::FileFetched { size_bytes })
}

/// `send_file`.
///
/// # Errors
/// [`AgentError`]: bad arguments, a missing or too big file (not retryable); the
/// helper or the control plane being away (retryable).
pub async fn send_file(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::SendFile {
        tenant_id,
        uid,
        transfer_id,
        path,
        url,
        max_bytes,
        token,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    if !plain_url(&url) {
        return Err(invalid("the upload address must be https"));
    }
    let token = token
        .ok_or_else(|| invalid("no credential for the upload"))?
        .0;
    let dir = transfers_dir(&context);
    std::fs::create_dir_all(&dir)
        .map_err(|e| failed(&format!("cannot make the folder: {}", e.kind())).retryable())?;
    let name = format!("{}.out", transfer_id.simple());
    let staged = dir.join(&name);
    let _ = std::fs::remove_file(&staged);
    let socket = context.helper_socket.clone();
    let action = Action::TakeFile {
        tenant_id: tenant_id.to_string(),
        uid,
        path,
        staged: name,
        max_bytes,
    };
    tokio::task::spawn_blocking(move || ask_helper(&socket, action))
        .await
        .map_err(|_| failed("the reading was cancelled"))??;
    let staged2 = staged.clone();
    let sent = tokio::task::spawn_blocking(move || upload(&url, &token, &staged2))
        .await
        .map_err(|_| failed("the upload was cancelled"))?;
    let _ = std::fs::remove_file(&staged);
    Ok(Output::FileSent { size_bytes: sent? })
}

/// Removes what a crashed transfer left behind (older than a day).
pub fn sweep(context: &TenantContext) {
    let Ok(entries) = std::fs::read_dir(transfers_dir(context)) else {
        return;
    };
    for item in entries.flatten() {
        let old = item
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(86_400));
        if old {
            let _ = std::fs::remove_file(item.path());
        }
    }
}
