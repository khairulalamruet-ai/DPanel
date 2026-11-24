//! A tenant's web server configuration on the node (T06.04): one file per domain
//! in the tenant's configuration directory, which its web container reads as
//! nginx's `conf.d`.
//!
//! The control plane renders the files. The agent writes one, has the web server
//! check the whole configuration (`nginx -t` inside the container) and reloads
//! it. When the check refuses, the file as it was before is put back: the site
//! that was running keeps running.

use std::{
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    sync::Arc,
};

use builtpanel_docker_manager::{DockerError, NginxCommand};
use builtpanel_domain::{Fqdn, TenantUsername};
use builtpanel_node_protocol::{
    AgentError, ErrorCode, MAX_HOSTS_PER_TENANT, MAX_VHOST_CONFIG_BYTES, Operation, Output,
};
use builtpanel_tenant_stack::web_container_name;

use super::tenant::{TenantContext, docker_error};

/// One change at a time: the check and the reload look at the whole directory.
static CHANGING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn invalid(message: impl Into<String>) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, message.into())
}

fn disk(what: &str, err: &std::io::Error) -> AgentError {
    AgentError::new(
        ErrorCode::Internal,
        format!("cannot {what}: {}", err.kind()),
    )
    .retryable()
}

/// The directory a tenant's configuration files are kept in: created if it is
/// not there, readable by the web container.
///
/// # Errors
/// The directory cannot be made.
pub fn conf_dir(root: &Path, username: &TenantUsername) -> std::io::Result<PathBuf> {
    let dir = root.join(username.as_str());
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(&dir)?;
    // Whatever umask the process has: the container's user must be able to read.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))?;
    Ok(dir)
}

/// The directory a tenant's php.ini options are kept in (`<user>.php`, which no
/// tenant name can be): created if it is not there, readable by the PHP containers.
///
/// # Errors
/// The directory cannot be made.
pub fn php_ini_dir(root: &Path, username: &TenantUsername) -> std::io::Result<PathBuf> {
    let dir = root.join(format!("{}.php", username.as_str()));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))?;
    Ok(dir)
}

fn check_config(config: &str) -> Result<(), AgentError> {
    if config.trim().is_empty() {
        return Err(invalid("the configuration is empty"));
    }
    if config.len() > MAX_VHOST_CONFIG_BYTES {
        return Err(invalid("the configuration is too large"));
    }
    if config
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(invalid("the configuration holds control characters"));
    }
    // Whatever the control plane made, a file that reads other directories or
    // loads code is not written.
    builtpanel_domain::vhost::check_site_config(config).map_err(|e| invalid(e.message))
}

/// Puts `content` in `path` (or removes the file for `None`), all at once.
fn put(dir: &Path, path: &Path, content: Option<&[u8]>) -> std::io::Result<()> {
    match content {
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        Some(bytes) => {
            // Not named `*.conf`: the web server must never see it half-written.
            let tmp = dir.join(format!(
                ".{}.tmp",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("site")
            ));
            let _ = std::fs::remove_file(&tmp);
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(&tmp)?;
            // `mode` above is cut down by the process's umask (the agent's service sets 0077):
            // the web container's user must be able to read this, so it is set outright.
            out.set_permissions(std::fs::Permissions::from_mode(0o644))?;
            std::io::Write::write_all(&mut out, bytes)?;
            out.sync_all()?;
            std::fs::rename(tmp, path)
        }
    }
}

/// What nginx said, shortened and on one line.
fn reason(output: &str) -> String {
    let line = output
        .lines()
        .find(|l| l.contains("[emerg]") || l.contains("[alert]") || l.contains("[crit]"))
        .or_else(|| output.lines().next())
        .unwrap_or("no reason given");
    line.chars().filter(|c| !c.is_control()).take(300).collect()
}

/// One file's new content (`None` removes it).
struct Change {
    domain: String,
    content: Option<String>,
}

