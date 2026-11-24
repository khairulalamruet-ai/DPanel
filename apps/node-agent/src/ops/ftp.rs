//! SFTP and FTPS accounts (T08.11, T08.12).
//!
//! The node runs one SFTPGo container (`bp-sftpgo`) for every tenant. Each account is a
//! virtual user of it whose home is a folder of the tenant's files and that works as the
//! tenant's own user id: SFTPGo changes to that id for everything it does with files,
//! so the files are as closed to the account as they are to the tenant, and SFTPGo
//! itself never lets a path leave the home. The agent drives it through its REST API on
//! the loopback address, with an administrator it made for itself (the password is in
//! the agent's secret folder and nowhere else).

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use builtpanel_docker_manager::{
    Capability, ContainerSpec, DockerError, Network, PublishedPort, Restart, Tmpfs, VolumeMount,
};
use builtpanel_node_protocol::{AgentError, ErrorCode, Operation, Output};
use builtpanel_tenant_stack::{Allowlist, Role};
use serde_json::{Value, json};

use super::{
    db,
    sqlimport::{failed, invalid, random_password},
    tenant::{TenantContext, docker_error},
};

/// The container's name.
pub const CONTAINER: &str = "bp-sftpgo";
const MIB: u64 = 1024 * 1024;

fn port_from(env: &str, default: u16) -> u16 {
    std::env::var(env)
        .ok()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p >= 1024)
        .unwrap_or(default)
}

/// The port SFTP is offered on (`BUILTPANEL_AGENT_SFTP_PORT`).
#[must_use]
pub fn sftp_port() -> u16 {
    port_from("BUILTPANEL_AGENT_SFTP_PORT", 2222)
}

/// The port FTP (with TLS) is offered on (`BUILTPANEL_AGENT_FTP_PORT`).
#[must_use]
pub fn ftp_port() -> u16 {
    port_from("BUILTPANEL_AGENT_FTP_PORT", 2121)
}

/// First port of the passive data range (`BUILTPANEL_AGENT_FTP_PASSIVE_START`, 20 ports).
#[must_use]
pub fn passive_start() -> u16 {
    port_from("BUILTPANEL_AGENT_FTP_PASSIVE_START", 21_100)
}

/// Where the REST API listens on the loopback address (`BUILTPANEL_AGENT_SFTPGO_API_PORT`).
#[must_use]
pub fn api_port() -> u16 {
    port_from("BUILTPANEL_AGENT_SFTPGO_API_PORT", 18_083)
}

const PASSIVE_PORTS: u16 = 20;

fn state_dir(context: &TenantContext) -> PathBuf {
    context.vhosts_root.with_file_name("ftp")
}

fn secret_file(context: &TenantContext) -> PathBuf {
    context
        .vhosts_root
        .with_file_name("secrets")
        .join("sftpgo-admin")
}

/// The administrator's password: made once, kept private.
fn admin_password(context: &TenantContext) -> Result<String, AgentError> {
    let file = secret_file(context);
    if let Ok(text) = std::fs::read_to_string(&file) {
        let text = text.trim().to_owned();
        if !text.is_empty() {
            return Ok(text);
        }
    }
    use std::os::unix::fs::OpenOptionsExt as _;
    let password =
        random_password().map_err(|e| failed(&format!("cannot make a password: {}", e.kind())))?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| failed(&format!("cannot keep it: {}", e.kind())))?;
    }
    let mut out = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&file)
        .map_err(|e| failed(&format!("cannot keep the password: {}", e.kind())))?;
    std::io::Write::write_all(&mut out, password.as_bytes())
        .map_err(|e| failed(&format!("cannot keep the password: {}", e.kind())))?;
    Ok(password)
}

