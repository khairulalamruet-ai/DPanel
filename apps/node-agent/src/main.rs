//! `builtpanel-agent`: the Built Panel node agent (typed operations only, never a shell).
//!
//! ```text
//! builtpanel-agent enroll --server https://panel.example.com:8443 \
//!                      --ca-fingerprint sha256:… --token dhn_…   # once, as the agent user
//! builtpanel-agent status                                            # who am I, when does my certificate expire
//! builtpanel-agent run                                               # the service (systemd)
//! ```
//!
//! Exit codes follow sysexits.h: 64 bad usage, 65 the answer or state is
//! unusable, 69 control plane unreachable, 74 file errors, 75 try again later,
//! 77 the token was refused, 78 configuration problem.

use std::{path::PathBuf, process::ExitCode};

use builtpanel_node_agent::{
    AGENT_VERSION,
    enroll::{self, EnrollError, Options},
    identity::{DEFAULT_STATE_DIR, StateDir, StateError},
    ops::{
        self, DEFAULT_MAX_PARALLEL, Executor, Registry,
        cache::{CACHE_FILE, ResultCache},
    },
    stream,
    verify::VerifyError,
};
use clap::{Parser, Subcommand};
use secrecy::SecretString;

const EXIT_USAGE: u8 = 64;
const EXIT_DATAERR: u8 = 65;
const EXIT_UNAVAILABLE: u8 = 69;
const EXIT_IOERR: u8 = 74;
const EXIT_TEMPFAIL: u8 = 75;
const EXIT_NOPERM: u8 = 77;
const EXIT_CONFIG: u8 = 78;

#[derive(Debug, Parser)]
#[command(name = "builtpanel-agent", version, about = "Built Panel node agent")]
struct Cli {
    /// Directory holding this node's key, certificate and identity.
    #[arg(long, global = true, env = "BUILTPANEL_AGENT_STATE_DIR", default_value = DEFAULT_STATE_DIR)]
    state_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Args)]
struct EnrollArgs {
    /// Address of the control plane, as printed by the panel.
    #[arg(long)]
    server: String,
    /// Fingerprint of the control plane's CA, as printed by the panel. The
    /// agent refuses any other CA.
    #[arg(long)]
    ca_fingerprint: String,
    /// The one-time token. Prefer --token-file or the environment on shared
    /// machines: command lines are visible to other users.
    #[arg(long, env = "BUILTPANEL_ENROLL_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Read the token from this file (first line).
    #[arg(long, conflicts_with = "token")]
    token_file: Option<PathBuf>,
    /// Host name to report (detected when omitted).
    #[arg(long)]
    hostname: Option<String>,
    /// Replace an existing identity. The old key and certificate are removed.
    #[arg(long)]
    force: bool,
    /// Trust this extra root certificate (PEM) for the enrolment request.
    /// For private CAs and local testing; public roots are built in.
    #[arg(long)]
    tls_ca_file: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Join the panel with the one-time token shown when the node was added.
    /// Run it as the agent's own user, once.
    Enroll(EnrollArgs),
    /// Show this node's identity and certificate lifetime.
    Status,
    /// Run the agent (what the systemd service does).
    Run {
        /// Where to connect (`https://host:port`), when the control plane did
        /// not say at enrolment.
        #[arg(long, env = "BUILTPANEL_AGENT_STREAM_URL")]
        stream_url: Option<String>,
    },
}

fn fail(code: u8, message: &str) -> ExitCode {
    eprintln!("builtpanel-agent: {message}");
    ExitCode::from(code)
}

fn exit_code(err: &EnrollError) -> u8 {
    match err {
        EnrollError::InvalidServer(_) | EnrollError::TlsCa(_) => EXIT_USAGE,
        EnrollError::Verify(VerifyError::BadPin) => EXIT_USAGE,
        EnrollError::Verify(_) | EnrollError::BadResponse(_) => EXIT_DATAERR,
        EnrollError::Crypto(_) => 70,
        EnrollError::Network(_) => EXIT_UNAVAILABLE,
        EnrollError::Rejected { status: 401, .. } => EXIT_NOPERM,
        EnrollError::Rejected { status: 429, .. } => EXIT_TEMPFAIL,
        EnrollError::Rejected { status, .. } if *status >= 500 => EXIT_UNAVAILABLE,
        EnrollError::Rejected { .. } => EXIT_DATAERR,
        EnrollError::State(s) => state_code(s),
    }
}

fn state_code(err: &StateError) -> u8 {
    match err {
        StateError::AlreadyEnrolled(_) | StateError::Corrupt(_) => EXIT_DATAERR,
        StateError::NotEnrolled(_)
        | StateError::InsecureDirectory { .. }
        | StateError::InsecureKey { .. } => EXIT_CONFIG,
        StateError::Io { .. } => EXIT_IOERR,
    }
}

