//! WordPress installs (T12.02): WP-CLI in a container of its own.
//!
//! The install runs in a one-off container made from the pinned `wpcli` image (PHP with
//! mysqli, gd and the rest, so the tenant's own PHP image does not have to have them at that
//! moment). The container runs as the tenant's own user id, on the tenant's network (it
//! reaches the database the way the site will), with the tenant's files mounted and nothing
//! else. The two passwords are not in the command line or the environment: they are two small
//! files in a folder of the agent's state that is mounted read-only for the run and removed
//! afterwards. The subcommands are the fixed ones below; what a caller supplies is checked
//! to be plain text first.

use std::{path::Path, sync::Arc, time::Duration};

use builtpanel_docker_manager::{Network, Tmpfs, VolumeMount};
use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{
    AgentError, ErrorCode, Operation, Output, WpCommand, WpItem, WpOverview, WpScan,
};
use builtpanel_tenant_stack::{Allowlist, Role, files_volume_name, network_name};

use super::{
    db,
    scan::{base_spec, state},
    sqlimport::failed,
    tenant::{TenantContext, docker_error},
};

const LIMIT: Duration = Duration::from_secs(5 * 60);

/// What the container runs. Every value arrives in an environment variable, so nothing is
/// ever put into the script text.
const SCRIPT: &str = r#"
set -eu
W="php -d memory_limit=512M /usr/local/bin/wp --path=$WP_PATH $WP_EXTRA"
if [ -e "$WP_PATH/wp-config.php" ] || [ -e "$WP_PATH/wp-load.php" ]; then
  echo "WordPress is in that folder already" >&2
  exit 3
fi
mkdir -p "$WP_PATH"
set -- core download
[ -z "$WP_VERSION" ] || set -- "$@" --version="$WP_VERSION"
[ -z "$WP_LOCALE" ] || set -- "$@" --locale="$WP_LOCALE"
$W "$@"
$W config create --dbname="$DB_NAME" --dbuser="$DB_USER" --dbhost="$DB_HOST" --dbpass="$(cat /run/dh/dbpass)" --skip-salts
$W config shuffle-salts
$W core install --url="$SITE_URL" --title="$SITE_TITLE" --admin_user="$ADMIN_USER" --admin_password="$(cat /run/dh/adminpass)" --admin_email="$ADMIN_EMAIL" --skip-email
for plugin in $PLUGINS; do
  $W plugin install "$plugin" --activate
done
echo "BP_VERSION=$($W core version)"
"#;

fn invalid(what: &str) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, what.to_owned())
}

pub(super) fn plain(text: &str, max: usize, extra: &str) -> bool {
    !text.is_empty()
        && text.len() <= max
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

/// Whether `url` is a plain http(s) address with nothing a shell or WordPress would trip on.
pub(super) fn plain_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() <= 300
        && url
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&()*+,;=%".contains(c))
}

/// A folder of the tenant's files, relative to `www`; empty for `www` itself.
pub(super) fn plain_folder(folder: &str) -> bool {
    folder.is_empty()
        || (folder.len() <= 200
            && !folder.starts_with('/')
            && folder.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            }))
}

/// `install_wordpress`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable), a folder that has WordPress already,
/// Docker or the image failing (retryable), WP-CLI ending badly (the last of what it said,
/// without the passwords).
pub async fn install_wordpress(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let uid = match &op {
        Operation::InstallWordPress { uid, .. } => *uid,
        _ => return Err(super::wrong_operation()),
    };
    install_as(context, op, (uid, uid)).await
}

