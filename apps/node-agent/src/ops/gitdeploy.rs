//! Deploying a site or an application from a git repository (T12.09).
//!
//! Everything runs in one-off containers as the tenant's own user, on the tenant's network (the
//! only way out is the network of the account, as for the account's own applications):
//!
//! 1. **fetch**: `git clone --depth 1` of one branch into a new release folder,
//!    `.bp-releases/<name>/<time>` below the web directory (over http(s) only; the token of a
//!    private repository is in a configuration file that is mounted read-only, never in the
//!    container's command line or environment);
//! 2. **build**: each build step is a list of words run in the runtime's image (Node.js or Python)
//!    in the release folder, no shell;
//! 3. **switch**: the folder `dest` becomes a link to the new release with one rename, so
//!    visitors see the old release or the new one and never something in between; older
//!    releases beyond the last few are removed;
//! 4. an application that runs the files is restarted.
//!
//! A deploy that fails before the switch leaves what was live as it was.

use std::{os::unix::fs::PermissionsExt as _, sync::Arc, time::Duration};

use builtpanel_docker_manager::{ContainerSpec, DockerError, Name, Network, Tmpfs, VolumeMount};
use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{AgentError, ErrorCode, Operation, Output};
use builtpanel_tenant_stack::{Allowlist, Role, files_volume_name, network_name};

use super::{
    db,
    scan::state,
    sqlimport::failed,
    tenant::{TenantContext, docker_error, pull_with_fallback},
};

/// Releases kept besides the one in use.
const KEEP_RELEASES: u32 = 4;
/// How long the fetch and each build step may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// What stays of each step's output in the log.
const LOG_PER_STEP: usize = 1500;
const APP_ROOT: &str = "/app";
const CONFIG_DIR: &str = "/opt/bp-git";

fn invalid(what: &str) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, what.to_owned())
}

fn valid_name(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A folder below the web directory: plain, relative, no `..`, not empty.
fn valid_dest(dir: &str) -> bool {
    !dir.is_empty()
        && dir.len() <= 200
        && !dir.starts_with('/')
        && !dir
            .split('/')
            .any(|p| p.is_empty() || p == ".." || p == "." || p.starts_with(".bp-"))
        && dir
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
}

fn valid_branch(b: &str) -> bool {
    !b.is_empty()
        && b.len() <= 100
        && !b.starts_with('-')
        && !b.split('/').any(|p| p == ".." || p == ".")
        && b.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'/' | b'-'))
}

