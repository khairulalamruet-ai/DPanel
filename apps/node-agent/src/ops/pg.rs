//! The node's PostgreSQL server (T07.12): the same jobs as [`super::db`] for MariaDB.
//!
//! One PostgreSQL container per node, named `bp-pg`, with its data in the volume
//! `bp-pg-data`. It is published on the node's loopback address only, and it joins the
//! network of each tenant that uses it, as `pg`. Its superuser password is made here, kept
//! in the agent's secret store (`<state>/secrets/postgres-root`, 0600) and never leaves the
//! node. Which user may connect from where is the server's `pg_hba.conf`, which this module
//! writes (a directory of its own, `<state>/pg-conf`, mounted read-only into the container)
//! from what it knows of every user: its tenant's network, and the remote addresses it was
//! given. Nobody else is let in, and the file is rewritten and reloaded with every change.
//!
//! What a user may do in a database is held in three roles that belong to that database,
//! `<db>__ro`, `<db>__rw` and `<db>__all`, which cannot log in: a user has an access level
//! in a database by being a member of one of them. They hold the privileges (on the
//! database, its schema, the tables and sequences that exist and, through default
//! privileges, those that are made later by anyone who may make them), so one place says
//! what each level means. The database itself belongs to the superuser, so nobody but the
//! node can drop it; the objects in it belong to whoever made them, or, for a dump that was
//! imported, to `<db>__all`, which every user with full access is a member of.

use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use builtpanel_docker_manager::{
    Capability, ContainerSpec, DockerError, Name, Network, PublishedPort, Restart, Tmpfs,
    VolumeMount,
};
use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{AgentError, DbSize, ErrorCode, Operation, Output};
use builtpanel_tenant_stack::{Allowlist, Role, network_name};
use serde::{Deserialize, Serialize};

use super::{
    db::{name, secret_file, secrets_dir},
    tenant::{TenantContext, docker_error, pull_with_fallback},
};

/// The server's container.
pub const CONTAINER: &str = "bp-pg";
/// Its network (the tenants' networks are added to it as it is used).
const NETWORK: &str = "bp-pg-net";
/// Its data.
const VOLUME: &str = "bp-pg-data";
/// The name tenants reach it by on their own network.
pub const ALIAS: &str = "pg";
/// The loopback port it is published on unless `BUILTPANEL_AGENT_PG_PORT` says another.
pub const DEFAULT_PORT: u16 = 5433;
/// The public port unless `BUILTPANEL_AGENT_PG_PUBLIC_PORT` says another.
pub const DEFAULT_PUBLIC_PORT: u16 = 15432;
/// The label that says the container publishes its public port (and which).
const PUBLIC_LABEL: &str = "bp-pg-public";
/// Where the configuration directory is mounted in the container.
const CONF_AT: &str = "/builtpanel-pg";
/// Longest wait for the server to accept connections.
const READY_TIMEOUT: Duration = Duration::from_secs(120);
/// Longest a statement may take.
const STATEMENT_TIMEOUT: Duration = Duration::from_secs(300);
/// The longest name of a database or user (the server's 63 bytes, less the longest suffix the node adds: `__all`, `__imp`).
pub const MAX_NAME_LEN: usize = 58;

const MIB: u64 = 1024 * 1024;

/// One of the three roles a database has: what it is called and what its members may do.
#[derive(Clone, Copy)]
struct Level {
    /// The suffix of the group role.
    suffix: &'static str,
    /// What members may do with the tables.
    tables: &'static str,
    /// What members may do with the sequences.
    sequences: &'static str,
    /// Whether members may make objects (tables, ...) in the schema.
    create: bool,
}

const LEVELS: [Level; 3] = [
    Level {
        suffix: "ro",
        tables: "SELECT",
        sequences: "SELECT",
        create: false,
    },
    Level {
        suffix: "rw",
        tables: "SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER",
        sequences: "USAGE, SELECT, UPDATE",
        create: true,
    },
    Level {
        suffix: "all",
        tables: "ALL",
        sequences: "ALL",
        create: true,
    },
];

/// The name of a database's group role for an access level (`read_only`, ...).
fn level_of(access: &str) -> Option<Level> {
    match access {
        "read_only" => Some(LEVELS[0]),
        "read_write" => Some(LEVELS[1]),
        "all" => Some(LEVELS[2]),
        _ => None,
    }
}

/// The loopback port of the server.
#[must_use]
pub fn port() -> u16 {
    std::env::var("BUILTPANEL_AGENT_PG_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p >= 1024)
        .unwrap_or(DEFAULT_PORT)
}

/// The port the server is published on for clients outside the node, when remote access is
/// on.
#[must_use]
pub fn public_port() -> u16 {
    std::env::var("BUILTPANEL_AGENT_PG_PUBLIC_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p >= 1024)
        .unwrap_or(DEFAULT_PUBLIC_PORT)
}

/// The server's container, for the server this task works on (the shared one, or a tenant's own).
#[must_use]
pub fn container_name() -> String {
    super::dbserver::container(super::dbserver::Kind::Postgres, CONTAINER)
}

/// The loopback port of the server this task works on.
///
/// # Errors
/// The disk failing, when a tenant's own server has no port yet.
pub fn server_port(context: &TenantContext) -> Result<u16, AgentError> {
    match super::dbserver::current() {
        Some(tenant) => {
            super::dbserver::dedicated_port(context, super::dbserver::Kind::Postgres, &tenant)
        }
        None => Ok(port()),
    }
}

fn invalid(message: &str) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, message.to_owned())
}

