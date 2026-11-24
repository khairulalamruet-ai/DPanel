//! `builtpanel-root-auth`: checks the server's root password for the panel's root login.
//!
//! ```text
//! builtpanel-root-auth [--socket PATH] [--client-user NAME] [--user NAME] [--shadow PATH]
//! ```
//!
//! Started by systemd as root (`builtpanel-root-auth.service`). See the library documentation.
//! Exit codes: 64 bad usage, 78 the panel's user does not exist, 74 the socket cannot be set up.

use std::{path::PathBuf, process::ExitCode, sync::atomic::AtomicBool, time::Duration};

use builtpanel_root_auth::{
    Config, DEFAULT_SOCKET, Log as _, StderrLog, bind, lookup_system_user, serve,
};

const USAGE: &str = "usage: builtpanel-root-auth [--socket PATH] [--client-user NAME] [--user NAME] [--shadow PATH]";

fn main() -> ExitCode {
    let log = StderrLog;
    let mut socket = PathBuf::from(DEFAULT_SOCKET);
    let mut client = String::from("builtpanel");
    let mut user = String::from("root");
    let mut shadow = PathBuf::from("/etc/shadow");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let Some(value) = args.next() else {
            log.line(USAGE);
            return ExitCode::from(64);
        };
        match arg.as_str() {
            "--socket" => socket = PathBuf::from(value),
            "--client-user" => client = value,
            "--user" => user = value,
            "--shadow" => shadow = PathBuf::from(value),
            _ => {
                log.line(USAGE);
                return ExitCode::from(64);
            }
        }
    }
    let Some((client_uid, client_gid)) = lookup_system_user(&client) else {
        log.line(&format!("the panel's user '{client}' does not exist"));
        return ExitCode::from(78);
    };
    let config = Config {
        socket,
        shadow,
        user,
        client_uid,
        client_gid,
        wrong_delay: Duration::from_secs(1),
        read_timeout: Duration::from_secs(5),
    };
    let listener = match bind(&config) {
        Ok(listener) => listener,
        Err(err) => {
            log.line(&format!(
                "cannot listen on {}: {err}",
                config.socket.display()
            ));
            return ExitCode::from(74);
        }
    };
    log.line(&format!(
        "checking {}'s password for uid {client_uid} on {}",
        config.user,
        config.socket.display()
    ));
    // Stopped by systemd's SIGTERM, which ends the process; nothing needs flushing.
    serve(&listener, &config, &log, &AtomicBool::new(false));
    ExitCode::SUCCESS
}
