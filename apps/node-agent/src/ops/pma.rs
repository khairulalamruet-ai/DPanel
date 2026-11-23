//! phpMyAdmin for clients (T07.09), signed in without a password.
//!
//! One container, `bp-pma`, runs the node's phpMyAdmin in single-sign-on mode:
//! it has no sign-in form and no server list, it only takes what a sign-in page of
//! our own hands it. A client asks the control plane for a link; the control plane
//! makes a token, keeps it to itself and tells the node only its SHA-256. The node
//! makes a temporary database account that has rights on that one database,
//! writes its credentials into a file named by the token's hash and, when the
//! client opens the link, the page inside the container moves that file away (so
//! the link works once), reads it and signs the client in. The account is dropped
//! when the session's time is up.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use builtpanel_docker_manager::{
    Capability, ContainerSpec, DockerError, Network, PublishedPort, Restart, Tmpfs, VolumeMount,
};
use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{AgentError, ErrorCode, Operation, Output};
use builtpanel_tenant_stack::{Allowlist, Role};
use mysql_async::prelude::Queryable as _;

use super::{
    db,
    sqlimport::{failed, invalid, random_password, server_words},
    tenant::{TenantContext, docker_error},
};

/// The container's name.
pub const CONTAINER: &str = "bp-pma";
/// The network the web server joins to reach it.
pub const FRONT_NETWORK: &str = "bp-pma-net";
const MIB: u64 = 1024 * 1024;
/// How long a link works after it was made.
const LINK_SECS: i64 = 300;