fn disk(what: &str, err: &std::io::Error) -> AgentError {
    AgentError::new(
        ErrorCode::Internal,
        format!("cannot {what}: {}", err.kind()),
    )
    .retryable()
}

/// What went wrong with a statement, said without quoting anything the server sent back (it
/// may echo values). The SQLSTATE code is only letters and digits.
pub(super) fn sql_error(err: &tokio_postgres::Error) -> AgentError {
    if let Some(db) = err.as_db_error() {
        let code = db.code().code();
        // "The system is starting up" (an engine that opens the port before the server is
        // ready answers like this): it will be fine in a moment.
        if code == "57P03" {
            return AgentError::new(ErrorCode::Internal, "the database server is starting up")
                .retryable();
        }
        let message = if code == "28P01" || code == "28000" {
            "the database server refused the agent's password".to_owned()
        } else {
            format!("the database server refused the statement (SQLSTATE {code})")
        };
        return AgentError::new(ErrorCode::Internal, message);
    }
    AgentError::new(ErrorCode::Internal, "the database server is not reachable").retryable()
}

/// A database, role or user name as a statement writes it: a plain identifier (checked),
/// not one the server keeps for itself, and quoted.
pub(super) fn identifier(text: &str) -> Result<String, AgentError> {
    builtpanel_domain::databases::check_identifier(text).map_err(|e| invalid(&e.to_string()))?;
    if text.len() > MAX_NAME_LEN {
        return Err(invalid("a PostgreSQL name is at most 58 characters"));
    }
    if text.starts_with("pg_") || matches!(text, "postgres" | "template0" | "template1") {
        return Err(invalid("this name is kept by the PostgreSQL server"));
    }
    // Plain letters, digits and `_` (checked above): nothing in it needs escaping.
    Ok(format!("\"{text}\""))
}

/// A role name that came out of the server's own catalog, held to the same rule as one from
/// a request before it is put in a statement.
fn quoted_role(text: &str) -> Result<String, AgentError> {
    let plain = !text.is_empty()
        && text.len() <= 63
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if plain {
        Ok(format!("\"{text}\""))
    } else {
        Err(invalid("a role name is not usable"))
    }
}

/// The quoted name of one of a database's group roles.
fn group(database: &str, level: Level) -> Result<String, AgentError> {
    identifier(database)?;
    Ok(format!("\"{database}__{}\"", level.suffix))
}

// ------------------------------------------------------------------ who may connect

/// What the server's `pg_hba.conf` is made from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Hosts {
    /// Per user: the addresses it may connect from.
    users: BTreeMap<String, UserHosts>,
}

/// One user's addresses.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct UserHosts {
    /// Its tenant's network, `172.20.0.0/16`.
    own: Option<String>,
    /// The remote addresses it was given.
    remote: BTreeSet<String>,
}

/// One change of the file at a time.
static HBA: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn conf_dir(context: &TenantContext) -> PathBuf {
    match super::dbserver::current() {
        Some(tenant) => context
            .vhosts_root
            .with_file_name(format!("pg-conf-{tenant}")),
        None => context.vhosts_root.with_file_name("pg-conf"),
    }
}

/// The text of `pg_hba.conf`: the superuser (from anywhere: the port is on loopback and
/// the password is long and random), each user from its own addresses, nobody else.
fn hba_text(hosts: &Hosts) -> String {
    let mut text = String::from(
        "# Written by the Built Panel node agent. Changes are overwritten.\n\
         local all all reject\n\
         host all postgres 0.0.0.0/0 scram-sha-256\n",
    );
    for (user, h) in &hosts.users {
        for cidr in h.own.iter().chain(h.remote.iter()) {
            text.push_str(&format!("host all \"{user}\" {cidr} scram-sha-256\n"));
        }
    }
    text.push_str("host all all 0.0.0.0/0 reject\nhost all all ::/0 reject\n");
    text
}

