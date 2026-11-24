//! `builtpanel-priv-helper`: the small root helper of the node agent.
//!
//! ```text
//! builtpanel-priv-helper [--socket PATH] [--agent-user NAME | --agent-uid UID --agent-gid GID]
//! ```
//!
//! Started by systemd as root. See the library documentation for what it will
//! and will not do. Exit codes: 64 bad usage, 78 the agent user does not exist,
//! 74 the socket cannot be set up.

use std::{path::PathBuf, process::ExitCode, sync::atomic::AtomicBool};

use builtpanel_priv_helper::{
    server::{Config, DEFAULT_SOCKET, Log as _, StderrLog, bind, lookup_system_user, serve},
    tenant::TenantEnv,
};

const USAGE: &str = "usage: builtpanel-priv-helper [--socket PATH] [--tenants-root DIR] [--db-port PORT] [--pg-port PORT] [--transfers-dir DIR] [--uid-map subuid|identity] [--agent-user NAME | --agent-uid UID --agent-gid GID]";

#[allow(clippy::print_stdout)]
fn main() -> ExitCode {
    let log = StderrLog;
    // The file transfer workers, started by the helper itself as the tenant's user id.
    if let Some(mode @ ("fileput" | "fileget")) = std::env::args().nth(1).as_deref() {
        return transfer_worker(mode);
    }
    if std::env::args().nth(1).as_deref() == Some("filebackup") {
        let root = std::env::args().nth(3).map(PathBuf::from);
        return match root {
            Some(root) => match builtpanel_priv_helper::files::backup_worker(
                &root,
                std::io::stdin(),
                &mut std::io::stdout(),
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(answer) => {
                    eprintln!("{}", serde_json::to_string(&answer).unwrap_or_default());
                    ExitCode::from(3)
                }
            },
            None => ExitCode::from(64),
        };
    }
    if std::env::args().nth(1).as_deref() == Some("filerestore") {
        let args: Vec<String> = std::env::args().collect();
        let root = args.get(3).map(PathBuf::from);
        let folder = (args.get(4).map(String::as_str) == Some("--folder"))
            .then(|| args.get(5).cloned())
            .flatten();
        return match root {
            Some(root) => match builtpanel_priv_helper::files::restore_worker(
                &root,
                std::io::BufReader::new(std::io::stdin()),
                folder.as_deref(),
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(answer) => {
                    eprintln!("{}", serde_json::to_string(&answer).unwrap_or_default());
                    ExitCode::from(3)
                }
            },
            None => ExitCode::from(64),
        };
    }
    // The file worker: started by the helper itself, as the tenant's user id.
    if std::env::args().nth(1).as_deref() == Some("fileop") {
        let root = std::env::args().nth(3).map(PathBuf::from);
        return match (std::env::args().nth(2).as_deref(), root) {
            (Some("--root"), Some(root)) => {
                match builtpanel_priv_helper::files::worker(
                    &root,
                    std::io::stdin(),
                    std::io::stdout(),
                ) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(_) => ExitCode::FAILURE,
                }
            }
            _ => ExitCode::from(64),
        };
    }
    let mut socket = PathBuf::from(DEFAULT_SOCKET);
    let mut user = String::from("builtpanel-agent");
    let mut tenants_root = PathBuf::from("/srv/tenants");
    let mut db_port: u16 = 13306;
    let mut pg_port: u16 = 15432;
    let mut transfers = PathBuf::from("/var/lib/builtpanel-transfers");
    let mut server = builtpanel_priv_helper::servertools::Env::default();
    let mut ssh_port: u16 = 22;
    // How the containers' ids reach the host: through the agent's subordinate range (a rootless engine,
    // the default), or as they are (a root engine such as Kiri). The unit can say it with the variable.
    let mut identity = std::env::var("BUILTPANEL_HELPER_UID_MAP").is_ok_and(|v| v == "identity");
    let (mut uid, mut gid) = (None::<u32>, None::<u32>);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next();
        let parsed = match arg.as_str() {
            "--socket" => value().map(|v| socket = v.into()),
            "--agent-user" => value().map(|v| user = v),
            "--tenants-root" => value().map(|v| tenants_root = v.into()),
            "--uid-map" => value()
                .and_then(|v| match v.as_str() {
                    "identity" => Some(true),
                    "subuid" => Some(false),
                    _ => None,
                })
                .map(|v| identity = v),
            "--transfers-dir" => value().map(|v| transfers = v.into()),
            // For tests: where the server tools find their commands and files.
            "--ssh-port" => value()
                .and_then(|v| v.parse().ok())
                .filter(|p: &u16| *p >= 1)
                .map(|p| ssh_port = p),
            "--reboot-command" => value().map(|v| server.reboot_command = v.into()),
            "--swap-file" => value().map(|v| server.swap_file = v.into()),
            "--zoneinfo-dir" => value().map(|v| server.zoneinfo = v.into()),
            "--direct-server-tools" => {
                server.direct = true;
                Some(())
            }
            "--db-port" => value()
                .and_then(|v| v.parse().ok())
                .filter(|p: &u16| *p >= 1024)
                .map(|p| db_port = p),
            "--pg-port" => value()
                .and_then(|v| v.parse().ok())
                .filter(|p: &u16| *p >= 1024)
                .map(|p| pg_port = p),
            "--agent-uid" => value().and_then(|v| v.parse().ok()).map(|v| uid = Some(v)),
            "--agent-gid" => value().and_then(|v| v.parse().ok()).map(|v| gid = Some(v)),
            "--version" => {
                println!("{}", builtpanel_common::build_banner("priv-helper"));
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => None,
        };
        if parsed.is_none() {
            log.line(&format!(
                "bad or incomplete option `{}`; {USAGE}",
                arg.escape_default()
            ));
            return ExitCode::from(64);
        }
    }
    let (agent_uid, agent_gid) = match (uid, gid) {
        (Some(uid), Some(gid)) => (uid, gid),
        (None, None) => match lookup_system_user(&user) {
            Some(ids) => ids,
            None => {
                log.line(&format!("user `{}` does not exist", user.escape_default()));
                return ExitCode::from(78);
            }
        },
        _ => {
            log.line("--agent-uid and --agent-gid go together");
            return ExitCode::from(64);
        }
    };
    let config = Config {
        socket,
        agent_uid,
        agent_gid,
        tenants: TenantEnv {
            root: tenants_root,
            agent_name: user.clone(),
            subuid_file: "/etc/subuid".into(),
            agent_gid,
            mountinfo: "/proc/self/mountinfo".into(),
            identity,
        },
        db_port,
        pg_port,
        transfers,
        server,
        ssh_port,
    };
    let listener = match bind(&config) {
        Ok(listener) => listener,
        Err(err) => {
            log.line(&format!("cannot set up {}: {err}", config.socket.display()));
            return ExitCode::from(74);
        }
    };
    log.line(&format!(
        "listening on {} for uid {agent_uid}",
        config.socket.display()
    ));
    // Runs until systemd stops the process; a stale socket file is replaced at the next start.
    match serve(&listener, &config, &log, &AtomicBool::new(false)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log.line(&format!("stopped: {err}"));
            ExitCode::FAILURE
        }
    }
}

/// `fileput` and `fileget`: `--root DIR` and `--dest`/`--path`, `--max-bytes`, `--overwrite`.
fn transfer_worker(mode: &str) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let (Some(root), Some(max)) = (
        value("--root"),
        value("--max-bytes").and_then(|v| v.parse::<u64>().ok()),
    ) else {
        return ExitCode::from(64);
    };
    let root = PathBuf::from(root);
    if mode == "fileput" {
        let Some(dest) = value("--dest") else {
            return ExitCode::from(64);
        };
        let overwrite = args.iter().any(|a| a == "--overwrite");
        builtpanel_priv_helper::files::put_worker(
            &root,
            &dest,
            overwrite,
            max,
            std::io::stdin(),
            std::io::stdout(),
        );
        return ExitCode::SUCCESS;
    }
    let Some(path) = value("--path") else {
        return ExitCode::from(64);
    };
    match builtpanel_priv_helper::files::get_worker(&root, &path, max, &mut std::io::stdout()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(answer) => {
            eprintln!("{}", serde_json::to_string(&answer).unwrap_or_default());
            ExitCode::from(3)
        }
    }
}
