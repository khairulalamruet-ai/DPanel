//! The node's shared database server (T07.01).
//!
//! One MariaDB container per node, named `bp-db`, with its data in the volume
//! `bp-db-data`. It is published on the node's loopback address only (the agent
//! reaches it there; nothing outside the node can), and it joins the network of
//! each tenant that may use databases, as `db`, so a tenant's containers reach it
//! and nobody else's do. Its root password is made here, kept in the agent's
//! secret store (`<state>/secrets/mariadb-root`, 0600) and never leaves the node.

use std::{
    io::Read as _,
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use builtpanel_docker_manager::{
    Capability, ContainerSpec, DockerError, Name, Network, PublishedPort, Restart, Tmpfs,
    VolumeMount,
};
use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{AgentError, ErrorCode, Operation, Output};
use builtpanel_tenant_stack::{Allowlist, Role, network_name};

use super::tenant::{TenantContext, docker_error};

/// The database container.
pub const CONTAINER: &str = "bp-db";
/// Its network (the tenants' networks are added to it as it is used).
pub(super) const NETWORK: &str = "bp-db-net";
/// Its data.
const VOLUME: &str = "bp-db-data";
/// The loopback port it is published on unless `BUILTPANEL_AGENT_DB_PORT` says another.
pub const DEFAULT_PORT: u16 = 3307;
/// The public port unless `BUILTPANEL_AGENT_DB_PUBLIC_PORT` says another.
pub const DEFAULT_PUBLIC_PORT: u16 = 13306;
/// The label that says the container publishes its public port (and which).
const PUBLIC_LABEL: &str = "bp-db-public";
/// The name tenants reach it by on their own network.
pub const ALIAS: &str = "db";
/// Longest wait for it to accept connections.
const READY_TIMEOUT: Duration = Duration::from_secs(120);

const MIB: u64 = 1024 * 1024;

/// The loopback port of the database server.
#[must_use]
pub fn port() -> u16 {
    std::env::var("BUILTPANEL_AGENT_DB_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p >= 1024)
        .unwrap_or(DEFAULT_PORT)
}

/// The port the server is published on for clients outside the node, when remote
/// access is on (`BUILTPANEL_AGENT_DB_PUBLIC_PORT`; the privileged helper's
/// `--db-port` must be the same).
#[must_use]
pub fn public_port() -> u16 {
    std::env::var("BUILTPANEL_AGENT_DB_PUBLIC_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p >= 1024)
        .unwrap_or(DEFAULT_PUBLIC_PORT)
}

/// Where the root password is kept: next to the agent's other secrets.
pub(super) fn secrets_dir(context: &TenantContext) -> PathBuf {
    context.vhosts_root.with_file_name("secrets")
}

fn disk(what: &str, err: &std::io::Error) -> AgentError {
    AgentError::new(
        ErrorCode::Internal,
        format!("cannot {what}: {}", err.kind()),
    )
    .retryable()
}

/// 64 hex digits from the system's random source.
fn random_secret() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The root password: read from the secret store, made first if there is none.
///
/// # Errors
/// The disk failing (retryable).
pub fn root_password(dir: &Path) -> Result<String, AgentError> {
    secret_file(dir, "mariadb-root")
}

/// A random secret kept in the agent's secret store under this file name (0600, in a 0700
/// directory): read if it is there and long enough, made first if not.
///
/// # Errors
/// The disk failing (retryable).
pub fn secret_file(dir: &Path, file: &str) -> Result<String, AgentError> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| disk("prepare the secret store", &e))?;
    let path = dir.join(file);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let text = text.trim().to_owned();
        if text.len() >= 32 {
            return Ok(text);
        }
    }
    let secret = random_secret().map_err(|e| disk("make a password", &e))?;
    let tmp = dir.join(format!(".{file}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let write = || -> std::io::Result<()> {
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        std::io::Write::write_all(&mut out, secret.as_bytes())?;
        out.sync_all()?;
        std::fs::rename(&tmp, &path)
    };
    write().map_err(|e| disk("keep the password", &e))?;
    Ok(secret)
}

/// The name of the server's container for the server this task works on.
#[must_use]
pub fn container_name() -> String {
    super::dbserver::container(super::dbserver::Kind::Mariadb, CONTAINER)
}

/// The loopback port of the server this task works on (the shared one, or a tenant's own).
///
/// # Errors
/// The disk failing, when a tenant's own server has no port yet.
pub fn server_port(context: &TenantContext) -> Result<u16, AgentError> {
    match super::dbserver::current() {
        Some(tenant) => {
            super::dbserver::dedicated_port(context, super::dbserver::Kind::Mariadb, &tenant)
        }
        None => Ok(port()),
    }
}

/// The administrator password of the server this task works on.
fn server_password(context: &TenantContext) -> Result<String, AgentError> {
    secret_file(
        &secrets_dir(context),
        &super::dbserver::secret_name(super::dbserver::Kind::Mariadb),
    )
}

pub(super) fn name(text: &str) -> Result<Name, AgentError> {
    Name::parse(text).map_err(|e| AgentError::new(ErrorCode::Internal, e.to_string()))
}

/// The connection options for the root account on the loopback port.
fn root_opts(password: &str, port: u16) -> mysql_async::Opts {
    mysql_async::OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(port)
        .user(Some("root"))
        .pass(Some(password))
        .into()
}

/// Waits until the server answers a statement.
async fn wait_ready(password: &str, port: u16) -> Result<(), AgentError> {
    use mysql_async::prelude::Queryable as _;
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        let attempt = async {
            let mut conn = mysql_async::Conn::new(root_opts(password, port)).await?;
            conn.query_drop("SELECT 1").await?;
            conn.disconnect().await
        };
        if attempt.await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(AgentError::new(
                ErrorCode::Internal,
                "the database server did not come up in time",
            )
            .retryable());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Makes sure the server runs. Returns whether this call made the container.
///
/// # Errors
/// Docker or the disk failing (retryable), or the server not coming up.
pub async fn ensure_server(context: &TenantContext) -> Result<bool, AgentError> {
    ensure_server_with(context, false).await
}

/// [`ensure_server`], and with `public` the server also has to publish its public
/// port: a container that does not is made again (its data is in the volume).
///
/// # Errors
/// As [`ensure_server`].
pub async fn ensure_server_with(context: &TenantContext, public: bool) -> Result<bool, AgentError> {
    let password = server_password(context)?;
    let docker = &context.docker;
    let container = name(&container_name())?;
    let dedicated = super::dbserver::current();
    let server_port = server_port(context)?;
    if dedicated.is_some() && public {
        return Err(AgentError::new(
            ErrorCode::InvalidArguments,
            "a tenant's own database server has no public port",
        ));
    }
    let wanted_label = public_port().to_string();
    let mut existing = docker.inspect(&container).await;
    if let Ok(info) = &existing
        && public
        && info.labels.get(PUBLIC_LABEL) != Some(&wanted_label)
    {
        docker
            .stop(&container, Duration::from_secs(30))
            .await
            .map_err(|e| docker_error(&e))?;
        docker
            .remove(&container, true)
            .await
            .map_err(|e| docker_error(&e))?;
        existing = Err(DockerError::NotFound);
    }
    let created = match existing {
        Ok(_) => false,
        Err(DockerError::NotFound) => {
            let allowlist = Allowlist::embedded().map_err(|e| {
                AgentError::new(
                    ErrorCode::Internal,
                    format!("the image list is broken: {e}"),
                )
            })?;
            let image = allowlist
                .current(Role::Db)
                .map_err(|e| AgentError::new(ErrorCode::Internal, e.to_string()))?
                .clone();
            // The server has a network of its own that tenants are added to (the shared one, or
            // one tenant's own server, which only that tenant is added to).
            let network = match &dedicated {
                Some(tenant) => name(&format!("bp-{tenant}-dbnet"))?,
                None => name(NETWORK)?,
            };
            docker
                .ensure_network(&network, false)
                .await
                .map_err(|e| docker_error(&e))?;
            let volume = name(&super::dbserver::volume(
                super::dbserver::Kind::Mariadb,
                VOLUME,
            ))?;
            docker
                .create_volume(&volume)
                .await
                .map_err(|e| docker_error(&e))?;
            let image = super::tenant::pull_with_fallback(docker, &allowlist, &image).await?;
            let mut spec = ContainerSpec::new(container.clone(), image);
            spec.command = vec![
                "--skip-name-resolve=1".into(),
                "--local-infile=0".into(),
                "--skip-symbolic-links".into(),
                "--max-connections=300".into(),
                "--max-allowed-packet=64M".into(),
            ];
            spec.env
                .insert("MARIADB_ROOT_PASSWORD".into(), password.clone());
            spec.env.insert("MARIADB_AUTO_UPGRADE".into(), "1".into());
            // The image starts as root and gives its own files to `mysql`: it needs
            // exactly the capabilities for that, and nothing else.
            spec.cap_add = vec![
                Capability::Chown,
                Capability::DacOverride,
                Capability::Fowner,
                Capability::Setgid,
                Capability::Setuid,
            ];
            // A tenant's own server is smaller than the node's shared one.
            let share = if dedicated.is_some() { 2 } else { 1 };
            spec.memory_bytes = 1024 * MIB / share;
            spec.cpu_millis = 2000 / share as u32;
            spec.pids_limit = 512 / share as u32;
            spec.tmpfs = vec![
                Tmpfs {
                    target: "/tmp".into(),
                    size_bytes: 256 * MIB,
                },
                Tmpfs {
                    target: "/run/mysqld".into(),
                    size_bytes: 16 * MIB,
                },
            ];
            spec.volumes = vec![VolumeMount {
                volume,
                target: "/var/lib/mysql".into(),
                read_only: false,
            }];
            spec.network = Network::Named(network);
            let bad = || AgentError::new(ErrorCode::Internal, "bad address");
            spec.ports = vec![PublishedPort {
                container_port: 3306,
                host_port: server_port,
                host_ip: "127.0.0.1".parse().map_err(|_| bad())?,
            }];
            if public {
                // Only ever after the packet filter has been set (see
                // `set_db_remote_access`): the port is open to nobody until it is.
                spec.ports.push(PublishedPort {
                    container_port: 3306,
                    host_port: public_port(),
                    host_ip: "0.0.0.0".parse().map_err(|_| bad())?,
                });
                spec.labels
                    .insert(PUBLIC_LABEL.to_owned(), wanted_label.clone());
            }
            spec.restart = Restart::UnlessStopped;
            match docker.create_container(&spec).await {
                Ok(_) | Err(DockerError::Conflict(_)) => {}
                Err(e) => return Err(docker_error(&e)),
            }
            true
        }
        Err(e) => return Err(docker_error(&e)),
    };
    docker
        .start(&container)
        .await
        .map_err(|e| docker_error(&e))?;
    if created && let Some(tenant) = &dedicated {
        // Found as `db` on the tenant's network, like the shared server.
        let network = network_name(
            &TenantUsername::parse(tenant)
                .map_err(|e| AgentError::new(ErrorCode::InvalidArguments, e.to_string()))?,
        )
        .map_err(|e| AgentError::new(ErrorCode::InvalidArguments, e.to_string()))?;
        let _ = docker.connect_network(&container, &network, ALIAS).await;
    }
    wait_ready(&password, server_port).await?;
    Ok(created)
}

/// A connection as the server's root, from the agent's secret store.
async fn root_conn(context: &TenantContext) -> Result<mysql_async::Conn, AgentError> {
    let password = server_password(context)?;
    mysql_async::Conn::new(root_opts(&password, server_port(context)?))
        .await
        .map_err(|e| sql_error(&e))
}

/// A connection as the server's root (for the import code).
///
/// # Errors
/// As [`ensure_server`].
pub async fn root_connection(context: &TenantContext) -> Result<mysql_async::Conn, AgentError> {
    root_conn(context).await
}

/// What went wrong with a statement, said without quoting anything the server sent
/// back (it may echo values).
fn sql_error(err: &mysql_async::Error) -> AgentError {
    let (message, retryable) = match err {
        mysql_async::Error::Io(_) => ("the database server is not reachable", true),
        mysql_async::Error::Server(e) if e.code == 1045 => {
            ("the database server refused the agent's password", false)
        }
        mysql_async::Error::Server(_) => ("the database server refused the statement", false),
        _ => ("the database server failed", true),
    };
    let error = AgentError::new(ErrorCode::Internal, message.to_owned());
    if retryable { error.retryable() } else { error }
}

fn identifier(name: &str) -> Result<String, AgentError> {
    builtpanel_domain::databases::check_identifier(name)
        .map_err(|e| AgentError::new(ErrorCode::InvalidArguments, e.to_string()))?;
    // Checked as plain letters, digits and `_` above, so nothing in it needs
    // escaping; the quotes keep a word the server knows from being read as one.
    Ok(format!("`{name}`"))
}

/// `create_database`.
///
/// # Errors
/// [`AgentError`]: a name that is not a plain identifier (not retryable, nothing
/// is sent to the server); the server not reachable (retryable).
pub async fn create_database(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    use mysql_async::prelude::Queryable as _;
    let Operation::CreateDatabase { name, .. } = op else {
        return Err(super::wrong_operation());
    };
    let quoted = identifier(&name)?;
    // The first database of a node starts its server.
    ensure_server(&context).await?;
    let mut conn = root_conn(&context).await?;
    let existed: Option<String> = conn
        .exec_first(
            "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = ?",
            (&name,),
        )
        .await
        .map_err(|e| sql_error(&e))?;
    if existed.is_none() {
        conn.query_drop(format!(
            "CREATE DATABASE {quoted} CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci"
        ))
        .await
        .map_err(|e| sql_error(&e))?;
    }
    let _ = conn.disconnect().await;
    Ok(Output::DatabaseCreated {
        created: existed.is_none(),
    })
}

/// `drop_database`.
///
/// # Errors
/// As [`create_database`].
pub async fn drop_database(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    use mysql_async::prelude::Queryable as _;
    let Operation::DropDatabase { name, .. } = op else {
        return Err(super::wrong_operation());
    };
    let quoted = identifier(&name)?;
    let mut conn = root_conn(&context).await?;
    let existed: Option<String> = conn
        .exec_first(
            "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = ?",
            (&name,),
        )
        .await
        .map_err(|e| sql_error(&e))?;
    if existed.is_some() {
        // The server keeps what users were granted on a database after it is gone: a
        // database made later under the same name would hand it back. Take it away.
        let who: Vec<(String, String)> = conn
            .exec("SELECT User, Host FROM mysql.db WHERE Db = ?", (&name,))
            .await
            .map_err(|e| sql_error(&e))?;
        for (user, host) in who {
            if let (Ok(u), Ok(h)) = (user_literal(&user), host_literal(&host)) {
                revoke_all(&mut conn, &quoted, &format!("{u}@{h}")).await?;
            }
        }
        conn.query_drop(format!("DROP DATABASE {quoted}"))
            .await
            .map_err(|e| sql_error(&e))?;
    }
    // The locked account that defines what an import made (T07.07), if there is one.
    conn.query_drop(format!(
        "DROP USER IF EXISTS '{}'@'%'",
        super::sqlimport::definer_account(&name)
    ))
    .await
    .map_err(|e| sql_error(&e))?;
    let _ = conn.disconnect().await;
    Ok(Output::DatabaseDropped {
        existed: existed.is_some(),
    })
}

/// A user's name as a statement writes it. (The same plain-identifier rule as for
/// databases; checked, then quoted.)
fn user_literal(name: &str) -> Result<String, AgentError> {
    builtpanel_domain::databases::check_identifier(name)
        .map_err(|e| AgentError::new(ErrorCode::InvalidArguments, e.to_string()))?;
    Ok(format!("'{name}'"))
}

/// A host part, `ip/netmask`, as a statement writes it: only digits, dots and one
/// slash (checked), so nothing in it can end the literal.
fn host_literal(host: &str) -> Result<String, AgentError> {
    let ok = !host.is_empty()
        && host.len() <= 40
        && host
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b'/');
    if !ok {
        return Err(AgentError::new(
            ErrorCode::Internal,
            "the network address is not usable",
        ));
    }
    Ok(format!("'{host}'"))
}

/// The server's own hash of a password, made with a bound parameter, and checked to
/// be what a hash looks like before it is put into a statement (the server does
/// not accept parameters in `CREATE USER`, and the password is never written into
/// a statement itself).
async fn password_hash(conn: &mut mysql_async::Conn, password: &str) -> Result<String, AgentError> {
    use mysql_async::prelude::Queryable as _;
    let hash: Option<String> = conn
        .exec_first("SELECT PASSWORD(?)", (password,))
        .await
        .map_err(|e| sql_error(&e))?;
    hash.filter(|h| {
        h.len() == 41 && h.starts_with('*') && h[1..].bytes().all(|b| b.is_ascii_hexdigit())
    })
    .ok_or_else(|| {
        AgentError::new(
            ErrorCode::Internal,
            "the database server gave no password hash",
        )
    })
}

/// Every host a user exists for.
async fn hosts_of(conn: &mut mysql_async::Conn, name: &str) -> Result<Vec<String>, AgentError> {
    use mysql_async::prelude::Queryable as _;
    conn.exec("SELECT Host FROM mysql.user WHERE User = ?", (name,))
        .await
        .map_err(|e| sql_error(&e))
}

/// `upsert_db_user`.
///
/// # Errors
/// [`AgentError`]: a bad name or password, or a new user without one (not
/// retryable); the tenant's network not found; the server not reachable (retryable).
pub async fn upsert_db_user(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    use mysql_async::prelude::Queryable as _;
    let Operation::UpsertDbUser {
        username,
        name,
        password,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let invalid = |m: &str| AgentError::new(ErrorCode::InvalidArguments, m.to_owned());
    let user = user_literal(&name)?;
    let tenant = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if let Some(p) = &password {
        builtpanel_domain::databases::check_password(&p.0).map_err(|e| invalid(&e.to_string()))?;
    }
    // Where it may connect from: its own tenant's network, never `%`.
    let network = network_name(&tenant).map_err(|e| invalid(&e.to_string()))?;
    let subnet = context
        .docker
        .network_subnet(&network)
        .await
        .map_err(|e| match e {
            DockerError::NotFound => invalid("the tenant has no network on this node"),
            other => docker_error(&other),
        })?;
    let host = builtpanel_domain::databases::host_for_network(&subnet).ok_or_else(|| {
        AgentError::new(
            ErrorCode::Internal,
            "the tenant's network range is not usable",
        )
    })?;
    let quoted_host = host_literal(&host)?;
    // The server runs, and this tenant's containers can reach it.
    ensure_server(&context).await?;
    connect_tenant(&context, &network).await?;
    let mut conn = root_conn(&context).await?;
    let existing = hosts_of(&mut conn, &name).await?;
    let here = existing.contains(&host);
    let created = existing.is_empty();
    match (&password, here) {
        (None, false) => {
            return Err(invalid("a password is needed for a user that is new here"));
        }
        (Some(p), false) => {
            let hash = password_hash(&mut conn, &p.0).await?;
            conn.query_drop(format!(
                "CREATE USER {user}@{quoted_host} IDENTIFIED BY PASSWORD '{hash}' WITH MAX_USER_CONNECTIONS 50"
            ))
            .await
            .map_err(|e| sql_error(&e))?;
        }
        (_, true) => {}
    }
    if let (Some(p), true) = (&password, here) {
        let hash = password_hash(&mut conn, &p.0).await?;
        for h in &existing {
            conn.query_drop(format!(
                "ALTER USER {user}@{} IDENTIFIED BY PASSWORD '{hash}'",
                host_or_pattern(h)?
            ))
            .await
            .map_err(|e| sql_error(&e))?;
        }
    }
    let _ = conn.disconnect().await;
    Ok(Output::DbUserSet { created, host })
}

/// The password hash of a user at its tenant's network, as the server holds it (so that the
/// user can be made again on another server with the same password).
///
/// # Errors
/// The server failing.
pub(super) async fn user_hash(
    context: &TenantContext,
    tenant: &TenantUsername,
    name: &str,
) -> Result<Option<String>, AgentError> {
    use mysql_async::prelude::Queryable as _;
    user_literal(name)?;
    let own = tenant_host(context, tenant).await?;
    let mut conn = root_conn(context).await?;
    let hash: Option<String> = conn
        .exec_first(
            "SELECT Password FROM mysql.user WHERE User = ? AND Host = ?",
            (name, &own),
        )
        .await
        .map_err(|e| sql_error(&e))?;
    let _ = conn.disconnect().await;
    Ok(hash.filter(|h| {
        h.len() == 41 && h.starts_with('*') && h[1..].bytes().all(|b| b.is_ascii_hexdigit())
    }))
}

/// Makes a user at its tenant's network with a password hash that came from another server,
/// or gives an existing one that hash.
///
/// # Errors
/// The server failing, or the tenant's network not found.
pub(super) async fn set_user_hash(
    context: &TenantContext,
    tenant: &TenantUsername,
    name: &str,
    hash: &str,
) -> Result<(), AgentError> {
    use mysql_async::prelude::Queryable as _;
    let user = user_literal(name)?;
    let valid = hash.len() == 41
        && hash.starts_with('*')
        && hash[1..].bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        return Err(AgentError::new(
            ErrorCode::Internal,
            "the password hash is not usable",
        ));
    }
    let network = network_name(tenant)
        .map_err(|e| AgentError::new(ErrorCode::InvalidArguments, e.to_string()))?;
    let own = tenant_host(context, tenant).await?;
    let quoted_host = host_literal(&own)?;
    ensure_server(context).await?;
    connect_tenant(context, &network).await?;
    let mut conn = root_conn(context).await?;
    let existing = hosts_of(&mut conn, name).await?;
    if existing.contains(&own) {
        conn.query_drop(format!(
            "ALTER USER {user}@{quoted_host} IDENTIFIED BY PASSWORD '{hash}'"
        ))
        .await
        .map_err(|e| sql_error(&e))?;
    } else {
        conn.query_drop(format!(
            "CREATE USER {user}@{quoted_host} IDENTIFIED BY PASSWORD '{hash}' WITH MAX_USER_CONNECTIONS 50"
        ))
        .await
        .map_err(|e| sql_error(&e))?;
    }
    let _ = conn.disconnect().await;
    Ok(())
}

/// A host the server holds, as a literal: the ones this agent made are digits,
/// dots and a slash; anything else (someone made it by hand) is left alone.
fn host_or_pattern(host: &str) -> Result<String, AgentError> {
    host_literal(host)
}

/// `grant_db`.
///
/// # Errors
/// [`AgentError`]: a bad name or access level (not retryable); the user does not
/// exist; the server not reachable (retryable).
pub async fn grant_db(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    use mysql_async::prelude::Queryable as _;
    let Operation::GrantDb {
        database,
        user,
        access,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let level = builtpanel_domain::databases::Access::parse(&access)
        .ok_or_else(|| AgentError::new(ErrorCode::InvalidArguments, "an unknown access level"))?;
    let db = identifier(&database)?;
    let who = user_literal(&user)?;
    let mut conn = root_conn(&context).await?;
    let hosts = hosts_of(&mut conn, &user).await?;
    if hosts.is_empty() {
        return Err(AgentError::new(
            ErrorCode::InvalidArguments,
            "the database user does not exist here",
        ));
    }
    let privileges = level.privileges().join(", ");
    for host in &hosts {
        let account = format!("{who}@{}", host_or_pattern(host)?);
        revoke_all(&mut conn, &db, &account).await?;
        conn.query_drop(format!("GRANT {privileges} ON {db}.* TO {account}"))
            .await
            .map_err(|e| sql_error(&e))?;
    }
    let _ = conn.disconnect().await;
    Ok(Output::DbGranted)
}

/// Takes everything the account had in the database away; having nothing is fine.
async fn revoke_all(
    conn: &mut mysql_async::Conn,
    db: &str,
    account: &str,
) -> Result<(), AgentError> {
    use mysql_async::prelude::Queryable as _;
    match conn
        .query_drop(format!("REVOKE ALL PRIVILEGES ON {db}.* FROM {account}"))
        .await
    {
        Ok(()) => Ok(()),
        // 1141: no such grant for that user and host; 1147: none on that table.
        Err(mysql_async::Error::Server(e)) if matches!(e.code, 1141 | 1147) => Ok(()),
        Err(e) => Err(sql_error(&e)),
    }
}

/// `revoke_db`.
///
/// # Errors
/// As [`grant_db`].
pub async fn revoke_db(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::RevokeDb { database, user, .. } = op else {
        return Err(super::wrong_operation());
    };
    let db = identifier(&database)?;
    let who = user_literal(&user)?;
    let mut conn = root_conn(&context).await?;
    for host in hosts_of(&mut conn, &user).await? {
        revoke_all(
            &mut conn,
            &db,
            &format!("{who}@{}", host_or_pattern(&host)?),
        )
        .await?;
    }
    let _ = conn.disconnect().await;
    Ok(Output::DbRevoked)
}

/// The tenant's own network as the host part of a user.
async fn tenant_host(
    context: &TenantContext,
    tenant: &TenantUsername,
) -> Result<String, AgentError> {
    let invalid = |m: &str| AgentError::new(ErrorCode::InvalidArguments, m.to_owned());
    let network = network_name(tenant).map_err(|e| invalid(&e.to_string()))?;
    let subnet = context
        .docker
        .network_subnet(&network)
        .await
        .map_err(|e| match e {
            DockerError::NotFound => invalid("the tenant has no network on this node"),
            other => docker_error(&other),
        })?;
    builtpanel_domain::databases::host_for_network(&subnet).ok_or_else(|| {
        AgentError::new(
            ErrorCode::Internal,
            "the tenant's network range is not usable",
        )
    })
}

/// The databases a user has a grant in on one host, with the privileges, read from
/// the server's own `SHOW GRANTS` and kept only where they are exactly what this
/// agent writes: a database name that is a plain identifier and privileges from the
/// fixed lists. Anything else in the output is ignored, never run.
async fn grants_on(
    conn: &mut mysql_async::Conn,
    user: &str,
    host: &str,
) -> Result<Vec<(String, Vec<String>)>, AgentError> {
    use mysql_async::prelude::Queryable as _;
    let known: std::collections::BTreeSet<&str> = builtpanel_domain::databases::Access::All
        .privileges()
        .iter()
        .copied()
        .collect();
    let lines: Vec<String> = conn
        .query(format!(
            "SHOW GRANTS FOR {}@{}",
            user_literal(user)?,
            host_literal(host)?
        ))
        .await
        .map_err(|e| sql_error(&e))?;
    let mut out = Vec::new();
    for line in lines {
        let Some(rest) = line.strip_prefix("GRANT ") else {
            continue;
        };
        let Some((privs, after)) = rest.split_once(" ON `") else {
            continue;
        };
        let Some((db, tail)) = after.split_once("`.* TO ") else {
            continue;
        };
        let _ = tail;
        let privileges: Vec<String> = privs.split(", ").map(str::to_owned).collect();
        if builtpanel_domain::databases::check_identifier(db).is_ok()
            && privileges.iter().all(|p| known.contains(p.as_str()))
        {
            out.push((db.to_owned(), privileges));
        }
    }
    Ok(out)
}

/// `report_db_sizes`.
///
/// # Errors
/// As [`create_database`].
pub async fn report_db_sizes(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    use mysql_async::prelude::Queryable as _;
    let Operation::ReportDbSizes = op else {
        return Err(super::wrong_operation());
    };
    let mut conn = root_conn(&context).await?;
    // The server's own schemas are not the panel's business.
    let rows: Vec<(String, u64)> = conn
        .query(
            "SELECT s.SCHEMA_NAME, CAST(COALESCE(SUM(t.DATA_LENGTH + t.INDEX_LENGTH), 0) AS UNSIGNED)
             FROM information_schema.SCHEMATA s
             LEFT JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = s.SCHEMA_NAME
             WHERE s.SCHEMA_NAME NOT IN ('mysql', 'information_schema', 'performance_schema', 'sys')
             GROUP BY s.SCHEMA_NAME ORDER BY s.SCHEMA_NAME",
        )
        .await
        .map_err(|e| sql_error(&e))?;
    let _ = conn.disconnect().await;
    Ok(Output::DbSizes {
        items: rows
            .into_iter()
            .take(builtpanel_node_protocol::MAX_DB_SIZES_PER_REPORT)
            .map(|(name, bytes)| builtpanel_node_protocol::DbSize { name, bytes })
            .collect(),
    })
}

/// `set_db_remote_hosts`.
///
/// # Errors
/// [`AgentError`]: a bad address or too many, a user that does not exist (not
/// retryable); the server not reachable (retryable).
pub async fn set_db_remote_hosts(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    use mysql_async::prelude::Queryable as _;
    let Operation::SetDbRemoteHosts {
        username,
        name,
        cidrs,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let invalid = |m: &str| AgentError::new(ErrorCode::InvalidArguments, m.to_owned());
    let user = user_literal(&name)?;
    let tenant = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if cidrs.len() > builtpanel_node_protocol::MAX_DB_REMOTE_HOSTS {
        return Err(invalid("too many remote addresses"));
    }
    let mut wanted: Vec<String> = Vec::new();
    for cidr in &cidrs {
        let host = builtpanel_domain::databases::host_for_remote(cidr)
            .ok_or_else(|| invalid("a remote address is IPv4 a.b.c.d/n, no wider than /8"))?;
        if !wanted.contains(&host) {
            wanted.push(host);
        }
    }
    let own = tenant_host(&context, &tenant).await?;
    let mut conn = root_conn(&context).await?;
    let existing = hosts_of(&mut conn, &name).await?;
    if !existing.contains(&own) {
        return Err(invalid("the database user does not exist for this tenant"));
    }
    // The password the user has (its hash) and the grants it has at home.
    let hash: Option<String> = conn
        .exec_first(
            "SELECT Password FROM mysql.user WHERE User = ? AND Host = ?",
            (&name, &own),
        )
        .await
        .map_err(|e| sql_error(&e))?;
    let hash = hash
        .filter(|h| {
            h.len() == 41 && h.starts_with('*') && h[1..].bytes().all(|b| b.is_ascii_hexdigit())
        })
        .ok_or_else(|| {
            AgentError::new(
                ErrorCode::Internal,
                "the user's password hash is not readable",
            )
        })?;
    let grants = grants_on(&mut conn, &name, &own).await?;
    for host in &wanted {
        let quoted = host_literal(host)?;
        if !existing.contains(host) {
            conn.query_drop(format!(
                "CREATE USER {user}@{quoted} IDENTIFIED BY PASSWORD '{hash}' WITH MAX_USER_CONNECTIONS 20"
            ))
            .await
            .map_err(|e| sql_error(&e))?;
        }
        // The same access as at home, as this agent's own statements.
        for (db, privileges) in &grants {
            let quoted_db = identifier(db)?;
            conn.query_drop(format!(
                "GRANT {} ON {quoted_db}.* TO {user}@{quoted}",
                privileges.join(", ")
            ))
            .await
            .map_err(|e| sql_error(&e))?;
        }
    }
    // The ones that were allowed and no longer are.
    for host in existing
        .iter()
        .filter(|h| **h != own && !wanted.contains(h))
    {
        conn.query_drop(format!("DROP USER {user}@{}", host_or_pattern(host)?))
            .await
            .map_err(|e| sql_error(&e))?;
    }
    let _ = conn.disconnect().await;
    Ok(Output::DbRemoteHostsSet {
        hosts: wanted.len(),
    })
}

/// `set_db_remote_access`.
///
/// # Errors
/// [`AgentError`]: a bad address or too many (not retryable); the privileged
/// helper or Docker not available (retryable). The public port is opened only after
/// the packet filter was set; when the helper refuses, nothing is opened.
pub async fn set_db_remote_access(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetDbRemoteAccess { cidrs, .. } = op else {
        return Err(super::wrong_operation());
    };
    let invalid = |m: &str| AgentError::new(ErrorCode::InvalidArguments, m.to_owned());
    if cidrs.len() > builtpanel_node_protocol::MAX_DB_REMOTE_ACCESS {
        return Err(invalid("too many address ranges"));
    }
    for cidr in &cidrs {
        if builtpanel_domain::databases::host_for_remote(cidr).is_none() {
            return Err(invalid(
                "an address range is not IPv4 a.b.c.d/n (n from 8 to 32)",
            ));
        }
    }
    // 1. The packet filter: closed to everyone but these.
    let socket = context.helper_socket.clone();
    let sent = cidrs.clone();
    tokio::task::spawn_blocking(move || {
        super::tenant::ask_helper(
            &socket,
            builtpanel_priv_helper::protocol::Action::SetDbFirewall { cidrs: sent },
        )
    })
    .await
    .map_err(|_| {
        AgentError::new(ErrorCode::Internal, "the helper call was cancelled").retryable()
    })??;
    // 2. Only then the port: open when there is anybody to let in.
    let published = if cidrs.is_empty() {
        false
    } else {
        ensure_server_with(&context, true).await?;
        true
    };
    Ok(Output::DbRemoteAccessSet {
        ranges: cidrs.len(),
        published,
    })
}

/// `drop_db_user`.
///
/// # Errors
/// As [`grant_db`].
pub async fn drop_db_user(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    use mysql_async::prelude::Queryable as _;
    let Operation::DropDbUser { name, .. } = op else {
        return Err(super::wrong_operation());
    };
    let who = user_literal(&name)?;
    let mut conn = root_conn(&context).await?;
    let hosts = hosts_of(&mut conn, &name).await?;
    for host in &hosts {
        conn.query_drop(format!("DROP USER {who}@{}", host_or_pattern(host)?))
            .await
            .map_err(|e| sql_error(&e))?;
    }
    let _ = conn.disconnect().await;
    Ok(Output::DbUserDropped {
        existed: !hosts.is_empty(),
    })
}

/// `ensure_db_server`.
///
/// # Errors
/// As [`ensure_server`].
pub async fn ensure_db_server(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::EnsureDbServer = op else {
        return Err(super::wrong_operation());
    };
    let created = ensure_server(&context).await?;
    Ok(Output::DbServerReady {
        port: port(),
        created,
    })
}

/// Puts the server on a tenant's network as `db`.
async fn connect_tenant(context: &TenantContext, network: &Name) -> Result<(), AgentError> {
    match context
        .docker
        .connect_network(&name(&container_name())?, network, ALIAS)
        .await
    {
        Ok(()) => Ok(()),
        Err(DockerError::NotFound) => Err(AgentError::new(
            ErrorCode::InvalidArguments,
            "the tenant has no network on this node",
        )),
        Err(e) => Err(docker_error(&e)),
    }
}

/// `allow_tenant_databases`.
///
/// # Errors
/// As [`ensure_server`]; a tenant that has no network here (not retryable).
pub async fn allow_tenant_databases(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::AllowTenantDatabases { username, .. } = op else {
        return Err(super::wrong_operation());
    };
    let username = TenantUsername::parse(&username)
        .map_err(|e| AgentError::new(ErrorCode::InvalidArguments, e.to_string()))?;
    let network = network_name(&username)
        .map_err(|e| AgentError::new(ErrorCode::InvalidArguments, e.to_string()))?;
    ensure_server(&context).await?;
    connect_tenant(&context, &network).await?;
    Ok(Output::TenantDbAllowed {
        network: network.as_str().to_owned(),
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn the_root_password_is_made_once_kept_private_and_read_back() {
        let dir = std::env::temp_dir().join(format!("bp-secrets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = root_password(&dir).unwrap();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(root_password(&dir).unwrap(), first, "kept, not made again");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("mariadb-root")), 0o600);
        // A damaged file is replaced rather than used.
        std::fs::write(dir.join("mariadb-root"), "short").unwrap();
        let second = root_password(&dir).unwrap();
        assert_eq!(second.len(), 64);
        assert_ne!(second, first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_port_is_a_loopback_port_above_1023() {
        assert_eq!(port(), DEFAULT_PORT);
    }
}