/// Loopback port of the container (`BUILTPANEL_AGENT_PMA_PORT`).
#[must_use]
pub fn port() -> u16 {
    std::env::var("BUILTPANEL_AGENT_PMA_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p >= 1024)
        .unwrap_or(18_082)
}

fn base(context: &TenantContext) -> PathBuf {
    context.vhosts_root.with_file_name("pma")
}

fn sessions_dir(context: &TenantContext) -> PathBuf {
    base(context).join("sessions")
}

pub(super) fn accounts_dir(context: &TenantContext) -> PathBuf {
    base(context).join("accounts")
}

pub(super) fn disk(what: &str, err: &std::io::Error) -> AgentError {
    failed(&format!("{what}: {}", err.kind())).retryable()
}

/// The configuration of phpMyAdmin: single sign-on only, the sign-in done by the
/// one page below, no way to name another server.
const USER_CONFIG: &str = r#"<?php
$cfg['Servers'][1]['auth_type'] = 'signon';
$cfg['Servers'][1]['SignonSession'] = 'DHSIGNON';
$cfg['Servers'][1]['SignonURL'] = '/index.php?dhsignon=ended';
$cfg['Servers'][1]['LogoutURL'] = '/index.php?dhsignon=bye';
$cfg['Servers'][1]['host'] = 'bp-db';
$cfg['Servers'][1]['port'] = 3306;
$cfg['Servers'][1]['AllowNoPassword'] = false;
$cfg['AllowArbitraryServer'] = false;
$cfg['ShowServerInfo'] = false;
$cfg['VersionCheck'] = false;
$cfg['SendErrorReports'] = 'never';
$cfg['CheckConfigurationPermissions'] = false;
if (isset($_GET['dhsignon'])) {
    $say = static function (int $code, string $text): void {
        http_response_code($code);
        header('Content-Type: text/plain; charset=utf-8');
        header('Cache-Control: no-store');
        echo $text . "\n";
        exit;
    };
    $t = (string) $_GET['dhsignon'];
    if ($t === 'bye') {
        session_name('DHSIGNON');
        session_start();
        $_SESSION = [];
        session_destroy();
        $say(200, 'Signed out. You can close this tab.');
    }
    if ($t === 'ended') {
        $say(403, 'This session has ended. Open phpMyAdmin again from the panel.');
    }
    if (!preg_match('/^[0-9a-f]{64}$/', $t)) {
        $say(403, 'This link does not work.');
    }
    $file = '/dhsessions/' . hash('sha256', $t);
    $mine = $file . '.' . bin2hex(random_bytes(8));
    // Whoever moves the file away first has the only sign-in it will ever give.
    if (!@rename($file, $mine)) {
        $say(403, 'This link does not work, or it was used already. Open phpMyAdmin again from the panel.');
    }
    $raw = @file_get_contents($mine);
    @unlink($mine);
    $d = json_decode((string) $raw, true);
    if (!is_array($d) || (int) ($d['link_expires'] ?? 0) < time()) {
        $say(403, 'This link has run out. Open phpMyAdmin again from the panel.');
    }
    session_name('DHSIGNON');
    session_start();
    $_SESSION['PMA_single_signon_user'] = (string) $d['user'];
    $_SESSION['PMA_single_signon_password'] = (string) $d['password'];
    session_write_close();
    header('Cache-Control: no-store');
    header('Location: index.php?route=/database/structure&db=' . rawurlencode((string) $d['db']));
    exit;
}
"#;

/// Makes sure the container runs. Returns whether this call made it.
async fn ensure_container(context: &TenantContext) -> Result<(), AgentError> {
    let docker = &context.docker;
    let container = db::name(CONTAINER)?;
    match docker.inspect(&container).await {
        Ok(_) => {}
        Err(DockerError::NotFound) => {
            let allowlist = Allowlist::embedded().map_err(|e| {
                AgentError::new(
                    ErrorCode::Internal,
                    format!("the image list is broken: {e}"),
                )
            })?;
            let image = allowlist
                .current(Role::Pma)
                .map_err(|e| AgentError::new(ErrorCode::Internal, e.to_string()))?
                .clone();
            let sessions = sessions_dir(context);
            std::fs::create_dir_all(&sessions).map_err(|e| disk("cannot make the folder", &e))?;
            // The page in the container (www-data there) moves the files away, so it
            // needs the folder; nobody else on the machine can reach it (the state
            // folder is private to the agent).
            set_mode(&sessions, 0o777)?;
            std::fs::create_dir_all(accounts_dir(context))
                .map_err(|e| disk("cannot make the folder", &e))?;
            let state = context
                .vhosts_root
                .parent()
                .ok_or_else(|| failed("the state folder is unknown"))?
                .to_path_buf();
            let volume = db::name("bp-pma-sessions")?;
            docker
                .create_bind_volume(&volume, &sessions, &state)
                .await
                .map_err(|e| docker_error(&e))?;
            let db_net = db::name(db::NETWORK)?;
            docker
                .ensure_network(&db_net, false)
                .await
                .map_err(|e| docker_error(&e))?;
            let front = db::name(FRONT_NETWORK)?;
            docker
                .ensure_network(&front, false)
                .await
                .map_err(|e| docker_error(&e))?;
            let image = super::tenant::pull_with_fallback(docker, &allowlist, &image).await?;
            let mut spec = ContainerSpec::new(container.clone(), image);
            // The node's web server reaches phpMyAdmin on the machine's loopback address (a port
            // Docker picks; the agent reads it back), the same way it reaches a site.
            spec.ports = vec![builtpanel_docker_manager::PublishedPort {
                container_port: 80,
                host_port: 0,
                host_ip: std::net::IpAddr::from([127, 0, 0, 1]),
            }];
            spec.env.insert(
                "PMA_USER_CONFIG_BASE64".into(),
                base64(USER_CONFIG.as_bytes()),
            );
            spec.env.insert("HIDE_PHP_VERSION".into(), "1".into());
            // The image starts Apache as root, which then runs as www-data.
            spec.cap_add = vec![
                Capability::Chown,
                Capability::DacOverride,
                Capability::Fowner,
                Capability::Setgid,
                Capability::Setuid,
                Capability::NetBindService,
                Capability::Kill,
            ];
            spec.writable_root = true;
            spec.memory_bytes = 256 * MIB;
            spec.cpu_millis = 1000;
            spec.pids_limit = 256;
            spec.tmpfs = vec![Tmpfs {
                target: "/tmp".into(),
                size_bytes: 64 * MIB,
            }];
            spec.volumes = vec![VolumeMount {
                volume,
                target: "/dhsessions".into(),
                read_only: false,
            }];
            spec.network = Network::Named(db_net);
            spec.ports = vec![PublishedPort {
                container_port: 80,
                host_port: port(),
                host_ip: "127.0.0.1"
                    .parse()
                    .map_err(|_| failed("bad loopback address"))?,
            }];
            spec.restart = Restart::UnlessStopped;
            match docker.create_container(&spec).await {
                Ok(_) | Err(DockerError::Conflict(_)) => {}
                Err(e) => return Err(docker_error(&e)),
            }
            docker
                .connect_network(&container, &front, "bp-pma")
                .await
                .map_err(|e| docker_error(&e))?;
        }
        Err(e) => return Err(docker_error(&e)),
    }
    docker
        .start(&container)
        .await
        .map_err(|e| docker_error(&e))?;
    Ok(())
}

pub(super) fn set_mode(path: &Path, mode: u32) -> Result<(), AgentError> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| disk("cannot set the folder's rights", &e))
}