/// A self-signed certificate for FTPS (made once; clients that care pin it or the
/// operator puts a real one in its place).
fn ensure_certificate(dir: &Path) -> Result<(), AgentError> {
    if dir.join("ftp.crt").exists() && dir.join("ftp.key").exists() {
        return Ok(());
    }
    let key = rcgen::KeyPair::generate().map_err(|_| failed("cannot make a key"))?;
    let mut params = rcgen::CertificateParams::new(vec!["builtpanel-ftp".to_owned()])
        .map_err(|_| failed("cannot make a certificate"))?;
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(3650);
    let cert = params
        .self_signed(&key)
        .map_err(|_| failed("cannot make a certificate"))?;
    use std::os::unix::fs::OpenOptionsExt as _;
    for (name, text, mode) in [
        ("ftp.crt", cert.pem(), 0o644),
        ("ftp.key", key.serialize_pem(), 0o644),
    ] {
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(mode)
            .open(dir.join(name))
            .map_err(|e| failed(&format!("cannot keep the certificate: {}", e.kind())))?;
        std::io::Write::write_all(&mut out, text.as_bytes())
            .map_err(|e| failed(&format!("cannot keep the certificate: {}", e.kind())))?;
    }
    Ok(())
}

fn host_port(p: u16) -> Result<PublishedPort, AgentError> {
    Ok(PublishedPort {
        container_port: p,
        host_port: p,
        host_ip: "0.0.0.0".parse().map_err(|_| failed("bad address"))?,
    })
}