fn read_hosts(dir: &Path) -> Hosts {
    std::fs::read_to_string(dir.join("hosts.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Writes the state and `pg_hba.conf` next to each other, each by a rename.
fn write_hosts(dir: &Path, hosts: &Hosts) -> Result<(), AgentError> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(dir)
        .map_err(|e| disk("prepare the PostgreSQL configuration directory", &e))?;
    // The container's user reads it, whatever the umask.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| disk("prepare the PostgreSQL configuration directory", &e))?;
    let put = |file: &str, text: &str, mode: u32| -> std::io::Result<()> {
        let tmp = dir.join(format!(".{file}.tmp"));
        let _ = std::fs::remove_file(&tmp);
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        std::io::Write::write_all(&mut out, text.as_bytes())?;
        out.sync_all()?;
        std::fs::rename(&tmp, dir.join(file))
    };
    let json = serde_json::to_string(hosts)
        .map_err(|_| AgentError::new(ErrorCode::Internal, "cannot write the user list"))?;
    put("hosts.json", &json, 0o600).map_err(|e| disk("keep the user list", &e))?;
    put("pg_hba.conf", &hba_text(hosts), 0o644).map_err(|e| disk("write pg_hba.conf", &e))?;
    Ok(())
}

/// Changes what the file is made from and has the server read it again (when it runs).
async fn change_hosts(
    context: &TenantContext,
    change: impl FnOnce(&mut Hosts),
) -> Result<(), AgentError> {
    let _one = HBA.lock().await;
    let dir = conf_dir(context);
    let mut hosts = read_hosts(&dir);
    change(&mut hosts);
    write_hosts(&dir, &hosts)?;
    if context
        .docker
        .inspect(&name(&container_name())?)
        .await
        .is_ok_and(|i| i.running)
    {
        let conn = connect(context, "postgres").await?;
        conn.simple_query("SELECT pg_reload_conf()")
            .await
            .map_err(|e| sql_error(&e))?;
    }
    Ok(())
}

// ------------------------------------------------------------------ the server

/// The superuser's password, from the secret store.
fn root_password(context: &TenantContext) -> Result<String, AgentError> {
    secret_file(
        &secrets_dir(context),
        &super::dbserver::secret_name(super::dbserver::Kind::Postgres),
    )
}

/// A connection as the superuser to one database, from the agent's secret store.
pub(super) async fn connect(
    context: &TenantContext,
    database: &str,
) -> Result<tokio_postgres::Client, AgentError> {
    let password = root_password(context)?;
    connect_as(&password, "postgres", database, server_port(context)?).await
}

/// A connection as one user to one database on the loopback port.
pub(super) async fn connect_as(
    password: &str,
    user: &str,
    database: &str,
    port: u16,
) -> Result<tokio_postgres::Client, AgentError> {
    let mut config = tokio_postgres::Config::new();
    config
        .host("127.0.0.1")
        .port(port)
        .user(user)
        .password(password)
        .dbname(database)
        .connect_timeout(Duration::from_secs(10))
        .application_name("builtpanel-agent");
    let (client, connection) = config
        .connect(tokio_postgres::NoTls)
        .await
        .map_err(|e| sql_error(&e))?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

async fn wait_ready(context: &TenantContext) -> Result<(), AgentError> {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        match connect(context, "postgres").await {
            Ok(client) => {
                if client.simple_query("SELECT 1").await.is_ok() {
                    return Ok(());
                }
            }
            Err(e) if !e.retryable => return Err(e),
            Err(_) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(AgentError::new(
                ErrorCode::Internal,
                "the PostgreSQL server did not start in time",
            )
            .retryable());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Makes sure the server runs.
///
/// # Errors
/// Docker, the disk, or the server not starting (retryable).
pub async fn ensure_server(context: &TenantContext) -> Result<bool, AgentError> {
    ensure_server_with(context, false).await
}

/// [`ensure_server`], and with `public` the server also publishes its public port: a
/// container that does not is made again (its data is in the volume).
///
/// # Errors
/// As [`ensure_server`].
pub async fn ensure_server_with(context: &TenantContext, public: bool) -> Result<bool, AgentError> {
    let password = root_password(context)?;
    let docker = &context.docker;
    let container = name(&container_name())?;
    let dedicated = super::dbserver::current();
    let server_port = server_port(context)?;
    if dedicated.is_some() && public {
        return Err(invalid("a tenant's own database server has no public port"));
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
            // The file the server reads must be there before it starts.
            {
                let _one = HBA.lock().await;
                let dir = conf_dir(context);
                write_hosts(&dir, &read_hosts(&dir))?;
            }
            let allowlist = Allowlist::embedded().map_err(|e| {
                AgentError::new(
                    ErrorCode::Internal,
                    format!("the image list is broken: {e}"),
                )
            })?;
            let image = allowlist
                .current(Role::Pg)
                .map_err(|e| AgentError::new(ErrorCode::Internal, e.to_string()))?
                .clone();
            let network = match &dedicated {
                Some(tenant) => name(&format!("bp-{tenant}-pgnet"))?,
                None => name(NETWORK)?,
            };
            docker
                .ensure_network(&network, false)
                .await
                .map_err(|e| docker_error(&e))?;
            let volume = name(&super::dbserver::volume(
                super::dbserver::Kind::Postgres,
                VOLUME,
            ))?;
            docker
                .create_volume(&volume)
                .await
                .map_err(|e| docker_error(&e))?;
            let conf_volume = name(&match &dedicated {
                Some(tenant) => format!("bp-{tenant}-pg-conf"),
                None => "bp-pg-conf".to_owned(),
            })?;
            let dir = conf_dir(context);
            let root = dir.parent().unwrap_or(&dir);
            docker
                .create_bind_volume(&conf_volume, &dir, root)
                .await
                .map_err(|e| docker_error(&e))?;
            let image = pull_with_fallback(docker, &allowlist, &image).await?;
            let mut spec = ContainerSpec::new(container.clone(), image);
            spec.command = vec![
                "postgres".into(),
                "-c".into(),
                format!("hba_file={CONF_AT}/pg_hba.conf"),
                "-c".into(),
                "listen_addresses=*".into(),
                "-c".into(),
                "max_connections=300".into(),
                "-c".into(),
                "password_encryption=scram-sha-256".into(),
                "-c".into(),
                "shared_buffers=128MB".into(),
            ];
            spec.env
                .insert("POSTGRES_PASSWORD".into(), password.clone());
            spec.env.insert(
                "POSTGRES_INITDB_ARGS".into(),
                "--auth-host=scram-sha-256".into(),
            );
            // The image starts as root and gives its own files to `postgres`: it needs
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
                    target: "/run/postgresql".into(),
                    size_bytes: 16 * MIB,
                },
            ];
            spec.volumes = vec![
                VolumeMount {
                    volume,
                    target: "/var/lib/postgresql".into(),
                    read_only: false,
                },
                VolumeMount {
                    volume: conf_volume,
                    target: CONF_AT.into(),
                    read_only: true,
                },
            ];
            spec.network = Network::Named(network);
            let bad = || AgentError::new(ErrorCode::Internal, "bad address");
            spec.ports = vec![PublishedPort {
                container_port: 5432,
                host_port: server_port,
                host_ip: "127.0.0.1".parse().map_err(|_| bad())?,
            }];
            if public {
                // Only ever after the packet filter has been set: the port is open to
                // nobody until it is.
                spec.ports.push(PublishedPort {
                    container_port: 5432,
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
        let network =
            network_name(&TenantUsername::parse(tenant).map_err(|e| invalid(&e.to_string()))?)
                .map_err(|e| invalid(&e.to_string()))?;
        let _ = docker.connect_network(&container, &network, ALIAS).await;
    }
    wait_ready(context).await?;
    if created {
        // The server's own databases are nobody else's business.
        let conn = connect(context, "postgres").await?;
        for statement in [
            "REVOKE CONNECT ON DATABASE postgres FROM PUBLIC",
            "REVOKE CONNECT ON DATABASE template1 FROM PUBLIC",
        ] {
            conn.simple_query(statement)
                .await
                .map_err(|e| sql_error(&e))?;
        }
    }
    Ok(created)
}

/// `ensure_pg_server`.
///
/// # Errors
/// As [`ensure_server`].
pub async fn ensure_pg_server(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::EnsurePgServer = op else {
        return Err(super::wrong_operation());
    };
    let created = ensure_server(&context).await?;
    Ok(Output::DbServerReady {
        port: port(),
        created,
    })
}

/// Puts the server on a tenant's network as `pg`.
async fn connect_tenant(context: &TenantContext, network: &Name) -> Result<(), AgentError> {
    match context
        .docker
        .connect_network(&name(&container_name())?, network, ALIAS)
        .await
    {
        Ok(()) => Ok(()),
        Err(DockerError::NotFound) => Err(invalid("the tenant has no network on this node")),
        Err(e) => Err(docker_error(&e)),
    }
}

/// `allow_tenant_databases` for PostgreSQL.
///
/// # Errors
/// As [`ensure_server`]; a tenant that has no network here (not retryable).
pub async fn allow_tenant(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::AllowTenantDatabases { username, .. } = op else {
        return Err(super::wrong_operation());
    };
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    let network = network_name(&username).map_err(|e| invalid(&e.to_string()))?;
    ensure_server(&context).await?;
    connect_tenant(&context, &network).await?;
    Ok(Output::TenantDbAllowed {
        network: network.as_str().to_owned(),
    })
}

/// A tenant's own network as the address range its users connect from.
async fn tenant_range(
    context: &TenantContext,
    tenant: &TenantUsername,
) -> Result<String, AgentError> {
    let network = network_name(tenant).map_err(|e| invalid(&e.to_string()))?;
    let subnet = context
        .docker
        .network_subnet(&network)
        .await
        .map_err(|e| match e {
            DockerError::NotFound => invalid("the tenant has no network on this node"),
            other => docker_error(&other),
        })?;
    // Checked as an IPv4 range of a usable size; the text is what goes in the file.
    builtpanel_domain::databases::host_for_network(&subnet)
        .map(|_| subnet)
        .ok_or_else(|| {
            AgentError::new(
                ErrorCode::Internal,
                "the tenant's network range is not usable",
            )
        })
}

// ------------------------------------------------------------------ statements

async fn run(client: &tokio_postgres::Client, statement: &str) -> Result<(), AgentError> {
    tokio::time::timeout(STATEMENT_TIMEOUT, client.simple_query(statement))
        .await
        .map_err(|_| {
            AgentError::new(ErrorCode::Internal, "the database server took too long").retryable()
        })?
        .map_err(|e| sql_error(&e))?;
    Ok(())
}

async fn exists(
    client: &tokio_postgres::Client,
    query: &'static str,
    name: &str,
) -> Result<bool, AgentError> {
    Ok(client
        .query_opt(query, &[&name])
        .await
        .map_err(|e| sql_error(&e))?
        .is_some())
}

async fn database_exists(client: &tokio_postgres::Client, db: &str) -> Result<bool, AgentError> {
    exists(client, "SELECT 1 FROM pg_database WHERE datname = $1", db).await
}

async fn role_exists(client: &tokio_postgres::Client, role: &str) -> Result<bool, AgentError> {
    exists(client, "SELECT 1 FROM pg_roles WHERE rolname = $1", role).await
}

/// Gives each group role of a database what its level stands for, in the database itself:
/// on what is there now and, through default privileges, on what is made later by anyone
/// who may make objects (a member of `rw` or `all`, or the `all` role itself, which owns
/// what a dump creates). Safe to repeat.
pub(super) async fn refresh_privileges(
    context: &TenantContext,
    database: &str,
) -> Result<(), AgentError> {
    let admin = connect(context, "postgres").await?;
    let creators: Vec<String> = {
        let rows = admin
            .query(
                "SELECT m.rolname FROM pg_auth_members a
                 JOIN pg_roles m ON m.oid = a.member
                 JOIN pg_roles g ON g.oid = a.roleid
                 WHERE g.rolname = ANY($1)",
                &[&vec![format!("{database}__rw"), format!("{database}__all")]],
            )
            .await
            .map_err(|e| sql_error(&e))?;
        let mut names: Vec<String> = rows.iter().map(|r| r.get::<_, String>(0)).collect();
        names.push(format!("{database}__all"));
        names.sort();
        names.dedup();
        names
    };
    let conn = connect(context, database).await?;
    for level in LEVELS {
        let role = group(database, level)?;
        run(
            &conn,
            &format!(
                "GRANT {} ON SCHEMA public TO {role}",
                if level.create {
                    "USAGE, CREATE"
                } else {
                    "USAGE"
                }
            ),
        )
        .await?;
        run(
            &conn,
            &format!(
                "GRANT {} ON ALL TABLES IN SCHEMA public TO {role}",
                level.tables
            ),
        )
        .await?;
        run(
            &conn,
            &format!(
                "GRANT {} ON ALL SEQUENCES IN SCHEMA public TO {role}",
                level.sequences
            ),
        )
        .await?;
        for creator in &creators {
            let who = quoted_role(creator)?;
            run(
                &conn,
                &format!(
                    "ALTER DEFAULT PRIVILEGES FOR ROLE {who} IN SCHEMA public GRANT {} ON TABLES TO {role}",
                    level.tables
                ),
            )
            .await?;
            run(
                &conn,
                &format!(
                    "ALTER DEFAULT PRIVILEGES FOR ROLE {who} IN SCHEMA public GRANT {} ON SEQUENCES TO {role}",
                    level.sequences
                ),
            )
            .await?;
        }
    }
    Ok(())
}

/// Makes a temporary login role for one import or export: a member of one of a database's
/// group roles and nothing else, let in from `cidr`. [`finish_temporary_role`] removes it.
pub(super) async fn create_temporary_role(
    context: &TenantContext,
    admin: &tokio_postgres::Client,
    account: &str,
    password: &str,
    member_of: &str,
    cidr: &str,
) -> Result<(), AgentError> {
    let who = quoted_role(account)?;
    let group = quoted_role(member_of)?;
    let verifier = verifier(password)?;
    if role_exists(admin, account).await? {
        // A leftover of an import that was cut short: its password is made new.
        run(
            admin,
            &format!("ALTER ROLE {who} LOGIN PASSWORD '{verifier}' CONNECTION LIMIT 3"),
        )
        .await?;
        if !member_of_role(admin, account, member_of).await? {
            run(admin, &format!("GRANT {group} TO {who}")).await?;
        }
    } else {
        run(
            admin,
            &format!(
                "CREATE ROLE {who} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 3 PASSWORD '{verifier}' IN ROLE {group}"
            ),
        )
        .await?;
    }
    let own = cidr.to_owned();
    change_hosts(context, |h| {
        h.users.entry(account.to_owned()).or_default().own = Some(own);
    })
    .await
}

async fn member_of_role(
    admin: &tokio_postgres::Client,
    member: &str,
    group: &str,
) -> Result<bool, AgentError> {
    Ok(admin
        .query_opt(
            "SELECT 1 FROM pg_auth_members a
             JOIN pg_roles m ON m.oid = a.member JOIN pg_roles g ON g.oid = a.roleid
             WHERE m.rolname = $1 AND g.rolname = $2",
            &[&member, &group],
        )
        .await
        .map_err(|e| sql_error(&e))?
        .is_some())
}

/// Removes a role made by [`create_temporary_role`]: its sessions end, what it made in
/// `database` goes to that database's `__all` role (when `keep_objects`), and it is dropped.
pub(super) async fn finish_temporary_role(
    context: &TenantContext,
    database: &str,
    account: &str,
    keep_objects: bool,
) -> Result<(), AgentError> {
    let who = quoted_role(account)?;
    let admin = connect(context, "postgres").await?;
    if role_exists(&admin, account).await? {
        admin
            .execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = $1",
                &[&account],
            )
            .await
            .map_err(|e| sql_error(&e))?;
        if database_exists(&admin, database).await? {
            let inside = connect(context, database).await?;
            if keep_objects {
                let heir = group(database, LEVELS[2])?;
                run(&inside, &format!("REASSIGN OWNED BY {who} TO {heir}")).await?;
            }
            run(&inside, &format!("DROP OWNED BY {who}")).await?;
        }
        run(&admin, &format!("DROP ROLE {who}")).await?;
    }
    change_hosts(context, |h| {
        h.users.remove(account);
    })
    .await
}

/// `create_database` for PostgreSQL.
///
/// # Errors
/// [`AgentError`]: a name that is not a plain identifier (not retryable, nothing is sent to
/// the server); the server not reachable (retryable).
pub async fn create_database(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::CreateDatabase { name: database, .. } = op else {
        return Err(super::wrong_operation());
    };
    let quoted = identifier(&database)?;
    ensure_server(&context).await?;
    let conn = connect(&context, "postgres").await?;
    let existed = database_exists(&conn, &database).await?;
    if !existed {
        for level in LEVELS {
            let role = group(&database, level)?;
            if !role_exists(&conn, &format!("{database}__{}", level.suffix)).await? {
                run(&conn, &format!("CREATE ROLE {role} NOLOGIN")).await?;
            }
        }
        run(
            &conn,
            &format!("CREATE DATABASE {quoted} ENCODING 'UTF8' TEMPLATE template0"),
        )
        .await?;
        run(
            &conn,
            &format!("REVOKE ALL ON DATABASE {quoted} FROM PUBLIC"),
        )
        .await?;
        for level in LEVELS {
            let role = group(&database, level)?;
            let on_database = if level.suffix == "all" {
                "ALL"
            } else {
                "CONNECT, TEMPORARY"
            };
            run(
                &conn,
                &format!("GRANT {on_database} ON DATABASE {quoted} TO {role}"),
            )
            .await?;
        }
        // Nobody but the groups has the schema.
        let inside = connect(&context, &database).await?;
        run(&inside, "REVOKE ALL ON SCHEMA public FROM PUBLIC").await?;
        refresh_privileges(&context, &database).await?;
    }
    Ok(Output::DatabaseCreated { created: !existed })
}

/// `drop_database` for PostgreSQL.
///
/// # Errors
/// As [`create_database`].
pub async fn drop_database(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::DropDatabase { name: database, .. } = op else {
        return Err(super::wrong_operation());
    };
    let quoted = identifier(&database)?;
    let conn = connect(&context, "postgres").await?;
    let existed = database_exists(&conn, &database).await?;
    if existed {
        // Whoever is connected is sent away first, or the drop is refused.
        conn.execute(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
            &[&database],
        )
        .await
        .map_err(|e| sql_error(&e))?;
        run(&conn, &format!("DROP DATABASE {quoted}")).await?;
    }
    // The group roles, and the account an import made (T07.07), if there is one.
    for level in LEVELS {
        let role = format!("{database}__{}", level.suffix);
        if role_exists(&conn, &role).await? {
            run(&conn, &format!("DROP ROLE {}", group(&database, level)?)).await?;
        }
    }
    let import = format!("{database}__imp");
    if role_exists(&conn, &import).await? {
        run(&conn, &format!("DROP ROLE \"{import}\"")).await?;
        change_hosts(&context, |h| {
            h.users.remove(&import);
        })
        .await?;
    }
    Ok(Output::DatabaseDropped { existed })
}

/// `upsert_db_user` for PostgreSQL.
///
/// # Errors
/// [`AgentError`]: a bad name or password, or a new user without one (not retryable); the
/// tenant's network not found; the server not reachable (retryable).
pub async fn upsert_db_user(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::UpsertDbUser {
        username,
        name: user,
        password,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let quoted = identifier(&user)?;
    let tenant = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if let Some(p) = &password {
        builtpanel_domain::databases::check_password(&p.0).map_err(|e| invalid(&e.to_string()))?;
    }
    let range = tenant_range(&context, &tenant).await?;
    let network = network_name(&tenant).map_err(|e| invalid(&e.to_string()))?;
    ensure_server(&context).await?;
    connect_tenant(&context, &network).await?;
    let conn = connect(&context, "postgres").await?;
    let existing = role_exists(&conn, &user).await?;
    match (&password, existing) {
        (None, false) => return Err(invalid("a password is needed for a user that is new here")),
        (Some(p), _) => {
            let verifier = verifier(&p.0)?;
            let verb = if existing { "ALTER" } else { "CREATE" };
            run(
                &conn,
                &format!(
                    "{verb} ROLE {quoted} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 50 PASSWORD '{verifier}'"
                ),
            )
            .await?;
        }
        (None, true) => {}
    }
    let own = range.clone();
    change_hosts(&context, |h| {
        h.users.entry(user.clone()).or_default().own = Some(own);
    })
    .await?;
    Ok(Output::DbUserSet {
        created: !existing,
        host: range,
    })
}

/// A password as the server keeps it (SCRAM-SHA-256), so that the password itself is never
/// in a statement. The text is checked to be what such a verifier looks like.
fn verifier_is_plain(text: &str) -> bool {
    text.starts_with("SCRAM-SHA-256$")
        && text.len() < 300
        && text.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'$' | b':' | b'=' | b'+' | b'/' | b'-')
        })
}

fn verifier(password: &str) -> Result<String, AgentError> {
    let text = postgres_protocol::password::scram_sha_256(password.as_bytes());
    let plain = text.starts_with("SCRAM-SHA-256$")
        && text.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'$' | b':' | b'=' | b'+' | b'/' | b'-')
        });
    if plain {
        Ok(text)
    } else {
        Err(AgentError::new(
            ErrorCode::Internal,
            "the password verifier is not usable",
        ))
    }
}