/// [`install_wordpress`] running as the given user (a test passes another one).
///
/// # Errors
/// As [`install_wordpress`].
pub async fn install_as(
    context: Arc<TenantContext>,
    op: Operation,
    run_as: (u32, u32),
) -> Result<Output, AgentError> {
    use std::os::unix::fs::PermissionsExt as _;
    let Operation::InstallWordPress {
        app_id,
        tenant_id: _,
        username,
        uid: _,
        path,
        database,
        database_password,
        site,
        admin_password,
        version,
        locale,
        plugins,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let (Some(database_password), Some(admin_password)) = (database_password, admin_password)
    else {
        return Err(invalid("the passwords are missing"));
    };
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if !plain_folder(&path) {
        return Err(invalid("the folder must stay inside the tenant's files"));
    }
    if !plain(&database.name, 64, "_")
        || !plain(&database.user, 64, "_")
        || database_password.0.is_empty()
        || database_password.0.len() > 128
        || database_password.0.contains(['\0', '\n', '\r'])
        || admin_password.0.len() < 12
        || admin_password.0.len() > 128
        || admin_password.0.contains(['\0', '\n', '\r'])
    {
        return Err(invalid("a database name, user or password is not valid"));
    }
    if !plain_url(&site.url)
        || site.title.is_empty()
        || site.title.chars().count() > 100
        || site.title.chars().any(char::is_control)
        || !plain(&site.admin_user, 60, "_.-")
        || site.admin_user.len() < 3
        || !site.admin_email.contains('@')
        || site.admin_email.len() > 200
        || !site
            .admin_email
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c))
    {
        return Err(invalid(
            "the site's address, title or administrator is not valid",
        ));
    }
    if version
        .as_deref()
        .is_some_and(|v| !(plain(v, 20, ".") && v.starts_with(|c: char| c.is_ascii_digit())))
        || locale.as_deref().is_some_and(|l| !plain(l, 10, "_"))
        || plugins.len() > 10
        || plugins.iter().any(|p| !plain(p, 100, "-"))
    {
        return Err(invalid("the version, locale or a plugin name is not valid"));
    }
    let allowlist = Allowlist::embedded().map_err(|e| failed(&format!("the image list: {e}")))?;
    let image = allowlist
        .current(Role::Wpcli)
        .map_err(|e| failed(&e.to_string()))?
        .clone();
    context
        .docker
        .pull_image(&image)
        .await
        .map_err(|e| docker_error(&e))?;
    // The two passwords, as files for this run only.
    let short = app_id.simple().to_string();
    let short = &short[..12];
    let state_dir = state(&context)?;
    let secrets = state_dir.join("wpcli").join(short);
    let cleanup = |dir: &Path| {
        let _ = std::fs::remove_dir_all(dir);
    };
    cleanup(&secrets);
    std::fs::create_dir_all(&secrets)
        .map_err(|e| failed(&format!("cannot make the folder: {e}")))?;
    let written = (|| -> std::io::Result<()> {
        std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o755))?;
        for (name, text) in [
            ("dbpass", &database_password.0),
            ("adminpass", &admin_password.0),
        ] {
            let file = secrets.join(name);
            std::fs::write(&file, text)?;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))?;
        }
        Ok(())
    })();
    if let Err(e) = written {
        cleanup(&secrets);
        return Err(failed(&format!("cannot write the secrets: {}", e.kind())));
    }
    let env: Vec<(String, String)> = [
        ("WP_VERSION", version.as_deref().unwrap_or("")),
        ("WP_LOCALE", locale.as_deref().unwrap_or("")),
        ("DB_NAME", database.name.as_str()),
        ("DB_USER", database.user.as_str()),
        ("DB_HOST", "db"),
        ("SITE_URL", site.url.as_str()),
        ("SITE_TITLE", site.title.as_str()),
        ("ADMIN_USER", site.admin_user.as_str()),
        ("ADMIN_EMAIL", site.admin_email.as_str()),
        ("PLUGINS", plugins.join(" ").as_str()),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    let outcome = run(
        &context, &username, &secrets, short, image, run_as, &path, SCRIPT, env, LIMIT,
    )
    .await;
    cleanup(&secrets);
    let _ = context
        .docker
        .remove_volume(&db::name(&format!("bp-wpi-{short}"))?)
        .await;
    let done = outcome?;
    let scrub = |text: &str| {
        let mut out = text.to_owned();
        for secret in [&database_password.0, &admin_password.0] {
            out = out.replace(secret.as_str(), "***");
        }
        out
    };
    if done.exit_code != 0 {
        let tail: String = scrub(&done.output)
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ")
            .chars()
            .filter(|c| !c.is_control())
            .take(400)
            .collect();
        let code = if done.exit_code == 3 {
            ErrorCode::InvalidArguments
        } else {
            ErrorCode::Internal
        };
        return Err(AgentError::new(
            code,
            format!("WP-CLI ended with {}: {tail}", done.exit_code),
        ));
    }
    let version = done
        .output
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("BP_VERSION="))
        .unwrap_or("")
        .trim()
        .to_owned();
    Ok(Output::WordPressInstalled { version })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    context: &TenantContext,
    username: &TenantUsername,
    secrets: &Path,
    short: &str,
    image: builtpanel_docker_manager::ImageRef,
    run_as: (u32, u32),
    path: &str,
    script: &str,
    env: Vec<(String, String)>,
    limit: Duration,
) -> Result<builtpanel_docker_manager::CommandResult, AgentError> {
    let state_dir = state(context)?;
    let secrets_volume = db::name(&format!("bp-wpi-{short}"))?;
    context
        .docker
        .create_bind_volume(&secrets_volume, secrets, &state_dir)
        .await
        .map_err(|e| docker_error(&e))?;
    let files = files_volume_name(username).map_err(|e| invalid(&e.to_string()))?;
    let network = network_name(username).map_err(|e| invalid(&e.to_string()))?;
    let mut spec = base_spec(
        db::name(&format!("bp-wpi-{short}"))?,
        image,
        &["sh", "-c", script],
    );
    spec.cap_add = Vec::new();
    spec.user = Some(run_as);
    spec.memory_bytes = 1024 * 1024 * 1024;
    spec.pids_limit = 128;
    spec.network = Network::Named(network);
    spec.tmpfs = vec![Tmpfs {
        target: "/tmp".into(),
        size_bytes: 256 * 1024 * 1024,
    }];
    spec.volumes = vec![
        VolumeMount {
            volume: files,
            target: "/var/www/html".into(),
            read_only: false,
        },
        VolumeMount {
            volume: secrets_volume,
            target: "/run/dh".into(),
            read_only: true,
        },
    ];
    let folder = if path.is_empty() {
        "/var/www/html".to_owned()
    } else {
        format!("/var/www/html/{path}")
    };
    spec.env = [
        ("HOME", "/tmp"),
        ("WP_CLI_CACHE_DIR", "/tmp/wp-cache"),
        ("WP_PATH", folder.as_str()),
        ("WP_EXTRA", if run_as.0 == 0 { "--allow-root" } else { "" }),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .chain(env)
    .collect();
    let _ = context.docker.remove(&spec.name, true).await;
    context
        .docker
        .run_to_end(&spec, limit)
        .await
        .map_err(|e| docker_error(&e))
}

/// The part of the script every management command starts with.
const MANAGE_HEAD: &str = r#"
set -eu
W="php -d memory_limit=512M /usr/local/bin/wp --path=$WP_PATH $WP_EXTRA"
if [ ! -e "$WP_PATH/wp-load.php" ]; then
  echo "There is no WordPress in that folder" >&2
  exit 4
fi
"#;

const OVERVIEW: &str = r#"
echo "BP_CORE=$($W core version)"
echo "BP_UPDATE=$($W core check-update --field=version 2>/dev/null | head -1 || true)"
echo "BP_MAINT=$($W maintenance-mode status 2>&1 | head -1 || true)"
echo "BP_PLUGINS"
$W plugin list --fields=name,status,version,update,update_version --format=json
echo
echo "BP_THEMES"
$W theme list --fields=name,status,version,update,update_version --format=json
echo
"#;

const SCAN: &str = r#"
echo "BP_SUM_START"
$W core verify-checksums 2>&1 || true
echo "BP_SUM_END"
echo "BP_DEBUG=$($W config get WP_DEBUG 2>/dev/null | head -1 || true)"
echo "BP_FILEEDIT=$($W config get DISALLOW_FILE_EDIT 2>/dev/null | head -1 || true)"
if [ -f "$WP_PATH/wp-content/mu-plugins/builtpanel-hardening.php" ]; then echo "BP_XMLRPC=off"; else echo "BP_XMLRPC=on"; fi
echo "BP_MODE=$(stat -c %a "$WP_PATH/wp-config.php" 2>/dev/null || echo '')"
echo "BP_WRITABLE=$(find "$WP_PATH" -not -type l -perm -o+w 2>/dev/null | wc -l)"
"#;

const HARDEN: &str = r#"
if [ "$H_DEBUG" = 1 ]; then
  $W config set WP_DEBUG false --raw 2>&1
  $W config set WP_DEBUG_DISPLAY false --raw 2>&1
  echo "debug off"
fi
if [ "$H_EDIT" = 1 ]; then
  $W config set DISALLOW_FILE_EDIT true --raw 2>&1
  echo "file editor off"
fi
if [ "$H_XMLRPC" = 1 ]; then
  mkdir -p "$WP_PATH/wp-content/mu-plugins"
  printf '%s
' '<?php' '/* Switched on and off by the hosting panel. */' "add_filter( 'xmlrpc_enabled', '__return_false' );" > "$WP_PATH/wp-content/mu-plugins/builtpanel-hardening.php"
  echo "xmlrpc off"
fi
if [ "$H_PERMS" = 1 ]; then
  find "$WP_PATH" -not -type l -perm -o+w -exec chmod o-w {} + 2>&1 || true
  chmod 640 "$WP_PATH/wp-config.php" 2>&1 || true
  echo "permissions fixed"
fi
"#;

fn manage_script(command: &WpCommand) -> String {
    let body = match command {
        WpCommand::Overview => OVERVIEW,
        WpCommand::Scan => SCAN,
        WpCommand::Harden { .. } => HARDEN,
        WpCommand::UpdatePlugins { slug: Some(_) } => r#"$W plugin update "$WP_SLUG" 2>&1"#,
        WpCommand::UpdatePlugins { slug: None } => "$W plugin update --all 2>&1",
        WpCommand::UpdateThemes { slug: Some(_) } => r#"$W theme update "$WP_SLUG" 2>&1"#,
        WpCommand::UpdateThemes { slug: None } => "$W theme update --all 2>&1",
        WpCommand::UpdateCore { version: Some(_) } => {
            r#"$W core update --version="$WP_VERSION" 2>&1; $W core update-db 2>&1"#
        }
        WpCommand::UpdateCore { version: None } => "$W core update 2>&1; $W core update-db 2>&1",
        WpCommand::Maintenance { on: true } => "$W maintenance-mode activate 2>&1",
        WpCommand::Maintenance { on: false } => "$W maintenance-mode deactivate 2>&1",
        WpCommand::ResetAdminPassword { .. } => {
            r#"$W user update "$WP_USER" --user_pass="$(cat /run/dh/adminpass)" 2>&1"#
        }
    };
    format!("{MANAGE_HEAD}{body}")
}

/// Reads the scan script's output.
///
/// # Errors
/// The text is not what the script prints.
pub fn parse_scan(output: &str) -> Result<WpScan, String> {
    let (mut modified, mut extra, mut missing) = (Vec::new(), Vec::new(), Vec::new());
    let mut in_sum = false;
    let mut saw_end = false;
    let (mut debug, mut file_edit, mut xmlrpc_off) = (String::new(), String::new(), false);
    let (mut mode, mut writable) = (String::new(), 0u32);
    let mut checksums_ok = true;
    let clean = |p: &str| -> String { p.chars().filter(|c| !c.is_control()).take(300).collect() };
    for line in output.lines() {
        if line == "BP_SUM_START" {
            in_sum = true;
        } else if line == "BP_SUM_END" {
            in_sum = false;
            saw_end = true;
        } else if in_sum {
            if let Some(p) = line.strip_prefix("Warning: File doesn't verify against checksum: ") {
                checksums_ok = false;
                if modified.len() < 100 {
                    modified.push(clean(p));
                }
            } else if let Some(p) = line.strip_prefix("Warning: File should not exist: ") {
                checksums_ok = false;
                if extra.len() < 100 {
                    extra.push(clean(p));
                }
            } else if let Some(p) = line.strip_prefix("Warning: File is missing: ") {
                checksums_ok = false;
                if missing.len() < 100 {
                    missing.push(clean(p));
                }
            } else if line.starts_with("Error:") {
                checksums_ok = false;
            }
        } else if let Some(v) = line.strip_prefix("BP_DEBUG=") {
            debug = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("BP_FILEEDIT=") {
            file_edit = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("BP_XMLRPC=") {
            xmlrpc_off = v.trim() == "off";
        } else if let Some(v) = line.strip_prefix("BP_MODE=") {
            mode = v
                .trim()
                .chars()
                .filter(char::is_ascii_digit)
                .take(4)
                .collect();
        } else if let Some(v) = line.strip_prefix("BP_WRITABLE=") {
            writable = v.trim().parse().unwrap_or(0);
        }
    }
    if !saw_end {
        return Err("the checksum check did not finish".into());
    }
    Ok(WpScan {
        checksums_ok,
        modified,
        extra,
        missing,
        debug: debug == "1" || debug.eq_ignore_ascii_case("true"),
        file_edit_allowed: !(file_edit == "1" || file_edit.eq_ignore_ascii_case("true")),
        xmlrpc_disabled: xmlrpc_off,
        config_mode: mode,
        world_writable: writable,
    })
}

/// A line that is a JSON list of objects (PHP's own `[date] PHP Notice` lines are not).
fn is_list(line: &str) -> bool {
    line == "[]" || line.starts_with("[{")
}

/// Reads the overview script's output.
///
/// # Errors
/// The text is not what the script prints.
pub fn parse_overview(output: &str) -> Result<WpOverview, String> {
    let mut core = String::new();
    let mut update = String::new();
    let mut maintenance = false;
    let mut section = "";
    let (mut plugins, mut themes) = (String::new(), String::new());
    for line in output.lines() {
        if let Some(v) = line.strip_prefix("BP_CORE=") {
            core = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("BP_UPDATE=") {
            update = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("BP_MAINT=") {
            maintenance = v.to_ascii_lowercase().contains("is active");
        } else if line == "BP_PLUGINS" {
            section = "plugins";
        } else if line == "BP_THEMES" {
            section = "themes";
        } else if section == "plugins" && is_list(line) {
            // (Warnings and notices WP-CLI or PHP print around the list are not part of it.)
            plugins.push_str(line);
        } else if section == "themes" && is_list(line) {
            themes.push_str(line);
        }
    }
    let items = |json: &str| -> Result<Vec<WpItem>, String> {
        let list: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| {
            format!(
                "unreadable list: {e}: {}",
                json.chars()
                    .filter(|c| !c.is_control())
                    .take(400)
                    .collect::<String>()
            )
        })?;
        Ok(list
            .iter()
            .take(500)
            .map(|v| {
                let text = |k: &str| {
                    v.get(k)
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(100)
                        .collect::<String>()
                };
                WpItem {
                    name: text("name"),
                    status: text("status"),
                    version: text("version"),
                    update_available: text("update") == "available",
                    update_version: text("update_version"),
                }
            })
            .collect())
    };
    if core.is_empty() {
        return Err("no core version".into());
    }
    Ok(WpOverview {
        core,
        core_update: (!update.is_empty()).then_some(update),
        maintenance,
        plugins: items(&plugins)?,
        themes: items(&themes)?,
    })
}

/// `run_wp_cli`.
///
/// # Errors
/// [`AgentError`]: bad arguments or no WordPress there (not retryable), Docker or the image
/// failing (retryable), WP-CLI ending badly (the last of what it said, without the password).
pub async fn run_wp_cli(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let uid = match &op {
        Operation::RunWpCli { uid, .. } => *uid,
        _ => return Err(super::wrong_operation()),
    };
    manage_as(context, op, (uid, uid)).await
}

/// [`run_wp_cli`] running as the given user (a test passes another one).
///
/// # Errors
/// As [`run_wp_cli`].
pub async fn manage_as(
    context: Arc<TenantContext>,
    op: Operation,
    run_as: (u32, u32),
) -> Result<Output, AgentError> {
    use std::os::unix::fs::PermissionsExt as _;
    let Operation::RunWpCli {
        app_id,
        tenant_id: _,
        username,
        uid: _,
        path,
        command,
        password,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if !plain_folder(&path) {
        return Err(invalid("the folder must stay inside the tenant's files"));
    }
    let mut env: Vec<(String, String)> = Vec::new();
    let mut secret: Option<String> = None;
    match &command {
        WpCommand::UpdatePlugins { slug: Some(slug) }
        | WpCommand::UpdateThemes { slug: Some(slug) } => {
            if !plain(slug, 100, "-_.") {
                return Err(invalid("a plugin or theme name is not valid"));
            }
            env.push(("WP_SLUG".into(), slug.clone()));
        }
        WpCommand::UpdateCore { version: Some(v) } => {
            if !(plain(v, 20, ".") && v.starts_with(|c: char| c.is_ascii_digit())) {
                return Err(invalid("the version is not valid"));
            }
            env.push(("WP_VERSION".into(), v.clone()));
        }
        WpCommand::Harden {
            debug_off,
            file_edit_off,
            xmlrpc_off,
            fix_permissions,
        } => {
            for (name, on) in [
                ("H_DEBUG", debug_off),
                ("H_EDIT", file_edit_off),
                ("H_XMLRPC", xmlrpc_off),
                ("H_PERMS", fix_permissions),
            ] {
                env.push((name.into(), if *on { "1" } else { "0" }.into()));
            }
        }
        WpCommand::ResetAdminPassword { user } => {
            let Some(password) = password else {
                return Err(invalid("the password is missing"));
            };
            if !plain(user, 60, "_.-")
                || password.0.chars().count() < 12
                || password.0.len() > 128
                || password.0.chars().any(char::is_control)
            {
                return Err(invalid("the user or the password is not valid"));
            }
            env.push(("WP_USER".into(), user.clone()));
            secret = Some(password.0);
        }
        _ => {}
    }
    let allowlist = Allowlist::embedded().map_err(|e| failed(&format!("the image list: {e}")))?;
    let image = allowlist
        .current(Role::Wpcli)
        .map_err(|e| failed(&e.to_string()))?
        .clone();
    context
        .docker
        .pull_image(&image)
        .await
        .map_err(|e| docker_error(&e))?;
    let short = format!("{}m", &app_id.simple().to_string()[..11]);
    let secrets = state(&context)?.join("wpcli").join(&short);
    let _ = std::fs::remove_dir_all(&secrets);
    std::fs::create_dir_all(&secrets)
        .map_err(|e| failed(&format!("cannot make the folder: {e}")))?;
    let written = (|| -> std::io::Result<()> {
        std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o755))?;
        if let Some(text) = &secret {
            let file = secrets.join("adminpass");
            std::fs::write(&file, text)?;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))?;
        }
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&secrets);
        return Err(failed(&format!("cannot write the secret: {}", e.kind())));
    }
    let limit = match command {
        WpCommand::Overview
        | WpCommand::Maintenance { .. }
        | WpCommand::ResetAdminPassword { .. } => Duration::from_secs(120),
        _ => Duration::from_secs(600),
    };
    let outcome = run(
        &context,
        &username,
        &secrets,
        &short,
        image,
        run_as,
        &path,
        &manage_script(&command),
        env,
        limit,
    )
    .await;
    let _ = std::fs::remove_dir_all(&secrets);
    let _ = context
        .docker
        .remove_volume(&db::name(&format!("bp-wpi-{short}"))?)
        .await;
    let done = outcome?;
    let scrub = |text: &str| match &secret {
        Some(s) => text.replace(s.as_str(), "***"),
        None => text.to_owned(),
    };
    if done.exit_code != 0 {
        let tail: String = scrub(&done.output)
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ")
            .chars()
            .filter(|c| !c.is_control())
            .take(400)
            .collect();
        let code = if done.exit_code == 4 {
            ErrorCode::InvalidArguments
        } else {
            ErrorCode::Internal
        };
        return Err(AgentError::new(
            code,
            format!("WP-CLI ended with {}: {tail}", done.exit_code),
        ));
    }
    if matches!(command, WpCommand::Scan) {
        return parse_scan(&done.output)
            .map(Output::WordPressScan)
            .map_err(|e| failed(&format!("the scan is unreadable: {e}")));
    }
    if matches!(command, WpCommand::Overview) {
        return parse_overview(&done.output)
            .map(Output::WordPressOverview)
            .map_err(|e| failed(&format!("the overview is unreadable: {e}")));
    }
    let tail: String = scrub(&done.output)
        .chars()
        .rev()
        .take(4000)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    Ok(Output::WpCliDone { output: tail })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_overview_is_read() {
        let text = "BP_CORE=6.4.3\nBP_UPDATE=6.8.1\nBP_MAINT=Maintenance mode is not active.\nBP_PLUGINS\n[{\"name\":\"akismet\",\"status\":\"active\",\"version\":\"5.0\",\"update\":\"available\",\"update_version\":\"5.3\"}]\nBP_THEMES\n[{\"name\":\"twentytwentyfour\",\"status\":\"inactive\",\"version\":\"1.1\",\"update\":\"none\",\"update_version\":\"\"}]\n";
        let o = parse_overview(text).unwrap();
        assert_eq!(
            (o.core.as_str(), o.core_update.as_deref(), o.maintenance),
            ("6.4.3", Some("6.8.1"), false)
        );
        assert!(o.plugins[0].update_available && o.plugins[0].update_version == "5.3");
        assert!(!o.themes[0].update_available);
        assert!(parse_overview("nothing").is_err());
        let on = parse_overview("BP_CORE=6.8\nBP_UPDATE=\nBP_MAINT=Maintenance mode is active.\nBP_PLUGINS\n[]\nBP_THEMES\n[]\n").unwrap();
        assert!(on.maintenance && on.core_update.is_none());
    }

    #[test]
    fn the_scan_is_read() {
        let text = "BP_SUM_START\nWarning: File doesn't verify against checksum: wp-includes/version.php\nWarning: File should not exist: shell.php\nError: WordPress installation doesn't verify against checksums.\nBP_SUM_END\nBP_DEBUG=true\nBP_FILEEDIT=\nBP_XMLRPC=on\nBP_MODE=644\nBP_WRITABLE=3\n";
        let s = parse_scan(text).unwrap();
        assert!(!s.checksums_ok && s.debug && s.file_edit_allowed && !s.xmlrpc_disabled);
        assert_eq!(
            (s.modified.as_slice(), s.extra.as_slice()),
            (
                ["wp-includes/version.php".to_owned()].as_slice(),
                ["shell.php".to_owned()].as_slice()
            )
        );
        assert_eq!((s.config_mode.as_str(), s.world_writable), ("644", 3));
        let clean = parse_scan("BP_SUM_START\nSuccess: WordPress installation verifies against checksums.\nBP_SUM_END\nBP_DEBUG=\nBP_FILEEDIT=1\nBP_XMLRPC=off\nBP_MODE=640\nBP_WRITABLE=0\n").unwrap();
        assert!(
            clean.checksums_ok && !clean.debug && !clean.file_edit_allowed && clean.xmlrpc_disabled
        );
        assert!(parse_scan("nothing").is_err());
    }

    #[test]
    fn only_plain_values_pass() {
        assert!(plain_folder("") && plain_folder("blog") && plain_folder("a/b-c_d.e"));
        for bad in ["/x", "../x", "a/../b", "a//b", "a b", "a;b"] {
            assert!(!plain_folder(bad), "{bad}");
        }
        assert!(plain_url("https://example.com/blog"));
        for bad in [
            "ftp://x",
            "https://a b",
            "https://x/\"y\"",
            "https://x/`y`",
            "javascript:alert(1)",
        ] {
            assert!(!plain_url(bad), "{bad}");
        }
    }
}