/// Makes the changes, has the web server check and use the result, and puts
/// every file back as it was when the web server refuses. Returns whether a
/// running web server has the result now.
async fn change(
    context: &TenantContext,
    username: &TenantUsername,
    changes: &[Change],
) -> Result<bool, AgentError> {
    let _one_at_a_time = CHANGING.lock().await;
    let dir =
        conf_dir(&context.vhosts_root, username).map_err(|e| disk("prepare the directory", &e))?;
    let mut before: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    for c in changes {
        let path = dir.join(format!("{}.conf", c.domain));
        let was = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(disk("read the current file", &e)),
        };
        before.push((path, was));
    }
    let restore = || {
        for (path, was) in &before {
            if let Err(e) = put(&dir, path, was.as_deref()) {
                eprintln!(
                    "builtpanel-agent: could not put back {}: {}",
                    path.display(),
                    e.kind()
                );
            }
        }
    };
    for (c, (path, _)) in changes.iter().zip(&before) {
        if let Err(e) = put(&dir, path, c.content.as_deref().map(str::as_bytes)) {
            restore();
            return Err(disk("write the file", &e));
        }
    }
    let name = web_container_name(username).map_err(|e| invalid(e.to_string()))?;
    let docker = &context.docker;
    let tested = match docker.run_nginx(&name, NginxCommand::Test).await {
        // Nothing runs: the files are kept and the container reads them when it starts.
        Err(DockerError::NotFound | DockerError::Conflict(_)) => return Ok(false),
        Err(e) => {
            restore();
            return Err(docker_error(&e));
        }
        Ok(result) => result,
    };
    if tested.exit_code != 0 {
        restore();
        // The web server that was running still runs the configuration it had.
        return Err(invalid(format!(
            "the web server refused the configuration: {}",
            reason(&tested.output)
        )));
    }
    match docker.run_nginx(&name, NginxCommand::Reload).await {
        Ok(result) if result.exit_code == 0 => Ok(true),
        Ok(result) => {
            restore();
            // Best effort: the old files back in use.
            let _ = docker.run_nginx(&name, NginxCommand::Reload).await;
            Err(AgentError::new(
                ErrorCode::Internal,
                format!("the web server did not reload: {}", reason(&result.output)),
            )
            .retryable())
        }
        Err(e) => {
            restore();
            Err(docker_error(&e))
        }
    }
}

fn parse(username: &str, domain: &str) -> Result<(TenantUsername, Fqdn), AgentError> {
    let username = TenantUsername::parse(username).map_err(|e| invalid(e.to_string()))?;
    let fqdn = Fqdn::parse(domain).map_err(|e| invalid(e.to_string()))?;
    if fqdn.as_str() != domain {
        return Err(invalid("the domain is not in its plain form"));
    }
    Ok((username, fqdn))
}