/// The SCRAM verifier of a user's password as the server holds it.
///
/// # Errors
/// The server failing.
pub(super) async fn user_verifier(
    context: &TenantContext,
    name: &str,
) -> Result<Option<String>, AgentError> {
    identifier(name)?;
    let conn = connect(context, "postgres").await?;
    let row = conn
        .query_opt(
            "SELECT rolpassword FROM pg_authid WHERE rolname = $1",
            &[&name],
        )
        .await
        .map_err(|e| sql_error(&e))?;
    Ok(row
        .and_then(|r| r.get::<_, Option<String>>(0))
        .filter(|v| verifier_is_plain(v)))
}

/// Makes a user with a verifier that came from another server (or gives an existing user that
/// verifier), allowed in from its tenant's network.
///
/// # Errors
/// The server failing, or the tenant's network not found.
pub(super) async fn set_user_verifier(
    context: &TenantContext,
    tenant: &TenantUsername,
    name: &str,
    verifier: &str,
) -> Result<(), AgentError> {
    let quoted = identifier(name)?;
    if !verifier_is_plain(verifier) {
        return Err(AgentError::new(
            ErrorCode::Internal,
            "the password verifier is not usable",
        ));
    }
    let range = tenant_range(context, tenant).await?;
    let network = network_name(tenant).map_err(|e| invalid(&e.to_string()))?;
    ensure_server(context).await?;
    connect_tenant(context, &network).await?;
    let conn = connect(context, "postgres").await?;
    let verb = if role_exists(&conn, name).await? {
        "ALTER"
    } else {
        "CREATE"
    };
    run(
        &conn,
        &format!(
            "{verb} ROLE {quoted} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 50 PASSWORD '{verifier}'"
        ),
    )
    .await?;
    change_hosts(context, |h| {
        h.users.entry(name.to_owned()).or_default().own = Some(range);
    })
    .await
}

