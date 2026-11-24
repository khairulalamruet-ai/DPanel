//! A tenant's own application (T12.07 Node.js, T12.08 Python): a container of the tenant.
//!
//! The container sits on the tenant's network only (no port is published) under the name
//! `bp-<account>-app-<name>`, where the account's web server reaches it. It runs the account's
//! files from one folder as the account's own user, with a read-only root, a memory limit and no
//! capabilities. The command is a list of words that is run as it is. The environment is
//! written to a file in a folder of the agent's state (mounted read-only) and read by a fixed
//! one-line start-up script, so that values with secrets never appear in the container's
//! description; the script is the only thing the container runs through `sh`.
//!
//! The dependencies are installed first (with `install`) in a one-off container: `npm ci` or
//! `npm install` when the folder has a `package.json`, `pip install -r requirements.txt` into
//! `.deps` when it has a `requirements.txt`.

use std::{os::unix::fs::PermissionsExt as _, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerError, Network, Restart, Tmpfs, VolumeMount};
use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{AgentError, ErrorCode, Operation, Output, RuntimeEnv};
use builtpanel_tenant_stack::{Allowlist, Role, files_volume_name, network_name};

use super::{
    db,
    scan::state,
    sqlimport::failed,
    tenant::{TenantContext, docker_error, pull_with_fallback},
};

/// How long the install step may take.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Where the account's files are inside the container.
const APP_ROOT: &str = "/app";
/// Where the environment file is inside the container.
const ENV_DIR: &str = "/opt/bp-app";
/// What starts the application: read the environment, then become the command.
const START_SCRIPT: &str = ". /opt/bp-app/env.sh && exec \"$@\"";

fn invalid(what: &str) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, what.to_owned())
}

fn role_of(kind: &str) -> Result<Role, AgentError> {
    match kind {
        "node" => Ok(Role::Node),
        "python" => Ok(Role::Python),
        _ => Err(invalid("the application is node or python")),
    }
}

fn valid_name(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A folder below the web directory: plain, relative, no `..`.
fn valid_dir(dir: &str) -> bool {
    dir.len() <= 200
        && !dir.starts_with('/')
        && !dir.split('/').any(|p| p == ".." || p == ".")
        && !dir.contains("//")
        && dir
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
}

/// Names a client may not set: they decide where and what the process loads, or are the
/// panel's own.
const RESERVED_ENV: [&str; 8] = [
    "PATH",
    "HOME",
    "PORT",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "PYTHONPATH",
    "PYTHONHOME",
];

fn valid_env(env: &RuntimeEnv) -> Result<(), AgentError> {
    if env.0.len() > 50 {
        return Err(invalid("at most 50 environment variables"));
    }
    for (k, v) in &env.0 {
        let name_ok = !k.is_empty()
            && k.len() <= 64
            && k.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
            && k.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if !name_ok || RESERVED_ENV.contains(&k.as_str()) {
            return Err(invalid(
                "an environment variable's name is not allowed (capitals, digits and _)",
            ));
        }
        if v.len() > 4096 || v.contains('\0') {
            return Err(invalid("an environment value is too long or has a NUL"));
        }
    }
    Ok(())
}

/// `export NAME='value'` lines, with every `'` in a value closed, escaped and reopened.
#[must_use]
pub fn env_script(env: &[(String, String)]) -> String {
    let mut out = String::new();
    for (k, v) in env {
        out.push_str("export ");
        out.push_str(k);
        out.push_str("='");
        out.push_str(&v.replace('\'', "'\\''"));
        out.push_str("'\n");
    }
    out
}

fn container_name(
    username: &TenantUsername,
    name: &str,
) -> Result<builtpanel_docker_manager::Name, AgentError> {
    db::name(&format!("bp-{}-app-{name}", username.as_str()))
}

fn install_script(kind: &str) -> &'static str {
    if kind == "node" {
        "if [ -f package-lock.json ]; then npm ci --omit=dev --no-audit --no-fund; \
         elif [ -f package.json ]; then npm install --omit=dev --no-audit --no-fund; fi"
    } else {
        "if [ -f requirements.txt ]; then pip install --no-cache-dir \
         --disable-pip-version-check --target .deps -r requirements.txt; fi"
    }
}