/// `attach_domain`.
///
/// # Errors
/// [`AgentError`]: bad arguments or a configuration the web server refuses (not
/// retryable; the previous file is back), Docker or the disk failing (retryable).
pub async fn attach_domain(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::AttachDomain {
        username,
        domain,
        config,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let (username, domain) = parse(&username, &domain)?;
    check_config(&config)?;
    let applied = change(
        &context,
        &username,
        &[Change {
            domain: domain.as_str().to_owned(),
            content: Some(config),
        }],
    )
    .await?;
    Ok(Output::DomainAttached { applied })
}

/// `detach_domain`.
///
/// # Errors
/// As [`attach_domain`].
pub async fn detach_domain(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::DetachDomain { username, domain } = op else {
        return Err(super::wrong_operation());
    };
    let (username, domain) = parse(&username, &domain)?;
    let applied = change(
        &context,
        &username,
        &[Change {
            domain: domain.as_str().to_owned(),
            content: None,
        }],
    )
    .await?;
    Ok(Output::DomainDetached { applied })
}

/// `set_php_ini`.
///
/// # Errors
/// [`AgentError`]: a file that has an option outside the list or a value that does
/// not fit (not retryable); the disk or Docker failing (retryable).
pub async fn set_php_ini(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::SetPhpIni { username, ini } = op else {
        return Err(super::wrong_operation());
    };
    let username = TenantUsername::parse(&username).map_err(|e| invalid(e.to_string()))?;
    if ini.len() > builtpanel_node_protocol::MAX_PHP_INI_BYTES {
        return Err(invalid("the file is too large"));
    }
    builtpanel_domain::php_ini::check_text(&ini).map_err(|e| invalid(e.to_string()))?;
    let _one_at_a_time = CHANGING.lock().await;
    let dir = php_ini_dir(&context.vhosts_root, &username)
        .map_err(|e| disk("prepare the directory", &e))?;
    let path = dir.join("builtpanel.ini");
    let content = (!ini.trim().is_empty()).then_some(ini.as_bytes());
    put(&dir, &path, content).map_err(|e| disk("write the file", &e))?;
    // The running PHP containers load it again, finishing the requests in progress.
    let mut reloaded = 0;
    let php = super::tenant::php_container_names(&context.docker, &username).await?;
    for name in php {
        match context
            .docker
            .signal(&name, builtpanel_docker_manager::Signal::Usr2)
            .await
        {
            Ok(()) => reloaded += 1,
            // Not running: it reads the file when it starts.
            Err(DockerError::NotFound | DockerError::Conflict(_)) => {}
            Err(e) => return Err(docker_error(&e)),
        }
    }
    Ok(Output::PhpIniSet { reloaded })
}

/// `keep_domains`.
///
/// # Errors
/// As [`attach_domain`].
pub async fn keep_domains(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::KeepDomains { username, domains } = op else {
        return Err(super::wrong_operation());
    };
    let username = TenantUsername::parse(&username).map_err(|e| invalid(e.to_string()))?;
    if domains.len() > MAX_HOSTS_PER_TENANT {
        return Err(invalid("too many domains"));
    }
    let mut keep = std::collections::BTreeSet::new();
    for domain in &domains {
        keep.insert(parse(username.as_str(), domain)?.1.as_str().to_owned());
    }
    let dir = context.vhosts_root.join(username.as_str());
    let mut extra = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            // Only files this agent could have written: `<plain domain>.conf`.
            let Some(domain) = file.strip_suffix(".conf") else {
                continue;
            };
            if Fqdn::parse(domain).is_ok_and(|f| f.as_str() == domain) && !keep.contains(domain) {
                extra.push(Change {
                    domain: domain.to_owned(),
                    content: None,
                });
            }
        }
    }
    if extra.is_empty() {
        return Ok(Output::DomainsKept {
            removed: 0,
            applied: true,
        });
    }
    let removed = extra.len();
    let applied = change(&context, &username, &extra).await?;
    Ok(Output::DomainsKept { removed, applied })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use builtpanel_docker_manager::DockerManager;

    use super::*;

    fn context(root: &Path) -> Arc<TenantContext> {
        Arc::new(TenantContext {
            helper_socket: "/nonexistent".into(),
            tenants_root: "/nonexistent".into(),
            docker: DockerManager::connect(Path::new("/")).unwrap(),
            vhosts_root: root.to_owned(),
        })
    }

    fn attach(domain: &str, config: &str) -> Operation {
        Operation::AttachDomain {
            username: "alice".into(),
            domain: domain.into(),
            config: config.into(),
        }
    }

    #[tokio::test]
    async fn bad_arguments_are_refused_before_any_file_is_written() {
        let root = std::env::temp_dir().join(format!("bp-vhost-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let c = context(&root);
        for op in [
            attach("Example.com", "server {}"),
            attach("example.com/../x", "server {}"),
            attach("example.com", ""),
            attach("example.com", "server {\u{0}}"),
            attach("example.com", &"x".repeat(MAX_VHOST_CONFIG_BYTES + 1)),
            attach("example.com", "server { root /etc; }"),
            attach("example.com", "server { root /var/www/html/../../etc; }"),
            attach("example.com", "server { alias /etc/passwd; }"),
            attach("example.com", "server { include /etc/shadow; }"),
            Operation::AttachDomain {
                username: "../root".into(),
                domain: "example.com".into(),
                config: "server {}".into(),
            },
            Operation::DetachDomain {
                username: "alice".into(),
                domain: "no way".into(),
            },
        ] {
            let err = match op {
                Operation::AttachDomain { .. } => attach_domain(c.clone(), op).await,
                _ => detach_domain(c.clone(), op).await,
            }
            .unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArguments);
            assert!(!err.retryable);
        }
        assert!(!root.exists(), "nothing was created");
    }

    #[tokio::test]
    async fn when_the_web_server_cannot_be_asked_the_file_is_put_back() {
        let root = std::env::temp_dir().join(format!("bp-vhost-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Docker is not there at all: that is a failure (not "no such container"),
        // and the file as it was comes back.
        let c = context(&root);
        let err = attach_domain(c.clone(), attach("example.com", "server {}"))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal, "{err:?}");
        assert!(!root.join("alice/example.com.conf").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn keep_domains_checks_its_list_and_does_nothing_when_nothing_is_extra() {
        let root = std::env::temp_dir().join(format!("bp-vhost-keep2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let c = context(&root);
        let keep = |domains: &[&str]| Operation::KeepDomains {
            username: "alice".into(),
            domains: domains.iter().map(|d| (*d).to_owned()).collect(),
        };
        assert_eq!(
            keep_domains(c.clone(), keep(&["Example.com"]))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidArguments
        );
        // Nothing there: nothing to remove, and no web server needs asking.
        assert_eq!(
            keep_domains(c.clone(), keep(&["example.com"]))
                .await
                .unwrap(),
            Output::DomainsKept {
                removed: 0,
                applied: true
            }
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn writes_are_all_or_nothing_and_readable_by_the_container() {
        let root = std::env::temp_dir().join(format!("bp-vhost-put-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let alice = TenantUsername::parse("alice").unwrap();
        let dir = conf_dir(&root, &alice).unwrap();
        let domain = Fqdn::parse("example.com").unwrap();
        let path = dir.join(format!("{}.conf", domain.as_str()));
        put(&dir, &path, Some(b"one")).unwrap();
        put(&dir, &path, Some(b"two")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&dir), mode(&path)), (0o755, 0o644));
        // Nothing but the site file is left behind for the web server to find.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["example.com.conf"]);
        put(&dir, &path, None).unwrap();
        put(&dir, &path, None).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nginx_messages_are_shortened_to_the_line_that_matters() {
        let text = "nginx: [emerg] unknown directive \"foo\" in /etc/nginx/conf.d/a.conf:3\nnginx: configuration file /etc/nginx/nginx.conf test failed\n";
        assert_eq!(
            reason(text),
            "nginx: [emerg] unknown directive \"foo\" in /etc/nginx/conf.d/a.conf:3"
        );
        assert_eq!(reason(""), "no reason given");
        assert!(reason(&"x".repeat(1000)).len() <= 300);
    }
}