pub(super) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(char::from(ALPHABET[(n >> 18) as usize & 63]));
        out.push(char::from(ALPHABET[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

pub(super) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// `open_phpmyadmin`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable); Docker or the database server
/// failing (retryable).
pub async fn open_phpmyadmin(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::OpenPhpMyAdmin {
        username,
        database,
        session_id,
        token_hash,
        session_minutes,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    builtpanel_domain::databases::check_identifier(&database)
        .map_err(|e| invalid(&e.to_string()))?;
    if token_hash.len() != 64
        || !token_hash
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(invalid("the token's hash is not 64 lower-case hex digits"));
    }
    if !(5..=720).contains(&session_minutes) {
        return Err(invalid("a session lasts 5 to 720 minutes"));
    }
    // Old sessions first, so a node does not collect accounts.
    sweep(&context).await;
    db::ensure_server(&context).await?;
    ensure_container(&context).await?;

    let password =
        random_password().map_err(|e| failed(&format!("cannot make a password: {}", e.kind())))?;
    let account = format!("dhpma-{}", &session_id.simple().to_string()[..12]);
    let account_ends = now() + i64::from(session_minutes) * 60 + LINK_SECS;
    {
        let mut admin = db::root_connection(&context).await?;
        let hash: Option<String> = admin
            .exec_first("SELECT PASSWORD(?)", (&password,))
            .await
            .map_err(|e| failed(&server_words(&e)))?;
        let hash = hash
            .filter(|h| {
                h.len() == 41 && h.starts_with('*') && h[1..].bytes().all(|b| b.is_ascii_hexdigit())
            })
            .ok_or_else(|| failed("the database server gave no password hash"))?;
        admin
            .query_drop(format!("DROP USER IF EXISTS '{account}'@'%'"))
            .await
            .map_err(|e| failed(&server_words(&e)))?;
        admin
            .query_drop(format!(
                "CREATE USER '{account}'@'%' IDENTIFIED BY PASSWORD '{hash}' WITH MAX_USER_CONNECTIONS 6"
            ))
            .await
            .map_err(|e| failed(&server_words(&e)))?;
        let privileges = builtpanel_domain::databases::Access::All
            .privileges()
            .join(", ");
        admin
            .query_drop(format!(
                "GRANT {privileges} ON `{database}`.* TO '{account}'@'%'"
            ))
            .await
            .map_err(|e| failed(&server_words(&e)))?;
        let _ = admin.disconnect().await;
    }
    // The account is written down first: whatever happens next, the sweep finds it.
    let accounts = accounts_dir(&context);
    std::fs::create_dir_all(&accounts).map_err(|e| disk("cannot keep the session", &e))?;
    std::fs::write(accounts.join(&account), account_ends.to_string())
        .map_err(|e| disk("cannot keep the session", &e))?;
    let link_expires = now() + LINK_SECS;
    let body = serde_json::json!({
        "user": account,
        "password": password,
        "db": database,
        "link_expires": link_expires,
    })
    .to_string();
    let dir = sessions_dir(&context);
    let tmp = dir.join(format!(".{token_hash}.tmp"));
    write_private(&tmp, &body).map_err(|e| disk("cannot keep the session", &e))?;
    std::fs::rename(&tmp, dir.join(&token_hash))
        .map_err(|e| disk("cannot keep the session", &e))?;
    Ok(Output::PhpMyAdminOpened {
        link_expires_at: link_expires,
    })
}

/// A file that the container's page can read (and move away) but that is not
/// listed for anyone else on the machine: the folder is the agent's own.
pub(super) fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::{
        io::Write as _,
        os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    };
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o644)
        .open(path)?;
    // Not left to the umask (the agent's service sets 0077): the container's user reads this.
    file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// Drops the accounts whose time is up (and ends their connections), and removes
/// links nobody used.
pub async fn sweep(context: &TenantContext) {
    let accounts = accounts_dir(context);
    let mut due = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&accounts) {
        for item in entries.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let ends: i64 = std::fs::read_to_string(item.path())
                .ok()
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0);
            let plain = name.starts_with("dhpma-")
                && name[6..].bytes().all(|b| b.is_ascii_hexdigit())
                && name.len() == 18;
            if plain && ends < now() {
                due.push((name, item.path()));
            }
        }
    }
    if !due.is_empty()
        && let Ok(mut admin) = db::root_connection(context).await
    {
        for (name, path) in due {
            let _ = admin.query_drop(format!("KILL USER '{name}'@'%'")).await;
            if admin
                .query_drop(format!("DROP USER IF EXISTS '{name}'@'%'"))
                .await
                .is_ok()
            {
                let _ = std::fs::remove_file(path);
            }
        }
        let _ = admin.disconnect().await;
    }
    // A link that was never opened.
    if let Ok(entries) = std::fs::read_dir(sessions_dir(context)) {
        for item in entries.flatten() {
            let old = item
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(LINK_SECS.unsigned_abs() + 60));
            if old {
                let _ = std::fs::remove_file(item.path());
            }
        }
    }
}

/// Runs [`sweep`] every half minute, for as long as the agent runs.
pub async fn sweep_forever(context: Arc<TenantContext>) {
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        sweep(&context).await;
        super::transfer::sweep(&context);
        super::webmail::sweep(&context);
    }
}