/// A container of an approved image (T12.12). The image's own start-up is used; the account's
/// files are not mounted. It is built the way every container here is: no capabilities, no new
/// privileges, a memory limit, a limit on processes, the account's own network and nothing
/// else (the container description has no way to ask for more). A volume of its own, `/data`, is
/// kept between restarts.
#[allow(clippy::too_many_arguments)]
async fn set_container(
    context: &Arc<TenantContext>,
    app_id: uuid::Uuid,
    username: &str,
    uid: u32,
    name: &str,
    image: Option<&str>,
    port: u16,
    memory_mb: u32,
    env: RuntimeEnv,
) -> Result<Output, AgentError> {
    let username = TenantUsername::parse(username).map_err(|e| invalid(&e.to_string()))?;
    if uid < 10_000 || !valid_name(name) || port < 1024 || !(32..=4096).contains(&memory_mb) {
        return Err(invalid("the container's settings are not valid"));
    }
    valid_env(&env)?;
    let image = builtpanel_docker_manager::ImageRef::parse(
        image.ok_or_else(|| invalid("a container needs an image"))?,
    )
    .map_err(|e| invalid(&e.to_string()))?;
    let docker = &context.docker;
    docker
        .pull_image(&image)
        .await
        .map_err(|e| docker_error(&e))?;
    let container = container_name(&username, name)?;
    let network = network_name(&username).map_err(|e| invalid(&e.to_string()))?;
    docker
        .ensure_network(&network, false)
        .await
        .map_err(|e| docker_error(&e))?;
    match docker.remove(&container, true).await {
        Ok(()) | Err(DockerError::NotFound) => {}
        Err(e) => return Err(docker_error(&e)),
    }
    let mut spec = ContainerSpec::new(container.clone(), image);
    spec.cpu_millis = 1000;
    spec.pids_limit = 256;
    spec.tenant = None;
    // An image may write anywhere in its own file system (it is thrown away with the
    // container); what it keeps goes to /data.
    spec.writable_root = true;
    spec.role = Some("container".to_owned());
    spec.network = Network::Named(network);
    spec.memory_bytes = u64::from(memory_mb) * 1024 * 1024;
    spec.restart = Restart::UnlessStopped;
    spec.tmpfs = vec![Tmpfs {
        target: "/tmp".into(),
        size_bytes: 64 * 1024 * 1024,
    }];
    let data = db::name(&format!("bp-{}-app-{name}-data", username.as_str()))?;
    docker
        .create_volume(&data)
        .await
        .map_err(|e| docker_error(&e))?;
    spec.volumes = vec![VolumeMount {
        volume: data,
        target: "/data".into(),
        read_only: false,
    }];
    for (key, value) in env.0 {
        spec.env.insert(key, value);
    }
    spec.env.insert("PORT".into(), port.to_string());
    spec.labels
        .insert("app".into(), app_id.simple().to_string());
    docker
        .create_container(&spec)
        .await
        .map_err(|e| docker_error(&e))?;
    docker
        .start(&container)
        .await
        .map_err(|e| docker_error(&e))?;
    if let Err(why) = wait_up(docker, &container).await {
        let _ = docker.remove(&container, true).await;
        return Err(why);
    }
    Ok(Output::RuntimeAppSet {
        host: format!("{}:{port}", container.as_str()),
    })
}

