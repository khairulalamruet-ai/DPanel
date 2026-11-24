//! The catalog's applications (T12.05): a definition (data) becomes an install.
//!
//! The same one-off container as the WordPress installer (tenant's user id, tenant's network,
//! the tenant's files mounted, nothing else) runs a script that is built here from the
//! definition's structure only. Every value a definition's texts take is expanded on the node
//! into small files that are mounted read-only for the run: the script reads them, it never
//! has them in its text, and a placeholder can only ever be one of the fixed ones.

use std::{os::unix::fs::PermissionsExt as _, path::Path, sync::Arc, time::Duration};

use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{
    AgentError, AppDefinition, AppStep, ErrorCode, Operation, Output, WpDatabase,
};
use builtpanel_tenant_stack::{Allowlist, Role};

use super::{
    db,
    scan::state,
    sqlimport::failed,
    tenant::{TenantContext, docker_error},
    wpinstall::{plain, plain_folder, plain_url, run},
};

/// The Composer the `composer` step uses: pinned, and checked by its checksum.
const COMPOSER_URL: &str = "https://getcomposer.org/download/2.11.2/composer.phar";
const COMPOSER_SHA256: &str = "dd158c0afd41d15fac89e1b4e47b5d42911fe316c3687cd5edac98319b25d028";
const LIMIT: Duration = Duration::from_secs(20 * 60);

fn invalid(what: &str) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, what.to_owned())
}

fn base64(bytes: &[u8]) -> String {
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

/// `n` random bytes from the system.
fn random_bytes(n: usize) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let mut bytes = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("no randomness: {}", e.kind()))?;
    Ok(bytes)
}

