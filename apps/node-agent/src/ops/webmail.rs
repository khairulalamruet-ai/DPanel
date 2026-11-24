//! Webmail for clients (T13.09), signed in without a password.
//!
//! One container, `bp-webmail`, runs Roundcube. Its sign-in form is left alone for anyone who
//! has a password, but a plugin of our own (`dhsignon`) takes a one-time link as well: the
//! control plane makes a token, keeps it to itself and tells the node only its SHA-256; the
//! node writes the credentials of a sign-in as the mailbox (the mail server's administrator
//! signing in as that address, which the mail server allows) into a file named by that hash,
//! and when the client opens the link the plugin moves that file away (so the link works once),
//! reads it and signs the client in. The mailbox's own password is never known to the panel.

use std::{path::PathBuf, sync::Arc, time::Duration};

use builtpanel_docker_manager::{
    Capability, ContainerSpec, DockerError, Network, PublishedPort, Restart, Tmpfs, VolumeMount,
};
use builtpanel_node_protocol::{AgentError, ErrorCode, Operation, Output};
use builtpanel_tenant_stack::{Allowlist, Role};

use super::{
    db, mail,
    pma::{disk, now, set_mode, write_private},
    sqlimport::{failed, invalid},
    tenant::{TenantContext, docker_error},
};

/// The container's name.
pub const CONTAINER: &str = "bp-webmail";
const MIB: u64 = 1024 * 1024;
/// How long a link works after it was made.
const LINK_SECS: i64 = 300;