fn read_token(token: Option<String>, file: Option<PathBuf>) -> Result<SecretString, String> {
    let raw = match (token, file) {
        (Some(t), _) => t,
        (None, Some(path)) => std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read the token file {}: {e}", path.display()))?,
        (None, None) => {
            return Err(
                "give the token with --token, --token-file or BUILTPANEL_ENROLL_TOKEN".to_owned(),
            );
        }
    };
    let token = raw.lines().next().unwrap_or_default().trim().to_owned();
    if token.is_empty() {
        return Err("the token is empty".to_owned());
    }
    Ok(SecretString::from(token))
}

fn enroll_command(state_dir: PathBuf, args: EnrollArgs) -> ExitCode {
    let EnrollArgs {
        server,
        ca_fingerprint,
        token,
        token_file,
        hostname,
        force,
        tls_ca_file,
    } = args;
    let token = match read_token(token, token_file) {
        Ok(t) => t,
        Err(message) => return fail(EXIT_USAGE, &message),
    };
    let tls_ca_pem = match tls_ca_file
        .map(|p| std::fs::read(&p).map_err(|e| format!("cannot read {}: {e}", p.display())))
    {
        Some(Err(message)) => return fail(EXIT_USAGE, &message),
        Some(Ok(pem)) => Some(pem),
        None => None,
    };
    let options = Options {
        server,
        token,
        ca_fingerprint,
        state_dir: state_dir.clone(),
        hostname,
        replace: force,
        tls_ca_pem,
    };
    match enroll::enroll(&options) {
        Ok(enrolled) => {
            let id = &enrolled.identity;
            println!("enrolled as '{}' ({})", id.name, id.node_id);
            println!("control plane: {}", id.server);
            println!("CA:            sha256:{}", id.ca_fingerprint_sha256);
            println!(
                "certificate:   valid until {}",
                format_unix(id.certificate_not_after)
            );
            println!(
                "identity kept in {} (the private key never left this machine)",
                state_dir.display()
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            let code = exit_code(&err);
            if let EnrollError::Rejected {
                retry_after: Some(seconds),
                ..
            } = &err
            {
                eprintln!(
                    "builtpanel-agent: {err}\nbuiltpanel-agent: try again in {seconds} seconds"
                );
                return ExitCode::from(code);
            }
            if matches!(err, EnrollError::Verify(ref v) if *v != VerifyError::BadPin) {
                // The control plane has already answered, so it used up the token and
                // created the node; this agent refused to trust the answer.
                eprintln!(
                    "builtpanel-agent: {err}\nbuiltpanel-agent: nothing was stored. The token is used up and the control plane \
                     may list this node: remove it in the panel, check the CA fingerprint, and add the node again"
                );
                return ExitCode::from(code);
            }
            if matches!(err, EnrollError::Rejected { status: 401, .. }) {
                eprintln!(
                    "builtpanel-agent: {err}\nbuiltpanel-agent: tokens work once and for 15 minutes; add the node again in the panel to get a new one"
                );
                return ExitCode::from(code);
            }
            fail(code, &err.to_string())
        }
    }
}

fn format_unix(seconds: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| seconds.to_string())
}

fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn status_command(state_dir: PathBuf) -> ExitCode {
    match StateDir::new(&state_dir).load() {
        Ok(loaded) => {
            let id = loaded.identity;
            let days_left = (id.certificate_not_after - now_unix()).div_euclid(86_400);
            println!("node:         {} ({})", id.name, id.node_id);
            println!("server:       {}", id.server);
            println!("CA:           sha256:{}", id.ca_fingerprint_sha256);
            println!("enrolled:     {}", id.enrolled_at);
            println!(
                "certificate:  valid until {} ({} days left{})",
                format_unix(id.certificate_not_after),
                days_left.max(0),
                if days_left < 0 { ", EXPIRED" } else { "" }
            );
            println!("agent:        {AGENT_VERSION}");
            if days_left < 0 {
                ExitCode::from(EXIT_DATAERR)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => fail(state_code(&err), &err.to_string()),
    }
}

fn run_command(state_dir: PathBuf, stream_url: Option<String>) -> ExitCode {
    let loaded = match StateDir::new(&state_dir).load() {
        Ok(l) => l,
        Err(err) => {
            return fail(
                state_code(&err),
                &format!("{err}. Enrol first: builtpanel-agent enroll --help"),
            );
        }
    };
    let id = loaded.identity;
    if id.certificate_not_after <= now_unix() {
        return fail(
            EXIT_DATAERR,
            "this node's certificate has expired; enrol again with a new token",
        );
    }
    let Some(address) = stream_url.or(id.stream_url.clone()) else {
        return fail(
            EXIT_CONFIG,
            "the control plane did not say where to connect (its agent port is probably off); \
             give the address with --stream-url https://host:port",
        );
    };
    if !stream::valid_stream_url(&address) {
        return fail(
            EXIT_CONFIG,
            &format!("`{address}` is not a valid stream address: expected https://host[:port]"),
        );
    }
    let read = |path: &std::path::Path| {
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
    };
    let (ca_pem, cert_pem, key_pem) = match (
        read(&loaded.ca_path),
        read(&loaded.cert_path),
        read(&loaded.key_path),
    ) {
        (Ok(ca), Ok(cert), Ok(key)) => (ca, cert, SecretString::from(key)),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return fail(EXIT_IOERR, &e),
    };
    println!(
        "builtpanel-agent {AGENT_VERSION}: node '{}' ({}), control plane {address}",
        id.name, id.node_id
    );
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => return fail(70, &format!("cannot start the runtime: {err}")),
    };
    runtime.block_on(async {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(err) => return fail(70, &format!("cannot listen for signals: {err}")),
            };
        // The web server (Caddy) is driven only when its admin address is given.
        let web = builtpanel_node_agent::web::Settings::from_env()
            .map(|s| builtpanel_node_agent::web::WebContext::new(s, &state_dir));
        if let Some(web) = &web {
            // Sites are reached on the loopback port their container is published on.
            if let Some(context) = tenant_context(&state_dir) {
                web.use_docker(context.docker.clone());
            }
            tokio::spawn(web.clone().keep_in_sync(std::time::Duration::from_secs(20)));
        }
        // phpMyAdmin's temporary accounts are dropped when their time is up.
        if let Some(context) = tenant_context(&state_dir) {
            let context = std::sync::Arc::new(context);
            tokio::spawn(builtpanel_node_agent::ops::pma::sweep_forever(
                context.clone(),
            ));
            // The accounts' cron jobs (T10.02).
            tokio::spawn(builtpanel_node_agent::ops::cron::run_forever(context));
        }
        let executor = open_executor(&state_dir, web);
        // Forget old results once an hour.
        tokio::spawn({
            let executor = executor.clone();
            async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
                loop {
                    tick.tick().await;
                    if let Err(err) = executor.cache().prune(now_unix()) {
                        eprintln!("builtpanel-agent: {err}");
                    }
                }
            }
        });
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let stream = stream::run(
            stream::Params {
                address,
                ca_pem,
                cert_pem,
                key_pem,
            },
            executor,
            stopped,
        );
        tokio::pin!(stream);
        let outcome = tokio::select! {
            r = &mut stream => Some(r),
            _ = terminate.recv() => None,
            _ = tokio::signal::ctrl_c() => None,
        };
        match outcome {
            // The stream only returns early on a configuration problem.
            Some(Err(err)) => fail(EXIT_CONFIG, &err.to_string()),
            Some(Ok(())) => ExitCode::SUCCESS,
            None => {
                println!("stopping");
                let _ = stop.send(true);
                // Let the connection say goodbye (bounded).
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), &mut stream).await;
                ExitCode::SUCCESS
            }
        }
    })
}

/// The tenant operations need Docker (from `DOCKER_HOST`, which the systemd
/// unit points at the agent's rootless daemon) and the privileged helper's
/// socket. Without Docker they are not offered, and the control plane gets
/// "unsupported operation" instead of an error later.
fn tenant_context(state_dir: &std::path::Path) -> Option<ops::tenant::TenantContext> {
    let socket = std::env::var("DOCKER_HOST")
        .ok()
        .and_then(|host| host.strip_prefix("unix://").map(PathBuf::from))?;
    let docker = builtpanel_docker_manager::DockerManager::connect(&socket).ok()?;
    Some(ops::tenant::TenantContext {
        helper_socket: std::env::var_os("BUILTPANEL_AGENT_HELPER_SOCKET")
            .map_or_else(|| "/run/builtpanel/priv-helper.sock".into(), PathBuf::from),
        tenants_root: std::env::var_os("BUILTPANEL_AGENT_TENANTS_ROOT")
            .map_or_else(|| "/srv/tenants".into(), PathBuf::from),
        docker,
        vhosts_root: state_dir.join("vhosts"),
    })
}

