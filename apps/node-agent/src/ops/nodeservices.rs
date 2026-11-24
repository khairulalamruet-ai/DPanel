//! The node's core services (T10.13): state, restart and log.
//!
//! Only the names in `NODE_SERVICES` exist as far as these operations go: the database
//! server, phpMyAdmin and the file server, each a container of the node with a fixed name.
//! The web server (Caddy) is checked through its admin address (see `web.rs`), and the agent
//! reports itself.

use std::{sync::Arc, time::Duration};

use builtpanel_docker_manager::{DockerError, Name};
use builtpanel_node_protocol::{
    AgentError, LogLine, MAX_LOG_LINES, NODE_SERVICES, Operation, Output, ServiceStatus,
};

use super::{
    sqlimport::invalid,
    tenant::{TenantContext, docker_error},
};

const GRACE: Duration = Duration::from_secs(20);

/// The containers of the core services: service name and container name.
pub const CONTAINERS: [(&str, &str); 3] = [
    ("database", super::db::CONTAINER),
    ("phpmyadmin", super::pma::CONTAINER),
    ("sftp", super::ftp::CONTAINER),
];

/// `report_services`.
///
/// # Errors
/// [`AgentError`]: none in practice (a container that cannot be asked about is `unreachable`).
pub async fn report_services(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    report_services_of(context, op, &CONTAINERS).await
}

/// [`report_services`] for an explicit list of containers (what a test uses, so as not to
/// touch the node's real ones).
///
/// # Errors
/// As [`report_services`].
pub async fn report_services_of(
    context: Arc<TenantContext>,
    op: Operation,
    containers: &[(&str, &str)],
) -> Result<Output, AgentError> {
    let Operation::ReportServices = op else {
        return Err(super::wrong_operation());
    };
    let mut items = vec![ServiceStatus {
        name: "agent".into(),
        state: "running".into(),
        detail: format!("version {}", env!("CARGO_PKG_VERSION")),
    }];
    for (service, container) in containers {
        let Ok(name) = Name::parse(container) else {
            continue;
        };
        let (state, detail) = match context.docker.inspect(&name).await {
            Ok(info) if info.running => ("running", info.status),
            Ok(info) => (
                "stopped",
                match info.exit_code {
                    Some(code) => format!("{} (exit code {code})", info.status),
                    None => info.status,
                },
            ),
            Err(DockerError::NotFound) => ("not_installed", "no such container".to_owned()),
            Err(e) => ("unreachable", e.to_string()),
        };
        items.push(ServiceStatus {
            name: (*service).into(),
            state: state.into(),
            detail,
        });
    }
    Ok(Output::Services { items })
}

fn known(name: &str, containers: &[(&str, &str)]) -> Result<Name, AgentError> {
    let (_, container) = containers
        .iter()
        .find(|(service, _)| *service == name && NODE_SERVICES.contains(service))
        .ok_or_else(|| invalid("not a core service of the node"))?;
    Name::parse(container).map_err(|_| invalid("not a core service of the node"))
}

/// `restart_node_service`.
///
/// # Errors
/// [`AgentError`]: a name that is not a core service (not retryable), or Docker failing.
pub async fn restart_node_service(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    restart_node_service_of(context, op, &CONTAINERS).await
}

/// [`restart_node_service`] for an explicit list of containers.
///
/// # Errors
/// As [`restart_node_service`].
pub async fn restart_node_service_of(
    context: Arc<TenantContext>,
    op: Operation,
    containers: &[(&str, &str)],
) -> Result<Output, AgentError> {
    let Operation::RestartNodeService { name } = op else {
        return Err(super::wrong_operation());
    };
    let container = known(&name, containers)?;
    context
        .docker
        .restart(&container, GRACE)
        .await
        .map_err(|e| match e {
            DockerError::NotFound => invalid("this service is not installed on the node"),
            other => docker_error(&other),
        })?;
    let state = context
        .docker
        .inspect(&container)
        .await
        .map(|i| i.status)
        .unwrap_or_default();
    Ok(Output::ContainerRestarted { name, state })
}

/// `read_service_log`.
///
/// # Errors
/// [`AgentError`]: a name that is not a core service or a count out of range (not
/// retryable), or Docker failing.
pub async fn read_service_log(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    read_service_log_of(context, op, &CONTAINERS).await
}

/// [`read_service_log`] for an explicit list of containers.
///
/// # Errors
/// As [`read_service_log`].
pub async fn read_service_log_of(
    context: Arc<TenantContext>,
    op: Operation,
    containers: &[(&str, &str)],
) -> Result<Output, AgentError> {
    let Operation::ReadServiceLog { name, tail } = op else {
        return Err(super::wrong_operation());
    };
    if !(1..=MAX_LOG_LINES).contains(&tail) {
        return Err(invalid("the number of lines is out of range"));
    }
    let container = known(&name, containers)?;
    let read = context
        .docker
        .logs(&container, 0, tail)
        .await
        .map_err(|e| match e {
            DockerError::NotFound => invalid("this service is not installed on the node"),
            other => docker_error(&other),
        })?;
    let keep = usize::try_from(tail).unwrap_or(usize::MAX);
    let mut lines: Vec<LogLine> = read
        .lines
        .into_iter()
        .map(|l| LogLine {
            at_ms: l.at_ms,
            text: l.text.chars().take(4000).collect(),
        })
        .collect();
    let truncated = read.truncated || lines.len() > keep;
    if lines.len() > keep {
        lines.drain(..lines.len() - keep);
    }
    Ok(Output::LogLines { lines, truncated })
}