/// `set_runtime_app`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable); Docker or the image failing (retryable); an
/// install that fails or an application that stops right after starting.
#[allow(clippy::too_many_lines)]
pub async fn set_runtime_app(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetRuntimeApp {
        app_id,
        tenant_id: _,
        username,
        uid,
        kind,
        version,
        name,
        dir,
        command,
        install,
        port,
        memory_mb,
        env,
        image,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    if kind == "container" {
        return set_container(
            &context,
            app_id,
            &username,
            uid,
            &name,
            image.as_deref(),
            port,
            memory_mb,
            env.unwrap_or(RuntimeEnv(Vec::new())),
        )
        .await;
    }
    let role = role_of(&kind)?;
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if uid < 10_000
        || !valid_name(&name)
        || !valid_dir(&dir)
        || port < 1024
        || !(32..=4096).contains(&memory_mb)
        || command.is_empty()
        || command.len() > 32
        || command
            .iter()
            .any(|w| w.is_empty() || w.len() > 500 || w.contains('\0'))
    {
        return Err(invalid("the application's settings are not valid"));
    }
    let env = env.unwrap_or(RuntimeEnv(Vec::new()));
    valid_env(&env)?;
    let allowlist = Allowlist::embedded().map_err(|e| failed(&format!("the image list: {e}")))?;
    let image = allowlist
        .runtime_image(role, &version)
        .map_err(|e| invalid(&e.to_string()))?
        .clone();
    let docker = &context.docker;
    let image = pull_with_fallback(docker, &allowlist, &image).await?;
    let container = container_name(&username, &name)?;
    let network = network_name(&username).map_err(|e| invalid(&e.to_string()))?;
    docker
        .ensure_network(&network, false)
        .await
        .map_err(|e| docker_error(&e))?;
    let files = files_volume_name(&username).map_err(|e| invalid(&e.to_string()))?;
    let workdir = if dir.is_empty() {
        APP_ROOT.to_owned()
    } else {
        format!("{APP_ROOT}/{dir}")
    };

    // The common shape of both containers: the tenant's own user, a read-only root, the
    // files in /app, scratch space in memory.
    let base = |container: builtpanel_docker_manager::Name, tmpfs_mb: u64| {
        let mut spec = ContainerSpec::new(container, image.clone());
        spec.user = Some((uid, uid));
        spec.cpu_millis = 1000;
        spec.pids_limit = 256;
        spec.tenant = None;
        spec.role = Some(kind.clone());
        spec.working_dir = Some(workdir.clone());
        spec.network = Network::Named(network.clone());
        spec.tmpfs = vec![Tmpfs {
            target: "/tmp".into(),
            size_bytes: tmpfs_mb * 1024 * 1024,
        }];
        spec.volumes = vec![VolumeMount {
            volume: files.clone(),
            target: APP_ROOT.into(),
            read_only: false,
        }];
        spec.env.insert("HOME".into(), "/tmp".into());
        spec.env
            .insert("npm_config_cache".into(), "/tmp/.npm".into());
        if kind == "python" {
            spec.env.insert("PYTHONUNBUFFERED".into(), "1".into());
            spec.env
                .insert("PYTHONDONTWRITEBYTECODE".into(), "1".into());
            spec.env
                .insert("PYTHONPATH".into(), format!("{workdir}/.deps"));
        }
        spec.labels
            .insert("app".into(), app_id.simple().to_string());
        spec
    };

    // The install step, in a container that exists only for it.
    if install {
        let job = db::name(&format!(
            "bp-{}-appinst-{}",
            username.as_str(),
            &app_id.simple().to_string()[..8]
        ))?;
        let _ = docker.remove(&job, true).await;
        let mut spec = base(job, 512);
        spec.memory_bytes = 1024 * 1024 * 1024;
        spec.entrypoint = vec!["sh".into(), "-c".into(), install_script(&kind).into()];
        let result = docker
            .run_to_end(&spec, INSTALL_TIMEOUT)
            .await
            .map_err(|e| docker_error(&e))?;
        if result.exit_code != 0 {
            let tail: String = result
                .output
                .chars()
                .rev()
                .take(600)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            return Err(failed(&format!(
                "installing the dependencies failed (exit code {}): {}",
                result.exit_code,
                tail.trim()
            )));
        }
    }

    // The environment, in a folder of the agent's state, mounted read-only.
    let dir_state = state(&context)?
        .join("apps")
        .join(format!("{}-{name}", username.as_str()));
    std::fs::create_dir_all(&dir_state)
        .map_err(|e| failed(&format!("cannot make the folder: {e}")))?;
    let script = dir_state.join("env.sh");
    std::fs::write(&script, env_script(&env.0))
        .map_err(|e| failed(&format!("cannot write the environment: {e}")))?;
    for (path, mode) in [(&dir_state, 0o755), (&script, 0o644)] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| failed(&format!("cannot set rights: {e}")))?;
    }
    // (The file is readable to the container's user; the folder is under the agent's state,
    // which only the agent and the container's user namespace reach.)
    let volume = db::name(&format!("bp-{}-app-{name}-env", username.as_str()))?;
    let _ = docker.remove_volume(&volume).await;
    docker
        .create_bind_volume(&volume, &dir_state, &state(&context)?)
        .await
        .map_err(|e| docker_error(&e))?;

    // The application. Replaced whole when it exists.
    match docker.remove(&container, true).await {
        Ok(()) | Err(DockerError::NotFound) => {}
        Err(e) => return Err(docker_error(&e)),
    }
    let mut spec = base(container.clone(), 64);
    spec.memory_bytes = u64::from(memory_mb) * 1024 * 1024;
    spec.restart = Restart::UnlessStopped;
    spec.env.insert("PORT".into(), port.to_string());
    if kind == "node" {
        spec.env.insert("NODE_ENV".into(), "production".into());
    }
    spec.volumes.push(VolumeMount {
        volume,
        target: ENV_DIR.into(),
        read_only: true,
    });
    spec.entrypoint = vec![
        "sh".into(),
        "-c".into(),
        START_SCRIPT.into(),
        "bp-app".into(),
    ];
    spec.command = command;
    docker
        .create_container(&spec)
        .await
        .map_err(|e| docker_error(&e))?;
    docker
        .start(&container)
        .await
        .map_err(|e| docker_error(&e))?;
    if let Err(why) = wait_up(docker, &container).await {
        // Not left to restart again and again.
        let _ = docker.remove(&container, true).await;
        return Err(why);
    }
    Ok(Output::RuntimeAppSet {
        host: format!("{}:{port}", container.as_str()),
    })
}