/// Opens the operation result cache in the state directory. A cache file that
/// cannot be used is set aside (never deleted) and a fresh one is started: the
/// node stays manageable, at the price of not recognising operations it ran
/// before, which the log line makes visible.
fn open_executor(
    state_dir: &std::path::Path,
    web: Option<std::sync::Arc<builtpanel_node_agent::web::WebContext>>,
) -> std::sync::Arc<Executor> {
    let path = state_dir.join(CACHE_FILE);
    let cache = match ResultCache::open(&path) {
        Ok(cache) => cache,
        Err(first) => {
            let aside = state_dir.join(format!("{CACHE_FILE}.unusable-{}", now_unix()));
            eprintln!(
                "builtpanel-agent: {first}; moving {} to {} and starting an empty cache",
                path.display(),
                aside.display()
            );
            let _ = std::fs::rename(&path, &aside);
            match ResultCache::open(&path).or_else(|second| {
                eprintln!(
                    "builtpanel-agent: {second}; results are kept in memory only until restart"
                );
                ResultCache::in_memory()
            }) {
                Ok(cache) => cache,
                Err(err) => {
                    eprintln!("builtpanel-agent: {err}");
                    std::process::exit(i32::from(EXIT_IOERR));
                }
            }
        }
    };
    Executor::new(
        Registry::builtin_with(state_dir, tenant_context(state_dir)).with_web(web),
        cache,
        DEFAULT_MAX_PARALLEL,
    )
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Enroll(args) => enroll_command(cli.state_dir, args),
        Command::Status => status_command(cli.state_dir),
        Command::Run { stream_url } => run_command(cli.state_dir, stream_url),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use clap::CommandFactory;
    use secrecy::ExposeSecret as _;

    use super::*;

    #[test]
    fn the_command_line_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_command_the_panel_prints_parses() {
        let cli = Cli::try_parse_from([
            "builtpanel-agent",
            "enroll",
            "--server",
            "https://panel.example.com:8443",
            "--ca-fingerprint",
            "sha256:abc",
            "--token",
            "dhn_secret",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Enroll(ref a)
            if a.server == "https://panel.example.com:8443" && a.token.as_deref() == Some("dhn_secret")));
        // Pin and server are required; the token must come from somewhere (checked at run time).
        assert!(
            Cli::try_parse_from([
                "builtpanel-agent",
                "enroll",
                "--server",
                "https://x.example"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "builtpanel-agent",
                "enroll",
                "--ca-fingerprint",
                "sha256:ab"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "builtpanel-agent",
                "enroll",
                "--server",
                "https://x.example",
                "--ca-fingerprint",
                "ab",
                "--token",
                "t",
                "--token-file",
                "f"
            ])
            .is_err()
        );
    }

    #[test]
    fn tokens_are_read_from_the_argument_or_the_first_line_of_a_file() {
        assert_eq!(
            read_token(Some("dhn_a".into()), None)
                .unwrap()
                .expose_secret(),
            "dhn_a"
        );
        let path = std::env::temp_dir().join(format!("agent-token-{}", std::process::id()));
        std::fs::write(&path, "dhn_from_file  \nsecond line ignored\n").unwrap();
        assert_eq!(
            read_token(None, Some(path.clone()))
                .unwrap()
                .expose_secret(),
            "dhn_from_file"
        );
        std::fs::write(&path, "\n").unwrap();
        assert!(read_token(None, Some(path.clone())).is_err(), "empty");
        let _ = std::fs::remove_file(&path);
        assert!(read_token(None, Some(path)).is_err(), "missing file");
        assert!(read_token(None, None).is_err(), "no token at all");
    }

    #[test]
    fn exit_codes_tell_scripts_what_went_wrong() {
        let rejected = |status| EnrollError::Rejected {
            status,
            code: None,
            detail: String::new(),
            retry_after: None,
        };
        assert_eq!(exit_code(&rejected(401)), EXIT_NOPERM);
        assert_eq!(exit_code(&rejected(429)), EXIT_TEMPFAIL);
        assert_eq!(exit_code(&rejected(503)), EXIT_UNAVAILABLE);
        assert_eq!(exit_code(&rejected(422)), EXIT_DATAERR);
        assert_eq!(
            exit_code(&EnrollError::Network("x".into())),
            EXIT_UNAVAILABLE
        );
        assert_eq!(
            exit_code(&EnrollError::InvalidServer("x".into())),
            EXIT_USAGE
        );
        assert_eq!(
            exit_code(&EnrollError::Verify(VerifyError::BadPin)),
            EXIT_USAGE
        );
        assert_eq!(
            exit_code(&EnrollError::Verify(VerifyError::CaMismatch)),
            EXIT_DATAERR
        );
        assert_eq!(
            exit_code(&StateError::AlreadyEnrolled(PathBuf::new()).into()),
            EXIT_DATAERR
        );
    }
}