/// An http(s) address without a login, with no white space or control characters.
fn valid_repo(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    url.len() <= 300
        && !host.is_empty()
        && !host.contains('@')
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Standard base64 (RFC 4648), padded.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(char::from(T[(n >> 18) as usize & 63]));
        out.push(char::from(T[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(T[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(T[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

/// The git configuration that sends a token with every request to the repository's host.
#[must_use]
pub fn git_config(token: &str) -> String {
    // `user:token` is used as written; a bare token is sent as `x-access-token`, which GitHub and
    // others accept.
    let pair = if token.contains(':') {
        token.to_owned()
    } else {
        format!("x-access-token:{token}")
    };
    format!(
        "[http]\n\textraheader = Authorization: Basic {}\n",
        base64(pair.as_bytes())
    )
}

fn tail(text: &str, chars: usize) -> String {
    let all: Vec<char> = text.trim().chars().collect();
    all[all.len().saturating_sub(chars)..].iter().collect()
}

/// How many `../` lead from the link's folder to the web directory.
fn up(dest: &str) -> String {
    "../".repeat(dest.matches('/').count())
}

const FETCH_SCRIPT: &str = "set -e; umask 022; rel=\"$3\"; mkdir -p \"$(dirname \"$rel\")\"; rm -rf \"$rel\"; \
     git clone --quiet --depth 1 --branch \"$2\" --single-branch -- \"$1\" \"$rel\" \
       || { rm -rf \"$rel\"; git clone --quiet --branch \"$2\" --single-branch -- \"$1\" \"$rel\"; }; \
     echo \"DHCOMMIT=$(git -C \"$rel\" rev-parse HEAD)\"; rm -rf \"$rel/.git\"";

const SWITCH_SCRIPT: &str = "set -e; target=\"$1\"; dest=\"$2\"; dir=\"$3\"; keep=\"$4\"; \
     if [ -e \"$dest\" ] && [ ! -L \"$dest\" ]; then echo \"$dest exists and is not a link made by a deploy\" >&2; exit 3; fi; \
     mkdir -p \"$(dirname \"$dest\")\"; rm -f \"$dest.bp-new\"; ln -s \"$target\" \"$dest.bp-new\"; mv -T \"$dest.bp-new\" \"$dest\"; \
     cd \"$dir\"; ls -1t | tail -n +$((keep + 1)) | while read -r d; do case \"$d\" in ''|*[!0-9]*) ;; *) rm -rf -- \"$d\" ;; esac; done";

/// `run_git_deploy`.
///
/// # Errors
/// [`AgentError`]: bad arguments; Docker failing (retryable); a fetch, a build or a switch that
/// fails (not retryable; the message ends with what the step printed).
#[allow(clippy::too_many_lines)]
pub async fn run_git_deploy(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::RunGitDeploy {
        deploy_id,
        tenant_id: _,
        username,
        uid,
        name,
        repo_url,
        branch,
        dest,
        build_kind,
        build_version,
        build_steps,
        restart_app,
        token,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if uid < 10_000
        || !valid_name(&name)
        || !valid_repo(&repo_url)
        || !valid_branch(&branch)
        || !valid_dest(&dest)
        || build_steps.len() > 5
        || build_steps.iter().any(|s| {
            s.is_empty()
                || s.len() > 32
                || s.iter()
                    .any(|w| w.is_empty() || w.len() > 500 || w.contains('\0'))
        })
        || restart_app.as_deref().is_some_and(|a| !valid_name(a))
        || token.as_ref().is_some_and(|t| {
            t.0.is_empty()
                || t.0.len() > 300
                || t.0.chars().any(|c| c.is_whitespace() || c.is_control())
        })
    {
        return Err(invalid("the deploy's settings are not valid"));
    }
    let allowlist = Allowlist::embedded().map_err(|e| failed(&format!("the image list: {e}")))?;
    let git_image = pull_with_fallback(
        &context.docker,
        &allowlist,
        &allowlist
            .current(Role::Git)
            .map_err(|e| failed(&e.to_string()))?
            .clone(),
    )
    .await?;
    let build_image = match (&build_kind, &build_version, build_steps.is_empty()) {
        (_, _, true) => None,
        (Some(kind), Some(version), false) => {
            let role = match kind.as_str() {
                "node" => Role::Node,
                "python" => Role::Python,
                _ => return Err(invalid("the build runs on node or python")),
            };
            let image = allowlist
                .runtime_image(role, version)
                .map_err(|e| invalid(&e.to_string()))?
                .clone();
            Some((
                role,
                pull_with_fallback(&context.docker, &allowlist, &image).await?,
            ))
        }
        _ => return Err(invalid("build steps need a runtime and its version")),
    };
    let docker = &context.docker;
    let network = network_name(&username).map_err(|e| invalid(&e.to_string()))?;
    docker
        .ensure_network(&network, false)
        .await
        .map_err(|e| docker_error(&e))?;
    let files = files_volume_name(&username).map_err(|e| invalid(&e.to_string()))?;
    // Milliseconds: two deploys in one second must not share a folder.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let release_dir = format!(".bp-releases/{name}");
    let release = format!("{release_dir}/{stamp}");
    let short = &deploy_id.simple().to_string()[..8];

    // The token's configuration file, in a folder of the agent's state, mounted read-only.
    let config_dir = state(&context)?
        .join("gitdeploy")
        .join(format!("{}-{name}", username.as_str()));
    let config_volume = db::name(&format!("bp-{}-git-{name}-cfg", username.as_str()))?;
    let _ = docker.remove_volume(&config_volume).await;
    if let Some(token) = &token {
        std::fs::create_dir_all(&config_dir)
            .map_err(|e| failed(&format!("cannot make the folder: {e}")))?;
        let file = config_dir.join("config");
        std::fs::write(&file, git_config(&token.0))
            .map_err(|e| failed(&format!("cannot write the configuration: {e}")))?;
        for (path, mode) in [(&config_dir, 0o755), (&file, 0o644)] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .map_err(|e| failed(&format!("cannot set rights: {e}")))?;
        }
        docker
            .create_bind_volume(&config_volume, &config_dir, &state(&context)?)
            .await
            .map_err(|e| docker_error(&e))?;
    }

    let job = |suffix: &str,
               image: &builtpanel_docker_manager::ImageRef,
               workdir: &str,
               tmpfs_mb: u64,
               memory_mb: u64|
     -> Result<ContainerSpec, AgentError> {
        let mut spec = ContainerSpec::new(
            db::name(&format!("bp-{}-gitjob-{short}-{suffix}", username.as_str()))?,
            image.clone(),
        );
        spec.user = Some((uid, uid));
        spec.cpu_millis = 1000;
        spec.pids_limit = 256;
        spec.memory_bytes = memory_mb * 1024 * 1024;
        spec.tenant = None;
        spec.working_dir = Some(workdir.to_owned());
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
        spec.env.insert("CI".into(), "true".into());
        Ok(spec)
    };
    let run = |spec: ContainerSpec| async move {
        let _ = docker.remove(&spec.name, true).await;
        docker
            .run_to_end(&spec, STEP_TIMEOUT)
            .await
            .map_err(|e| docker_error(&e))
    };

    let mut log = String::new();
    let outcome = async {
        // 1. fetch
        let mut spec = job("fetch", &git_image, APP_ROOT, 64, 512)?;
        spec.env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        spec.env
            .insert("GIT_ALLOW_PROTOCOL".into(), "https:http".into());
        if token.is_some() {
            spec.env
                .insert("GIT_CONFIG_GLOBAL".into(), format!("{CONFIG_DIR}/config"));
            spec.volumes.push(VolumeMount {
                volume: config_volume.clone(),
                target: CONFIG_DIR.into(),
                read_only: true,
            });
        }
        spec.entrypoint = vec![
            "sh".into(),
            "-c".into(),
            FETCH_SCRIPT.into(),
            "bp-git".into(),
            repo_url.clone(),
            branch.clone(),
            release.clone(),
        ];
        let result = run(spec).await?;
        log.push_str(&format!(
            "$ git clone {branch}\n{}\n",
            tail(&result.output, LOG_PER_STEP)
        ));
        let commit = result
            .output
            .lines()
            .find_map(|l| l.trim().strip_prefix("DHCOMMIT="))
            .filter(|c| c.len() == 40 && c.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_owned);
        let Some(commit) = commit.filter(|_| result.exit_code == 0) else {
            return Err(failed(&format!(
                "fetching the repository failed (exit code {}): {}",
                result.exit_code,
                tail(&result.output, 600)
            )));
        };
        // 2. build
        if let Some((_, image)) = &build_image {
            for step in &build_steps {
                let mut spec = job("build", image, &format!("{APP_ROOT}/{release}"), 512, 1024)?;
                spec.env
                    .insert("npm_config_cache".into(), "/tmp/.npm".into());
                spec.entrypoint = vec![step[0].clone()];
                spec.command = step[1..].to_vec();
                let result = run(spec).await?;
                log.push_str(&format!(
                    "$ {}\n{}\n",
                    step.join(" "),
                    tail(&result.output, LOG_PER_STEP)
                ));
                if result.exit_code != 0 {
                    return Err(failed(&format!(
                        "the build step `{}` failed (exit code {}): {}",
                        step.join(" "),
                        result.exit_code,
                        tail(&result.output, 600)
                    )));
                }
            }
        }
        // 3. switch
        let mut spec = job("switch", &git_image, APP_ROOT, 8, 128)?;
        spec.entrypoint = vec![
            "sh".into(),
            "-c".into(),
            SWITCH_SCRIPT.into(),
            "bp-switch".into(),
            format!("{}{release}", up(&dest)),
            dest.clone(),
            release_dir.clone(),
            KEEP_RELEASES.to_string(),
        ];
        let result = run(spec).await?;
        if result.exit_code != 0 {
            return Err(failed(&format!(
                "switching to the new release failed: {}",
                tail(&result.output, 600)
            )));
        }
        Ok(commit)
    }
    .await;
    let _ = docker.remove_volume(&config_volume).await;
    let _ = std::fs::remove_dir_all(&config_dir);
    let commit = match outcome {
        Ok(c) => c,
        Err(mut e) => {
            // A release that did not go live is not kept.
            let mut spec = job("clean", &git_image, APP_ROOT, 8, 128)?;
            spec.entrypoint = vec!["rm".into(), "-rf".into(), "--".into(), release.clone()];
            let _ = run(spec).await;
            e.message = format!("{}\n--- log ---\n{}", e.message, tail(&log, 1500));
            return Err(e);
        }
    };
    // 4. the application that runs the files
    if let Some(app) = restart_app {
        let container = db::name(&format!("bp-{}-app-{app}", username.as_str()))?;
        match docker.restart(&container, Duration::from_secs(10)).await {
            Ok(()) | Err(DockerError::NotFound) => {}
            Err(e) => return Err(docker_error(&e)),
        }
    }
    Ok(Output::GitDeployed {
        commit,
        log: tail(&log, 6000),
    })
}

/// `remove_git_deploy`.
///
/// # Errors
/// [`AgentError`]: bad arguments.
pub async fn remove_git_deploy(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::RemoveGitDeploy {
        tenant_id: _,
        username,
        name,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if !valid_name(&name) {
        return Err(invalid("the deploy's name is not valid"));
    }
    let _ = context
        .docker
        .remove_volume(
            &Name::parse(&format!("bp-{}-git-{name}-cfg", username.as_str()))
                .map_err(|e| invalid(&e.to_string()))?,
        )
        .await;
    let _ = std::fs::remove_dir_all(
        state(&context)?
            .join("gitdeploy")
            .join(format!("{}-{name}", username.as_str())),
    );
    Ok(Output::GitDeployRemoved)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_examples() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), coded);
        }
    }

    #[test]
    fn the_token_is_sent_as_basic_authentication_only_to_git() {
        let config = git_config("abc123");
        assert!(config.contains(&format!("Basic {}", base64(b"x-access-token:abc123"))));
        assert!(git_config("me:secret").contains(&format!("Basic {}", base64(b"me:secret"))));
    }

    #[test]
    fn repositories_branches_and_folders_are_checked() {
        for ok in ["https://github.com/a/b.git", "http://bp-x:8000/r.git"] {
            assert!(valid_repo(ok), "{ok}");
        }
        for bad in [
            "git@github.com:a/b.git",
            "ssh://git@x/y",
            "file:///etc",
            "https://u:p@x/y",
            "https://x/y z",
            "ext::sh -c id",
            "https://",
            "-oProxyCommand=x",
        ] {
            assert!(!valid_repo(bad), "{bad}");
        }
        for ok in ["main", "feature/x-1", "v1.2.3"] {
            assert!(valid_branch(ok), "{ok}");
        }
        for bad in ["", "-x", "a/../b", "a b", "a;b", "--upload-pack=x"] {
            assert!(!valid_branch(bad), "{bad}");
        }
        for ok in ["site", "apps/site"] {
            assert!(valid_dest(ok), "{ok}");
        }
        for bad in ["", "/x", "../x", "a//b", ".bp-releases/x", "a/.bp-x", "a b"] {
            assert!(!valid_dest(bad), "{bad}");
        }
        assert_eq!(up("site"), "");
        assert_eq!(up("apps/site"), "../");
    }
}