/// Removes a tenant's own server's files and ports from the node's state (the containers are
/// the caller's).
pub(super) fn forget_state(context: &TenantContext) {
    let _ = std::fs::remove_dir_all(conf_dir(context));
}

/// Takes away every access level a user has in a database.
async fn leave_all(
    conn: &tokio_postgres::Client,
    database: &str,
    user: &str,
) -> Result<(), AgentError> {
    for level in LEVELS {
        let role = group(database, level)?;
        if role_exists(conn, &format!("{database}__{}", level.suffix)).await? {
            run(conn, &format!("REVOKE {role} FROM {user}")).await?;
        }
    }
    Ok(())
}

/// `grant_db` for PostgreSQL.
///
/// # Errors
/// [`AgentError`]: a bad name or access level (not retryable); the user or the database
/// does not exist; the server not reachable (retryable).
pub async fn grant_db(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::GrantDb {
        database,
        user,
        access,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let level = level_of(&access).ok_or_else(|| invalid("an unknown access level"))?;
    identifier(&database)?;
    let who = identifier(&user)?;
    let conn = connect(&context, "postgres").await?;
    if !role_exists(&conn, &user).await? {
        return Err(invalid("the database user does not exist here"));
    }
    if !database_exists(&conn, &database).await? {
        return Err(invalid("the database does not exist here"));
    }
    leave_all(&conn, &database, &who).await?;
    run(
        &conn,
        &format!("GRANT {} TO {who}", group(&database, level)?),
    )
    .await?;
    refresh_privileges(&context, &database).await?;
    Ok(Output::DbGranted)
}

/// `revoke_db` for PostgreSQL.
///
/// # Errors
/// As [`grant_db`].
pub async fn revoke_db(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::RevokeDb { database, user, .. } = op else {
        return Err(super::wrong_operation());
    };
    identifier(&database)?;
    let who = identifier(&user)?;
    let conn = connect(&context, "postgres").await?;
    if role_exists(&conn, &user).await? && database_exists(&conn, &database).await? {
        leave_all(&conn, &database, &who).await?;
        refresh_privileges(&context, &database).await?;
    }
    Ok(Output::DbRevoked)
}

/// `drop_db_user` for PostgreSQL.
///
/// # Errors
/// As [`grant_db`].
pub async fn drop_db_user(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::DropDbUser { name: user, .. } = op else {
        return Err(super::wrong_operation());
    };
    let who = identifier(&user)?;
    let conn = connect(&context, "postgres").await?;
    let existed = role_exists(&conn, &user).await?;
    if existed {
        // What it made in each database goes to that database's `all` role, and what it was
        // given is taken back, or the server will not drop it.
        let databases: Vec<String> = conn
            .query(
                "SELECT datname FROM pg_database WHERE NOT datistemplate AND datallowconn AND datname <> 'postgres'",
                &[],
            )
            .await
            .map_err(|e| sql_error(&e))?
            .iter()
            .map(|r| r.get::<_, String>(0))
            .collect();
        conn.execute(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = $1",
            &[&user],
        )
        .await
        .map_err(|e| sql_error(&e))?;
        for database in databases {
            if identifier(&database).is_err() {
                continue;
            }
            let heir = format!("\"{database}__all\"");
            let inside = connect(&context, &database).await?;
            if role_exists(&conn, &format!("{database}__all")).await? {
                run(&inside, &format!("REASSIGN OWNED BY {who} TO {heir}")).await?;
            }
            run(&inside, &format!("DROP OWNED BY {who}")).await?;
        }
        run(&conn, &format!("DROP ROLE {who}")).await?;
    }
    change_hosts(&context, |h| {
        h.users.remove(&user);
    })
    .await?;
    Ok(Output::DbUserDropped { existed })
}

/// `set_db_remote_hosts` for PostgreSQL.
///
/// # Errors
/// [`AgentError`]: a bad address or too many, a user that does not exist (not retryable);
/// the server not reachable (retryable).
pub async fn set_db_remote_hosts(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetDbRemoteHosts {
        username,
        name: user,
        cidrs,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
    identifier(&user)?;
    let tenant = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if cidrs.len() > builtpanel_node_protocol::MAX_DB_REMOTE_HOSTS {
        return Err(invalid("too many remote addresses"));
    }
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for cidr in &cidrs {
        if builtpanel_domain::databases::host_for_remote(cidr).is_none() {
            return Err(invalid(
                "a remote address is IPv4 a.b.c.d/n, no wider than /8",
            ));
        }
        wanted.insert(cidr.clone());
    }
    let own = tenant_range(&context, &tenant).await?;
    let conn = connect(&context, "postgres").await?;
    if !role_exists(&conn, &user).await? {
        return Err(invalid("the database user does not exist for this tenant"));
    }
    let count = wanted.len();
    change_hosts(&context, |h| {
        let entry = h.users.entry(user.clone()).or_default();
        entry.own = Some(own);
        entry.remote = wanted;
    })
    .await?;
    Ok(Output::DbRemoteHostsSet { hosts: count })
}

/// `set_db_remote_access` for PostgreSQL.
///
/// # Errors
/// [`AgentError`]: a bad address or too many (not retryable); the privileged helper or Docker
/// not available (retryable). The public port is opened only after the packet filter was set;
/// when the helper refuses, nothing is opened.
pub async fn set_db_remote_access(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetDbRemoteAccess { cidrs, .. } = op else {
        return Err(super::wrong_operation());
    };
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
            builtpanel_priv_helper::protocol::Action::SetPgFirewall { cidrs: sent },
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

/// `report_pg_sizes`.
///
/// # Errors
/// As [`create_database`].
pub async fn report_sizes(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::ReportPgSizes = op else {
        return Err(super::wrong_operation());
    };
    let conn = connect(&context, "postgres").await?;
    let rows = conn
        .query(
            "SELECT datname, pg_database_size(datname)::bigint FROM pg_database
             WHERE NOT datistemplate AND datname <> 'postgres' ORDER BY datname",
            &[],
        )
        .await
        .map_err(|e| sql_error(&e))?;
    let items = rows
        .iter()
        .take(builtpanel_node_protocol::MAX_DB_SIZES_PER_REPORT)
        .map(|r| DbSize {
            name: r.get::<_, String>(0),
            bytes: u64::try_from(r.get::<_, i64>(1)).unwrap_or(0),
        })
        .collect();
    Ok(Output::DbSizes { items })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn names_are_plain_and_not_the_servers_own() {
        assert_eq!(identifier("alice_shop").unwrap(), "\"alice_shop\"");
        for bad in [
            "a; DROP DATABASE x",
            "a\"b",
            "Alice",
            "",
            "postgres",
            "template0",
            "template1",
            "pg_catalog",
            &"a".repeat(59),
        ] {
            assert!(identifier(bad).is_err(), "{bad}");
        }
        assert!(identifier(&"a".repeat(58)).is_ok());
    }

    #[test]
    fn the_hba_file_lets_each_user_in_from_its_own_addresses_only() {
        let mut hosts = Hosts::default();
        hosts.users.insert(
            "alice_web".into(),
            UserHosts {
                own: Some("172.20.0.0/16".into()),
                remote: BTreeSet::from(["203.0.113.7/32".to_owned()]),
            },
        );
        let text = hba_text(&hosts);
        assert!(text.contains("host all \"alice_web\" 172.20.0.0/16 scram-sha-256\n"));
        assert!(text.contains("host all \"alice_web\" 203.0.113.7/32 scram-sha-256\n"));
        assert!(text.contains("local all all reject\n"));
        assert!(text.ends_with("host all all 0.0.0.0/0 reject\nhost all all ::/0 reject\n"));
        // Nothing lets "all" in before the last two lines.
        assert_eq!(text.matches("scram-sha-256").count(), 3);
    }

    #[test]
    fn a_password_becomes_a_scram_verifier() {
        let v = verifier("Correct-Horse-9-Battery").unwrap();
        assert!(v.starts_with("SCRAM-SHA-256$4096:"));
        assert!(!v.contains("Correct"));
        // Made again, it differs (a new salt), and neither is the password.
        assert_ne!(v, verifier("Correct-Horse-9-Battery").unwrap());
    }

    #[test]
    fn access_levels_are_the_three_the_panel_has() {
        assert!(level_of("read_only").is_some_and(|l| !l.create));
        assert!(level_of("read_write").is_some_and(|l| l.create));
        assert!(level_of("all").is_some_and(|l| l.suffix == "all"));
        assert!(level_of("root").is_none());
    }
}