/// The values the placeholders take.
pub struct Values {
    pairs: Vec<(&'static str, String)>,
}

impl Values {
    fn get(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Replaces the placeholders of `text` (`{{random}}` and `{{app_key}}` are new each time).
///
/// # Errors
/// A placeholder that has no value.
pub fn expand(text: &str, values: &Values) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let name = after[..end].trim();
        match name {
            "random" => out.push_str(
                &random_bytes(16)?
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
            ),
            "app_key" => {
                out.push_str("base64:");
                out.push_str(&base64(&random_bytes(32)?));
            }
            other => out.push_str(
                values
                    .get(other)
                    .ok_or_else(|| format!("no value for {{{{{other}}}}}"))?,
            ),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The script for a definition: its structure only, with file names where values go.
#[must_use]
pub fn script_for(definition: &AppDefinition) -> String {
    let mut s = String::from(
        "set -eu\nT=\"$WP_PATH\"\nmkdir -p \"$T\"\n\
         other=$(ls -A \"$T\" | grep -v -x -e index.html -e index.php -e .well-known -e cgi-bin || true)\n\
         if [ -n \"$other\" ]; then echo \"That folder is not empty\" >&2; exit 3; fi\n\
         rm -f \"$T/index.html\" \"$T/index.php\"\n",
    );
    if let Some(source) = &definition.source {
        s.push_str("curl -fsSL --max-time 900 -o /tmp/src \"$SRC_URL\"\n");
        s.push_str("echo \"$SRC_SHA  /tmp/src\" | sha256sum -c - > /dev/null\n");
        let extract = |dest: &str| match source.format.as_str() {
            "zip" => format!("unzip -q /tmp/src -d {dest}\n"),
            _ => format!("tar -xzf /tmp/src -C {dest}\n"),
        };
        if source.strip == 0 {
            s.push_str(&extract("\"$T\""));
        } else {
            s.push_str("mkdir -p /tmp/x\n");
            s.push_str(&extract("/tmp/x"));
            s.push_str("cp -a /tmp/x/*/. \"$T/\"\nrm -rf /tmp/x\n");
        }
        s.push_str("rm -f /tmp/src\n");
    }
    s.push_str("cd \"$T\"\n");
    let args_loop = |i: usize| {
        format!("set --\nwhile IFS= read -r l; do set -- \"$@\" \"$l\"; done < /run/dh/s{i}\n")
    };
    let mut composer_ready = false;
    for (i, step) in definition.steps.iter().enumerate() {
        match step {
            AppStep::WriteFile { path, .. } => {
                s.push_str(&format!(
                    "mkdir -p \"$(dirname '{path}')\"\ncat /run/dh/s{i} > '{path}'\n"
                ));
            }
            AppStep::Php { .. } => {
                s.push_str(&args_loop(i));
                s.push_str("php -d memory_limit=512M \"$@\"\n");
            }
            AppStep::Composer { .. } => {
                if !composer_ready {
                    composer_ready = true;
                    s.push_str(
                        "curl -fsSL --max-time 300 -o /tmp/composer.phar \"$COMPOSER_URL\"\n\
                         echo \"$COMPOSER_SHA  /tmp/composer.phar\" | sha256sum -c - > /dev/null\n",
                    );
                }
                s.push_str(&args_loop(i));
                s.push_str("php -d memory_limit=1G /tmp/composer.phar --no-interaction \"$@\"\n");
            }
            AppStep::Remove { path } => s.push_str(&format!("rm -rf -- '{path}'\n")),
        }
    }
    s.push_str("echo BP_DONE\n");
    s
}

/// `install_app`.
///
/// # Errors
/// [`AgentError`]: a definition or value that is not valid (not retryable), a folder that is
/// not empty, Docker or the image failing (retryable), a step ending badly (the last of what
/// it said, without the passwords).
pub async fn install_app(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let uid = match &op {
        Operation::InstallApp { uid, .. } => *uid,
        _ => return Err(super::wrong_operation()),
    };
    install_app_as(context, op, (uid, uid)).await
}

/// [`install_app`] running as the given user (a test passes another one).
///
/// # Errors
/// As [`install_app`].
#[allow(clippy::too_many_lines)]
pub async fn install_app_as(
    context: Arc<TenantContext>,
    op: Operation,
    run_as: (u32, u32),
) -> Result<Output, AgentError> {
    let Operation::InstallApp {
        app_id,
        tenant_id: _,
        username,
        uid: _,
        path,
        definition,
        database,
        database_password,
        site,
        admin_password,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    definition
        .checked()
        .map_err(|e| invalid(&format!("the definition: {e}")))?;
    if definition.engine == "wordpress" {
        return Err(invalid("WordPress has its own installer"));
    }
    let username = TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    if !plain_folder(&path) {
        return Err(invalid("the folder must stay inside the tenant's files"));
    }
    // The values: plain text only, so a placeholder can never carry code into a file.
    let words = |t: &str, max: usize| {
        !t.is_empty()
            && t.chars().count() <= max
            && t.chars()
                .all(|c| c.is_alphanumeric() || " .,_:!&()?-".contains(c))
    };
    if !plain_url(&site.url) || !words(&site.title, 100) {
        return Err(invalid("the site's address or title is not valid"));
    }
    let mut pairs: Vec<(&'static str, String)> = vec![
        ("title", site.title.clone()),
        ("site_url", site.url.clone()),
    ];
    if definition.needs_admin() {
        let Some(password) = &admin_password else {
            return Err(invalid("the administrator's password is missing"));
        };
        if !plain(&site.admin_user, 60, "_.-")
            || site.admin_user.len() < 3
            || !site.admin_email.contains('@')
            || !plain(&site.admin_email, 200, "@._+-")
            || password.0.chars().count() < 12
            || password.0.len() > 128
            || password.0.chars().any(char::is_control)
        {
            return Err(invalid("the administrator is not valid"));
        }
        pairs.push(("admin_user", site.admin_user.clone()));
        pairs.push(("admin_email", site.admin_email.clone()));
        pairs.push(("admin_password", password.0.clone()));
    }
    let mut secrets_to_scrub: Vec<String> = admin_password.iter().map(|p| p.0.clone()).collect();
    if definition.needs_database {
        let (Some(WpDatabase { name, user }), Some(password)) = (&database, &database_password)
        else {
            return Err(invalid("the database or its password is missing"));
        };
        if !plain(name, 64, "_")
            || !plain(user, 64, "_")
            || password.0.is_empty()
            || password.0.len() > 128
            || !password
                .0
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        {
            return Err(invalid(
                "the database, its user or its password is not valid",
            ));
        }
        pairs.push(("db_name", name.clone()));
        pairs.push(("db_user", user.clone()));
        pairs.push(("db_password", password.0.clone()));
        pairs.push(("db_host", "db".to_owned()));
        secrets_to_scrub.push(password.0.clone());
    }
    let values = Values { pairs };
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
    // One small file per step, with the values in.
    let short = format!("{}a", &app_id.simple().to_string()[..11]);
    let secrets = state(&context)?.join("wpcli").join(&short);
    let _ = std::fs::remove_dir_all(&secrets);
    std::fs::create_dir_all(&secrets)
        .map_err(|e| failed(&format!("cannot make the folder: {e}")))?;
    let cleanup = |dir: &Path| {
        let _ = std::fs::remove_dir_all(dir);
    };
    let written = (|| -> Result<(), String> {
        std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        for (i, step) in definition.steps.iter().enumerate() {
            let text = match step {
                AppStep::WriteFile { content, .. } => expand(content, &values)?,
                AppStep::Php { args } | AppStep::Composer { args } => {
                    let mut lines = String::new();
                    for a in args {
                        let line = expand(a, &values)?;
                        if line.contains(['\n', '\r', '\0']) {
                            return Err("an argument holds a line break".into());
                        }
                        lines.push_str(&line);
                        lines.push('\n');
                    }
                    lines
                }
                AppStep::Remove { .. } => continue,
            };
            let file = secrets.join(format!("s{i}"));
            std::fs::write(&file, text).map_err(|e| e.to_string())?;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Err(e) = written {
        cleanup(&secrets);
        return Err(invalid(&format!(
            "the definition could not be filled in: {e}"
        )));
    }
    let mut env: Vec<(String, String)> = vec![("COMPOSER_HOME".into(), "/tmp/composer".into())];
    if run_as.0 == 0 {
        env.push(("COMPOSER_ALLOW_SUPERUSER".into(), "1".into()));
    }
    if let Some(source) = &definition.source {
        env.push(("SRC_URL".into(), source.url.clone()));
        env.push(("SRC_SHA".into(), source.sha256.clone()));
    }
    if definition
        .steps
        .iter()
        .any(|s| matches!(s, AppStep::Composer { .. }))
    {
        env.push(("COMPOSER_URL".into(), COMPOSER_URL.into()));
        env.push(("COMPOSER_SHA".into(), COMPOSER_SHA256.into()));
    }
    let outcome = run(
        &context,
        &username,
        &secrets,
        &short,
        image,
        run_as,
        &path,
        &script_for(&definition),
        env,
        LIMIT,
    )
    .await;
    cleanup(&secrets);
    let _ = context
        .docker
        .remove_volume(&db::name(&format!("bp-wpi-{short}"))?)
        .await;
    let done = outcome?;
    if done.exit_code != 0 {
        let mut text = done.output.clone();
        for secret in &secrets_to_scrub {
            text = text.replace(secret.as_str(), "***");
        }
        // The words of the error, not the frames of the stack trace under them.
        let tail: String = text
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !t.starts_with("vendor/")
                    && !t.chars().next().is_some_and(|c| c.is_ascii_digit())
                    && !t.contains("::")
            })
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
            format!("the install ended with {}: {tail}", done.exit_code),
        ));
    }
    Ok(Output::AppInstalled {
        version: definition.version,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_and_the_script_has_no_values() {
        let values = Values {
            pairs: vec![("title", "My site".into()), ("db_name", "alice_wp1".into())],
        };
        assert_eq!(
            expand("<h1>{{title}}</h1> {{ db_name }}", &values).unwrap(),
            "<h1>My site</h1> alice_wp1"
        );
        assert!(expand("{{nope}}", &values).is_err());
        let a = expand("{{random}}", &values).unwrap();
        assert_eq!(a.len(), 32);
        assert_ne!(a, expand("{{random}}", &values).unwrap());
        assert!(
            expand("{{app_key}}", &values)
                .unwrap()
                .starts_with("base64:")
        );
        let definition = AppDefinition {
            slug: "x1".into(),
            name: "X".into(),
            version: "1".into(),
            description: String::new(),
            category: String::new(),
            needs_database: false,
            engine: String::new(),
            document_root: String::new(),
            source: None,
            steps: vec![
                AppStep::WriteFile {
                    path: "a/b.txt".into(),
                    content: "{{title}}".into(),
                },
                AppStep::Php {
                    args: vec!["artisan".into(), "{{title}}".into()],
                },
            ],
        };
        definition.checked().unwrap();
        let script = script_for(&definition);
        assert!(script.contains("cat /run/dh/s0 > 'a/b.txt'") && script.contains("/run/dh/s1"));
        assert!(!script.contains("My site"));
    }
}
