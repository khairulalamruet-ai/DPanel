//! Which database server of the node an operation works on (T07.13): the node's shared one, or
//! a tenant's own, which runs in a container of its own on that tenant's network.
//!
//! The choice travels with the task: [`scope`] runs an operation's code for one server, and the
//! functions here tell that code the server's names (container, volume, network, secret file,
//! port). Code that starts its own thread must read what it needs first; the scope does not
//! reach it.

use std::{
    future::Future,
    os::unix::fs::DirBuilderExt as _,
    path::{Path, PathBuf},
};

use builtpanel_node_protocol::{AgentError, ErrorCode};

use super::tenant::TenantContext;

tokio::task_local! {
    static TENANT: Option<String>;
}

/// Runs `work` for a server: a tenant's own (`Some(system name)`) or the shared one (`None`).
pub async fn scope<F: Future>(tenant: Option<String>, work: F) -> F::Output {
    TENANT.scope(tenant, work).await
}

/// The tenant whose own server this task works on; none for the shared server.
#[must_use]
pub fn current() -> Option<String> {
    TENANT.try_with(Clone::clone).unwrap_or(None)
}

/// The kinds of server, as far as names go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// MariaDB.
    Mariadb,
    /// PostgreSQL.
    Postgres,
}

impl Kind {
    fn short(self) -> &'static str {
        match self {
            Self::Mariadb => "db",
            Self::Postgres => "pg",
        }
    }

    fn secret(self) -> &'static str {
        match self {
            Self::Mariadb => "mariadb-root",
            Self::Postgres => "postgres-root",
        }
    }
}

/// The container's name.
#[must_use]
pub fn container(kind: Kind, shared: &str) -> String {
    match current() {
        Some(t) => format!("bp-{t}-{}", kind.short()),
        None => shared.to_owned(),
    }
}

/// The data volume's name.
#[must_use]
pub fn volume(kind: Kind, shared: &str) -> String {
    match current() {
        Some(t) => format!("bp-{t}-{}-data", kind.short()),
        None => shared.to_owned(),
    }
}

/// The name of the secret file holding the server's administrator password.
#[must_use]
pub fn secret_name(kind: Kind) -> String {
    match current() {
        Some(t) => format!("{}-{t}", kind.secret()),
        None => kind.secret().to_owned(),
    }
}

/// Where the administrator password is kept.
#[must_use]
pub fn secret_dir(context: &TenantContext) -> PathBuf {
    context.vhosts_root.with_file_name("secrets")
}

/// The port a tenant's own server is published on (on the node's loopback address): made once
/// from the free ports of a range, then kept in a file so that it is the same after a restart.
///
/// # Errors
/// The disk failing (retryable), or no free port.
pub fn dedicated_port(
    context: &TenantContext,
    kind: Kind,
    tenant: &str,
) -> Result<u16, AgentError> {
    let dir = context.vhosts_root.with_file_name("dedicated-ports");
    let disk = |e: &std::io::Error| {
        AgentError::new(
            ErrorCode::Internal,
            format!("cannot keep the server's port: {}", e.kind()),
        )
        .retryable()
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|e| disk(&e))?;
    let file = dir.join(format!("{tenant}-{}", kind.short()));
    if let Some(port) = read_port(&file) {
        return Ok(port);
    }
    let taken: Vec<u16> = std::fs::read_dir(&dir)
        .map_err(|e| disk(&e))?
        .filter_map(Result::ok)
        .filter_map(|entry| read_port(&entry.path()))
        .collect();
    let port = (30_000..40_000u16)
        .find(|p| !taken.contains(p) && std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
        .ok_or_else(|| AgentError::new(ErrorCode::Internal, "no free port for the server"))?;
    std::fs::write(&file, port.to_string()).map_err(|e| disk(&e))?;
    Ok(port)
}

fn read_port(path: &Path) -> Option<u16> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .filter(|p| *p >= 1024)
}

/// Forgets the port of a tenant's server of one kind (the server is gone).
pub fn forget_port(context: &TenantContext, kind: Kind, tenant: &str) {
    let dir = context.vhosts_root.with_file_name("dedicated-ports");
    let _ = std::fs::remove_file(dir.join(format!("{tenant}-{}", kind.short())));
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn names_follow_the_server_the_task_works_on() {
        assert_eq!(container(Kind::Mariadb, "bp-db"), "bp-db");
        assert_eq!(current(), None);
        scope(Some("alice".into()), async {
            assert_eq!(current().as_deref(), Some("alice"));
            assert_eq!(container(Kind::Mariadb, "bp-db"), "bp-alice-db");
            assert_eq!(container(Kind::Postgres, "bp-pg"), "bp-alice-pg");
            assert_eq!(volume(Kind::Postgres, "bp-pg-data"), "bp-alice-pg-data");
            assert_eq!(secret_name(Kind::Mariadb), "mariadb-root-alice");
        })
        .await;
        assert_eq!(secret_name(Kind::Postgres), "postgres-root");
    }
}
