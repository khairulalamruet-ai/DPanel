//! `read_log` (T10.04): the end of an account's web, error or PHP log.
//!
//! The logs are what the account's containers write to standard output and error, which
//! Docker keeps: nginx writes requests to the first and its errors to the second, PHP-FPM
//! its errors to the second. Nothing else can be named: the source is one of three words and
//! the container is the account's own (`bp-<name>-web`, `bp-<name>-php`).
//! A live view is made by asking again with the time of the last line.

use std::sync::Arc;

use builtpanel_node_protocol::{AgentError, LogLine, LogSource, MAX_LOG_LINES, Operation, Output};

use super::{
    sqlimport::{failed, invalid},
    tenant::{TenantContext, docker_error},
};

/// `read_log`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable), the account has no such container, or
/// Docker failing.
pub async fn read_log(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::ReadLog {
        username,
        source,
        since_ms,
        tail,
        contains,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let tenant =
        builtpanel_domain::TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if !(1..=MAX_LOG_LINES).contains(&tail) || since_ms < 0 {
        return Err(invalid("the number of lines or the time is out of range"));
    }
    let needle = contains.map(|c| c.to_lowercase()).filter(|c| !c.is_empty());
    if needle
        .as_ref()
        .is_some_and(|c| c.chars().count() > 200 || c.contains('\0'))
    {
        return Err(invalid("the search text is not usable"));
    }
    let (container, stderr) = match source {
        LogSource::WebAccess => (builtpanel_tenant_stack::web_container_name(&tenant), false),
        LogSource::WebError => (builtpanel_tenant_stack::web_container_name(&tenant), true),
        LogSource::PhpError => (builtpanel_tenant_stack::php_container_name(&tenant), true),
    };
    let container = container.map_err(|e| failed(&e.to_string()))?;
    // Docker counts both streams, and a search needs matches: read plenty, then pick.
    let asked = MAX_LOG_LINES * 5;
    let read = context
        .docker
        .logs(&container, since_ms, asked)
        .await
        .map_err(|e| docker_error(&e))?;
    let mut lines: Vec<LogLine> = read
        .lines
        .into_iter()
        .filter(|l| l.stderr == stderr)
        .filter(|l| {
            needle
                .as_ref()
                .is_none_or(|n| l.text.to_lowercase().contains(n))
        })
        .map(|l| LogLine {
            at_ms: l.at_ms,
            text: l.text.chars().take(4000).collect(),
        })
        .collect();
    let keep = usize::try_from(tail).unwrap_or(usize::MAX);
    let truncated = read.truncated || lines.len() > keep;
    if lines.len() > keep {
        lines.drain(..lines.len() - keep);
    }
    Ok(Output::LogLines { lines, truncated })
}