/// Makes sure the server runs.
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
                .current(Role::Sftp)
                .map_err(|e| AgentError::new(ErrorCode::Internal, e.to_string()))?
                .clone();
            let base = state_dir(context);
            let (data, certs) = (base.join("data"), base.join("certs"));
            for d in [&data, &certs] {
                std::fs::create_dir_all(d).map_err(|e| {
                    failed(&format!("cannot make the folder: {}", e.kind())).retryable()
                })?;
            }
            ensure_certificate(&certs)?;
            let state = context
                .vhosts_root
                .parent()
                .ok_or_else(|| failed("the state folder is unknown"))?
                .to_path_buf();
            let tenants_parent = context
                .tenants_root
                .parent()
                .ok_or_else(|| failed("the tenants folder has no parent"))?
                .to_path_buf();
            let (v_data, v_certs, v_tenants) = (
                db::name("bp-sftpgo-data")?,
                db::name("bp-sftpgo-certs")?,
                db::name("bp-sftpgo-tenants")?,
            );
            docker
                .create_bind_volume(&v_data, &data, &state)
                .await
                .map_err(|e| docker_error(&e))?;
            docker
                .create_bind_volume(&v_certs, &certs, &state)
                .await
                .map_err(|e| docker_error(&e))?;
            docker
                .create_bind_volume(&v_tenants, &context.tenants_root, &tenants_parent)
                .await
                .map_err(|e| docker_error(&e))?;
            let network = db::name("bp-sftpgo-net")?;
            docker
                .ensure_network(&network, false)
                .await
                .map_err(|e| docker_error(&e))?;
            let image = super::tenant::pull_with_fallback(docker, &allowlist, &image).await?;
            let mut spec = ContainerSpec::new(container.clone(), image);
            // SFTPGo works as each account's own user id, which only root can do.
            spec.user = Some((0, 0));
            spec.cap_add = vec![
                Capability::Chown,
                Capability::DacOverride,
                Capability::Fowner,
                Capability::Setgid,
                Capability::Setuid,
            ];
            let env = &mut spec.env;
            env.insert(
                "SFTPGO_DATA_PROVIDER__CREATE_DEFAULT_ADMIN".into(),
                "true".into(),
            );
            env.insert("SFTPGO_DEFAULT_ADMIN_USERNAME".into(), "builtpanel".into());
            env.insert(
                "SFTPGO_DEFAULT_ADMIN_PASSWORD".into(),
                admin_password(context)?,
            );
            env.insert(
                "SFTPGO_SFTPD__BINDINGS__0__PORT".into(),
                sftp_port().to_string(),
            );
            env.insert(
                "SFTPGO_FTPD__BINDINGS__0__PORT".into(),
                ftp_port().to_string(),
            );
            env.insert("SFTPGO_FTPD__BINDINGS__0__TLS_MODE".into(), "0".into());
            env.insert(
                "SFTPGO_FTPD__PASSIVE_PORT_RANGE__START".into(),
                passive_start().to_string(),
            );
            env.insert(
                "SFTPGO_FTPD__PASSIVE_PORT_RANGE__END".into(),
                (passive_start() + PASSIVE_PORTS - 1).to_string(),
            );
            env.insert(
                "SFTPGO_FTPD__CERTIFICATE_FILE".into(),
                "/certs/ftp.crt".into(),
            );
            env.insert(
                "SFTPGO_FTPD__CERTIFICATE_KEY_FILE".into(),
                "/certs/ftp.key".into(),
            );
            if let Ok(ip) = std::env::var("BUILTPANEL_AGENT_FTP_PASSIVE_IP") {
                env.insert("SFTPGO_FTPD__BINDINGS__0__FORCE_PASSIVE_IP".into(), ip);
            }
            // The web interfaces are off; only the REST API is on, and only on loopback.
            env.insert("SFTPGO_HTTPD__BINDINGS__0__PORT".into(), "8080".into());
            env.insert(
                "SFTPGO_HTTPD__BINDINGS__0__ENABLE_WEB_ADMIN".into(),
                "false".into(),
            );
            env.insert(
                "SFTPGO_HTTPD__BINDINGS__0__ENABLE_WEB_CLIENT".into(),
                "false".into(),
            );
            env.insert("SFTPGO_WEBDAVD__BINDINGS__0__PORT".into(), "0".into());
            env.insert("SFTPGO_TELEMETRY__BIND_PORT".into(), "0".into());
            // Brute-force protection (T11.07): ten failed logins from one address ban that
            // address for an hour. This only counts correctly when the real client address
            // reaches the container, which the installer arranges for the agent's rootless
            // Docker (port driver slirp4netns); with another driver every client would look
            // like the bridge gateway and one bad actor would lock everybody out.
            for (key, value) in [
                ("ENABLED", "true"),
                ("DRIVER", "memory"),
                ("BAN_TIME", "60"),
                ("BAN_TIME_INCREMENT", "100"),
                ("THRESHOLD", "10"),
                ("OBSERVATION_TIME", "60"),
                ("SCORE_VALID", "1"),
                ("SCORE_INVALID", "1"),
                ("SCORE_NO_AUTH", "0"),
                ("SCORE_LIMIT_EXCEEDED", "1"),
            ] {
                env.insert(format!("SFTPGO_COMMON__DEFENDER__{key}"), value.into());
            }
            spec.memory_bytes = 512 * MIB;
            spec.cpu_millis = 1000;
            spec.pids_limit = 512;
            spec.writable_root = true;
            spec.tmpfs = vec![Tmpfs {
                target: "/tmp".into(),
                size_bytes: 64 * MIB,
            }];
            spec.volumes = vec![
                VolumeMount {
                    volume: v_data,
                    target: "/var/lib/sftpgo".into(),
                    read_only: false,
                },
                VolumeMount {
                    volume: v_certs,
                    target: "/certs".into(),
                    read_only: true,
                },
                VolumeMount {
                    volume: v_tenants,
                    target: "/srv/tenants".into(),
                    read_only: false,
                },
            ];
            spec.network = Network::Named(network);
            let mut ports = vec![host_port(sftp_port())?, host_port(ftp_port())?];
            for p in passive_start()..passive_start() + PASSIVE_PORTS {
                ports.push(host_port(p)?);
            }
            ports.push(PublishedPort {
                container_port: 8080,
                host_port: api_port(),
                host_ip: "127.0.0.1".parse().map_err(|_| failed("bad address"))?,
            });
            spec.ports = ports;
            spec.restart = Restart::UnlessStopped;
            match docker.create_container(&spec).await {
                Ok(_) | Err(DockerError::Conflict(_)) => {}
                Err(e) => return Err(docker_error(&e)),
            }
        }
        Err(e) => return Err(docker_error(&e)),
    }
    docker
        .start(&container)
        .await
        .map_err(|e| docker_error(&e))?;
    Ok(())
}