/// Loopback port of the container (`BUILTPANEL_AGENT_WEBMAIL_PORT`).
#[must_use]
pub fn port() -> u16 {
    std::env::var("BUILTPANEL_AGENT_WEBMAIL_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p >= 1024)
        .unwrap_or(18_083)
}

fn base(context: &TenantContext) -> PathBuf {
    context.vhosts_root.with_file_name("webmail")
}

fn sessions_dir(context: &TenantContext) -> PathBuf {
    base(context).join("sessions")
}

/// The plugin: on `?_dhsignon=<token>` it takes the credentials kept under the token's hash
/// (once: whoever moves the file away first has the only sign-in it will ever give) and lets
/// Roundcube's own sign-in use them.
const PLUGIN: &str = r#"<?php
class dhsignon extends rcube_plugin
{
    public function init()
    {
        $this->add_hook('startup', [$this, 'startup']);
        $this->add_hook('authenticate', [$this, 'authenticate']);
    }

    private function say($code, $text)
    {
        http_response_code($code);
        header('Content-Type: text/plain; charset=utf-8');
        header('Cache-Control: no-store');
        echo $text . "\n";
        exit;
    }

    public function startup($args)
    {
        if (isset($_GET['_dhsignon']) && empty($_SESSION['user_id'])) {
            $args['task'] = 'login';
            $args['action'] = 'login';
        }
        return $args;
    }

    public function authenticate($args)
    {
        if (!isset($_GET['_dhsignon'])) {
            return $args;
        }
        $t = (string) $_GET['_dhsignon'];
        if (!preg_match('/^[0-9a-f]{64}$/', $t)) {
            $this->say(403, 'This link does not work.');
        }
        $file = '/dhsessions/' . hash('sha256', $t);
        $mine = $file . '.' . bin2hex(random_bytes(8));
        if (!@rename($file, $mine)) {
            $this->say(403, 'This link does not work, or it was used already. Open webmail again from the panel.');
        }
        $raw = @file_get_contents($mine);
        @unlink($mine);
        $d = json_decode((string) $raw, true);
        if (!is_array($d) || (int) ($d['link_expires'] ?? 0) < time()) {
            $this->say(403, 'This link has run out. Open webmail again from the panel.');
        }
        $args['user'] = (string) $d['user'];
        $args['pass'] = (string) $d['password'];
        $args['cookiecheck'] = false;
        $args['valid'] = true;
        return $args;
    }
}
"#;

/// Run by the image after it has put Roundcube in place: the plugin goes into its folder.
const INSTALL_SCRIPT: &str = "#!/bin/sh\nset -e\nmkdir -p /var/www/html/plugins/dhsignon\ncp /entrypoint-tasks/post-setup/dhsignon.php /var/www/html/plugins/dhsignon/dhsignon.php\n";

/// Settings that are not Roundcube's defaults: the mail server is in the same Docker network and
/// has a certificate of its own making until the panel gives it a real one, so the connection is
/// not checked against a name; nothing about the sign-in is offered that the panel does not.
const CONFIG: &str = r#"<?php
$config['imap_conn_options'] = ['ssl' => ['verify_peer' => false, 'verify_peer_name' => false]];
$config['smtp_conn_options'] = ['ssl' => ['verify_peer' => false, 'verify_peer_name' => false]];
$config['product_name'] = 'Webmail';
$config['session_lifetime'] = 60;
$config['login_autocomplete'] = 0;
$config['enable_installer'] = false;
$config['support_url'] = '';
$config['imap_cache'] = null;
$config['messages_cache'] = false;
"#;

fn write_file(path: &std::path::Path, text: &str) -> Result<(), AgentError> {
    std::fs::write(path, text).map_err(|e| disk("cannot write a webmail file", &e))
}

/// Makes sure the container runs.
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
                .current(Role::Webmail)
                .map_err(|e| AgentError::new(ErrorCode::Internal, e.to_string()))?
                .clone();
            let base = base(context);
            let (sessions, tasks, config) = (
                sessions_dir(context),
                base.join("tasks"),
                base.join("config"),
            );
            for dir in [&sessions, &tasks, &config] {
                std::fs::create_dir_all(dir).map_err(|e| disk("cannot make the folder", &e))?;
            }
            // The page in the container (www-data there) moves the files away, so it needs the
            // folder; nobody else on the machine can reach it (the state folder is the agent's).
            set_mode(&sessions, 0o777)?;
            // The image copies Roundcube into place and then runs what is in this folder; the
            // plugin is put in the plugins folder by the script (a folder mounted over it would
            // be left out of the copy).
            write_file(&tasks.join("dhsignon.php"), PLUGIN)?;
            let script = tasks.join("10-dhsignon.sh");
            write_file(&script, INSTALL_SCRIPT)?;
            set_mode(&script, 0o755)?;
            write_file(&config.join("dh.inc.php"), CONFIG)?;
            let state = context
                .vhosts_root
                .parent()
                .ok_or_else(|| failed("the state folder is unknown"))?
                .to_path_buf();
            let mut mounts = Vec::new();
            for (name, path, target, read_only) in [
                ("bp-webmail-sessions", &sessions, "/dhsessions", false),
                (
                    "bp-webmail-tasks",
                    &tasks,
                    "/entrypoint-tasks/post-setup",
                    true,
                ),
                ("bp-webmail-config", &config, "/var/roundcube/config", true),
            ] {
                let volume = db::name(name)?;
                docker
                    .create_bind_volume(&volume, path, &state)
                    .await
                    .map_err(|e| docker_error(&e))?;
                mounts.push(VolumeMount {
                    volume,
                    target: target.into(),
                    read_only,
                });
            }
            let data = db::name("bp-webmail-data")?;
            docker
                .create_volume(&data)
                .await
                .map_err(|e| docker_error(&e))?;
            mounts.push(VolumeMount {
                volume: data,
                target: "/var/roundcube/db".into(),
                read_only: false,
            });
            let network = db::name(mail::NETWORK)?;
            docker
                .ensure_network(&network, false)
                .await
                .map_err(|e| docker_error(&e))?;
            let image = super::tenant::pull_with_fallback(docker, &allowlist, &image).await?;
            let mut spec = ContainerSpec::new(container.clone(), image);
            for (key, value) in [
                ("ROUNDCUBEMAIL_DEFAULT_HOST", "ssl://bp-mail"),
                ("ROUNDCUBEMAIL_DEFAULT_PORT", "993"),
                ("ROUNDCUBEMAIL_SMTP_SERVER", "ssl://bp-mail"),
                ("ROUNDCUBEMAIL_SMTP_PORT", "465"),
                ("ROUNDCUBEMAIL_PLUGINS", "dhsignon,archive,zipdownload"),
                ("ROUNDCUBEMAIL_SKIN", "elastic"),
                ("ROUNDCUBEMAIL_DB_TYPE", "sqlite"),
            ] {
                spec.env.insert(key.into(), value.into());
            }
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
            spec.memory_bytes = 384 * MIB;
            spec.cpu_millis = 1000;
            spec.pids_limit = 256;
            spec.tmpfs = vec![Tmpfs {
                target: "/tmp".into(),
                size_bytes: 64 * MIB,
            }];
            spec.volumes = mounts;
            spec.network = Network::Named(network);
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
        }
        Err(e) => return Err(docker_error(&e)),
    }
    docker
        .start(&container)
        .await
        .map_err(|e| docker_error(&e))?;
    Ok(())
}

/// `open_webmail`.
///
/// # Errors
/// [`AgentError`]: bad arguments, or a mail server that was never set up here (not retryable);
/// Docker failing (retryable).
pub async fn open_webmail(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::OpenWebmail {
        address,
        token_hash,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let address = mail::address_of(&address)?;
    if token_hash.len() != 64
        || !token_hash
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(invalid("the token's hash is not 64 lower-case hex digits"));
    }
    let (admin, secret) = mail::admin_credentials(&context)
        .ok_or_else(|| failed("the mail server is not set up on this node"))?;
    sweep(&context);
    ensure_container(&context).await?;
    let link_expires = now() + LINK_SECS;
    let body = serde_json::json!({
        "user": format!("{address}%{admin}"),
        "password": secret,
        "link_expires": link_expires,
    })
    .to_string();
    let dir = sessions_dir(&context);
    let tmp = dir.join(format!(".{token_hash}.tmp"));
    write_private(&tmp, &body).map_err(|e| disk("cannot keep the session", &e))?;
    std::fs::rename(&tmp, dir.join(&token_hash))
        .map_err(|e| disk("cannot keep the session", &e))?;
    Ok(Output::WebmailOpened {
        link_expires_at: link_expires,
    })
}

/// Removes links nobody used.
pub fn sweep(context: &TenantContext) {
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