/// Waits until the application has been running steadily for a few seconds. One that stops (or
/// keeps restarting) is an error of the application, not of the node: it is not retried, and
/// the end of what it printed is part of the message.
async fn wait_up(
    docker: &builtpanel_docker_manager::DockerManager,
    container: &builtpanel_docker_manager::Name,
) -> Result<(), AgentError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    let mut up_since: Option<tokio::time::Instant> = None;
    loop {
        let info = docker
            .inspect(container)
            .await
            .map_err(|e| docker_error(&e))?;
        if info.running && info.status == "running" {
            let since = *up_since.get_or_insert_with(tokio::time::Instant::now);
            if since.elapsed() >= Duration::from_secs(4) {
                return Ok(());
            }
        } else {
            up_since = None;
            let stopped = matches!(info.status.as_str(), "exited" | "dead" | "restarting");
            if stopped || tokio::time::Instant::now() >= deadline {
                let printed = docker
                    .logs(container, 0, 4000)
                    .await
                    .map(|read| {
                        read.lines
                            .iter()
                            .map(|l| l.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                let tail: String = {
                    let chars: Vec<char> = printed.trim().chars().collect();
                    chars[chars.len().saturating_sub(500)..].iter().collect()
                };
                return Err(failed(&format!(
                    "the application stopped right after starting (exit code {}): {tail}",
                    info.exit_code
                        .map_or_else(|| "?".to_owned(), |c| c.to_string())
                )));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(failed("the application did not come up in time"));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// `remove_runtime_app`.
///
/// # Errors
/// [`AgentError`]: bad arguments; Docker failing (retryable).
pub async fn remove_runtime_app(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::RemoveRuntimeApp {
        tenant_id: _,
        username,
        name,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if !valid_name(&name) {
        return Err(invalid("the application's name is not valid"));
    }
    let docker = &context.docker;
    let container = container_name(&username, &name)?;
    let existed = match docker.remove(&container, true).await {
        Ok(()) => true,
        Err(DockerError::NotFound) => false,
        Err(e) => return Err(docker_error(&e)),
    };
    let _ = docker
        .remove_volume(&db::name(&format!(
            "bp-{}-app-{name}-env",
            username.as_str()
        ))?)
        .await;
    // A container's own data goes with it.
    let _ = docker
        .remove_volume(&db::name(&format!(
            "bp-{}-app-{name}-data",
            username.as_str()
        ))?)
        .await;
    let _ = std::fs::remove_dir_all(
        state(&context)?
            .join("apps")
            .join(format!("{}-{name}", username.as_str())),
    );
    Ok(Output::RuntimeAppRemoved { existed })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn values_are_quoted_so_that_nothing_in_them_runs() {
        let script = env_script(&[
            ("A".into(), "plain".into()),
            ("B".into(), "it's $(whoami) `id`; rm -rf /".into()),
            ("C".into(), "line\nbreak".into()),
        ]);
        assert!(script.contains("export A='plain'\n"));
        assert!(script.contains("export B='it'\\''s $(whoami) `id`; rm -rf /'\n"));
        assert!(script.contains("export C='line\nbreak'\n"));
    }

    #[test]
    fn names_folders_and_environments_are_checked() {
        for ok in ["api", "my-app", "a1"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["a", "-a", "a-", "A", "a b", "a_b", ""] {
            assert!(!valid_name(bad), "{bad}");
        }
        for ok in ["", "api", "apps/api", "a.b/c_d-e"] {
            assert!(valid_dir(ok), "{ok}");
        }
        for bad in ["/etc", "../x", "a/../b", "a//b", "a b", "a;b", "./a"] {
            assert!(!valid_dir(bad), "{bad}");
        }
        assert!(valid_env(&RuntimeEnv(vec![("DATABASE_URL".into(), "x".into())])).is_ok());
        for bad in ["path", "PATH", "PORT", "LD_PRELOAD", "A B", "1A", ""] {
            assert!(
                valid_env(&RuntimeEnv(vec![(bad.into(), "x".into())])).is_err(),
                "{bad}"
            );
        }
    }
}