fn api(path: &str) -> String {
    format!("http://127.0.0.1:{}{path}", api_port())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .max_redirects(0)
        .build()
        .into()
}

/// A token for the REST API (waiting for the server to come up).
fn token(admin: &str) -> Result<String, AgentError> {
    let http = agent();
    let basic = format!(
        "Basic {}",
        base64_of(format!("builtpanel:{admin}").as_bytes(),)
    );
    let mut last = failed("the file transfer server did not come up").retryable();
    for _ in 0..60 {
        match http
            .get(&api("/api/v2/token"))
            .header("Authorization", &basic)
            .call()
        {
            Ok(mut r) if r.status().as_u16() == 200 => {
                let body: Value = r
                    .body_mut()
                    .read_json()
                    .map_err(|_| failed("the file transfer server's answer is unreadable"))?;
                return body["access_token"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| failed("the file transfer server gave no token"));
            }
            // Right after the start the administrator may not be made yet: try again.
            Ok(r) if r.status().as_u16() == 401 => {
                last = failed("the file transfer server refused the agent's password");
                std::thread::sleep(Duration::from_millis(500));
            }
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }
    Err(last)
}

fn base64_of(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(char::from(A[(n >> 18) as usize & 63]));
        out.push(char::from(A[(n >> 12) as usize & 63]));
        out.push(if c.len() > 1 {
            char::from(A[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            char::from(A[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

/// A name for an account: letters, digits, `_`, `-`, `.`; starts with a letter or digit.
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
}

/// A folder of the tenant's files, as text: no `..`, nothing odd.
fn valid_home(home: &str) -> Option<String> {
    if home.len() > 1024 || home.contains('\0') || home.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = home
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.len() > 32
        || parts
            .iter()
            .any(|p| *p == ".." || p.len() > 255 || p.chars().any(char::is_control))
        || parts.first() == Some(&".trash")
    {
        return None;
    }
    Some(parts.join("/"))
}

/// `upsert_ftp_account`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable); Docker or the server failing (retryable).
pub async fn upsert_ftp_account(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::UpsertFtpAccount {
        tenant_id,
        uid,
        name,
        home,
        password,
        sftp,
        ftps,
        ftp_plain,
        read_only,
        quota_mb,
        enabled,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    if !valid_name(&name) {
        return Err(invalid("the account name is not acceptable"));
    }
    let home = valid_home(&home).ok_or_else(|| invalid("the home folder is not acceptable"))?;
    if !(10_000..60_000).contains(&uid) {
        return Err(invalid("the user id is outside the tenant range"));
    }
    if let Some(p) = &password
        && (p.0.len() < 8 || p.0.len() > 128 || p.0.chars().any(char::is_control))
    {
        return Err(invalid("the password is not acceptable"));
    }
    ensure_container(&context).await?;
    let admin = admin_password(&context)?;
    let home_dir = if home.is_empty() {
        format!("/srv/tenants/{tenant_id}")
    } else {
        format!("/srv/tenants/{tenant_id}/{home}")
    };
    // Which protocols are off. FTP with TLS is `ftp_security` 1; plain FTP needs 0.
    let mut denied = vec!["DAV", "HTTP"];
    if !sftp {
        denied.push("SSH");
    }
    if !(ftps || ftp_plain) {
        denied.push("FTP");
    }
    let permissions: Value = if read_only {
        json!({"/": ["list", "download"]})
    } else {
        json!({"/": ["*"]})
    };
    let mut user = json!({
        "status": i32::from(enabled),
        "username": name,
        "home_dir": home_dir,
        "uid": uid,
        "gid": uid,
        "max_sessions": 10,
        "quota_size": quota_mb.map_or(0, |m| m.saturating_mul(1024 * 1024)),
        "permissions": permissions,
        "filters": {
            "denied_protocols": denied,
            "ftp_security": i32::from(!ftp_plain),
        },
        "description": format!("builtpanel:{tenant_id}"),
    });
    let created = tokio::task::spawn_blocking(move || -> Result<bool, AgentError> {
        let http = agent();
        let bearer = format!("Bearer {}", token(&admin)?);
        let exists = http
            .get(&api(&format!("/api/v2/users/{name}")))
            .header("Authorization", &bearer)
            .call()
            .map_err(|_| failed("the file transfer server is not reachable").retryable())?;
        match exists.status().as_u16() {
            200 => {
                // Someone else's account is never taken over.
                let mut exists = exists;
                let current: Value = exists
                    .body_mut()
                    .read_json()
                    .map_err(|_| failed("the file transfer server's answer is unreadable"))?;
                if current["description"] != user["description"] {
                    return Err(invalid("this account name belongs to someone else"));
                }
                if let Some(p) = &password {
                    user["password"] = json!(p.0);
                }
                let r = http
                    .put(&api(&format!("/api/v2/users/{name}?disconnect=1")))
                    .header("Authorization", &bearer)
                    .send_json(&user)
                    .map_err(|_| failed("the file transfer server is not reachable").retryable())?;
                if r.status().as_u16() != 200 {
                    return Err(failed(&format!(
                        "the file transfer server refused the account ({})",
                        r.status().as_u16()
                    )));
                }
                Ok(false)
            }
            404 => {
                let Some(p) = &password else {
                    return Err(invalid("a new account needs a password"));
                };
                user["password"] = json!(p.0);
                let r = http
                    .post(&api("/api/v2/users"))
                    .header("Authorization", &bearer)
                    .send_json(&user)
                    .map_err(|_| failed("the file transfer server is not reachable").retryable())?;
                if r.status().as_u16() != 201 {
                    return Err(failed(&format!(
                        "the file transfer server refused the account ({})",
                        r.status().as_u16()
                    )));
                }
                Ok(true)
            }
            _ => Err(failed("the file transfer server failed").retryable()),
        }
    })
    .await
    .map_err(|_| failed("the work was cancelled"))??;
    Ok(Output::FtpAccountSet { created })
}

/// `drop_ftp_account`.
///
/// # Errors
/// [`AgentError`]: a bad name (not retryable); the server failing (retryable).
pub async fn drop_ftp_account(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::DropFtpAccount { name } = op else {
        return Err(super::wrong_operation());
    };
    if !valid_name(&name) {
        return Err(invalid("the account name is not acceptable"));
    }
    // No server, no account.
    if context.docker.inspect(&db::name(CONTAINER)?).await.is_err() {
        return Ok(Output::FtpAccountDropped { existed: false });
    }
    ensure_container(&context).await?;
    let admin = admin_password(&context)?;
    let existed = tokio::task::spawn_blocking(move || -> Result<bool, AgentError> {
        let http = agent();
        let bearer = format!("Bearer {}", token(&admin)?);
        let r = http
            .delete(&api(&format!("/api/v2/users/{name}")))
            .header("Authorization", &bearer)
            .call()
            .map_err(|_| failed("the file transfer server is not reachable").retryable())?;
        match r.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            _ => Err(failed("the file transfer server failed").retryable()),
        }
    })
    .await
    .map_err(|_| failed("the work was cancelled"))??;
    Ok(Output::FtpAccountDropped { existed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_homes_are_checked() {
        assert!(valid_name("alice_site-2.x"));
        for bad in ["", "Alice", "-x", "a b", "a/b", &"a".repeat(65)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert_eq!(valid_home("www//sub/./x").as_deref(), Some("www/sub/x"));
        assert_eq!(valid_home("").as_deref(), Some(""));
        for bad in ["../x", "/etc", "a/../b", ".trash/x", "a\0b"] {
            assert_eq!(valid_home(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn base64_matches_the_known_answers() {
        assert_eq!(base64_of(b"builtpanel:secret"), "YnVpbHRwYW5lbDpzZWNyZXQ=");
        assert_eq!(base64_of(b"ab"), "YWI=");
    }
}
