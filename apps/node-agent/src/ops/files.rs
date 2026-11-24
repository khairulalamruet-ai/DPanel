//! `file_op` (T08.02-T08.06): one file manager request, done by the privileged
//! helper as the tenant's own user id (see `builtpanel_priv_helper::files`).

use std::sync::Arc;

use builtpanel_node_protocol::{
    AgentError, ErrorCode, Operation, Output,
    files::{FileErrorCode, FileResult},
};
use builtpanel_priv_helper::protocol::Action;

use super::tenant::{TenantContext, ask_helper_data};

/// Most an answer may be (a frame is 1 MiB, with the envelope around it).
const MAX_ANSWER_BYTES: usize = 900 * 1024;

/// `file_op`.
///
/// # Errors
/// [`AgentError`]: the helper is not there or refuses (retryable when it may work
/// later). A request that cannot be done (a missing file, a refused path) is not an
/// error here: it is the answer.
pub async fn file_op(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::FileOp {
        tenant_id,
        uid,
        request,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let socket = context.helper_socket.clone();
    let action = Action::FileOp {
        tenant_id: tenant_id.to_string(),
        uid,
        request,
    };
    let data = tokio::task::spawn_blocking(move || ask_helper_data(&socket, action))
        .await
        .map_err(|_| AgentError::new(ErrorCode::Internal, "the file request was cancelled"))??;
    let data = data.ok_or_else(|| {
        AgentError::new(
            ErrorCode::Internal,
            "the helper gave no answer to the file request",
        )
    })?;
    let result: FileResult = serde_json::from_value(data).map_err(|_| {
        AgentError::new(
            ErrorCode::Internal,
            "the helper's answer to the file request is unreadable",
        )
    })?;
    let too_big = serde_json::to_vec(&result).map_or(true, |v| v.len() > MAX_ANSWER_BYTES);
    Ok(Output::File {
        result: if too_big {
            FileResult::error(
                FileErrorCode::TooLarge,
                "the answer is too big to send; ask for less",
            )
        } else {
            result
        },
    })
}
