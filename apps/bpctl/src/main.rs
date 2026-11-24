//! Built Panel admin CLI for bootstrap, migrations and recovery.
//!
//! Reads the same configuration as the control plane (`BUILTPANEL_CONFIG`,
//! `BUILTPANEL_*` env, secret files).

use std::{io::BufRead as _, path::PathBuf, process::ExitCode};

use builtpanel_auth_core::{
    password::{Argon2Params, PasswordHasher},
    policy::PasswordPolicy,
};
use builtpanel_config::ControlConfig;
use builtpanel_database::{
    PoolSettings,
    accounts::{InsertAccountError, NewAccount},
    sqlx::PgPool,
};
use builtpanel_domain::{AccountKind, Email, Username};
use clap::{Parser, Subcommand};
use secrecy::SecretString;

/// `EX_USAGE` from sysexits.h (invalid input).
const EXIT_USAGE: u8 = 64;
/// `EX_DATAERR` from sysexits.h (input conflicts with existing data).
const EXIT_DATAERR: u8 = 65;
/// `EX_UNAVAILABLE` from sysexits.h (database unreachable or migration failed).
const EXIT_UNAVAILABLE: u8 = 69;
/// `EX_SOFTWARE` from sysexits.h (internal error).
const EXIT_SOFTWARE: u8 = 70;
/// `EX_IOERR` from sysexits.h (files could not be read or written).
const EXIT_IOERR: u8 = 74;
/// `EX_CONFIG` from sysexits.h.
const EXIT_CONFIG: u8 = 78;
/// Where the control plane keeps its secrets, including the node CA.
const DEFAULT_CA_DIR: &str = "/etc/builtpanel/secrets";

#[derive(Debug, Parser)]
#[command(name = "bpctl", version, about = "Built Panel admin CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Apply pending database migrations.
    Migrate {
        #[command(subcommand)]
        action: Option<MigrateAction>,
    },
    /// Manage administrator accounts.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    /// Work with the panel's own backups (restoring on a new server).
    SystemBackup {
        #[command(subcommand)]
        action: SystemBackupAction,
    },
    /// Manage the internal certificate authority for node mTLS.
    Ca {
        #[command(subcommand)]
        action: CaAction,
    },
    /// Sign, check and install a release (the signed release manifest).
    Release {
        #[command(subcommand)]
        action: ReleaseAction,
    },
}

#[derive(Debug, Subcommand)]
enum ReleaseAction {
    /// Make a release key (written 0600, never over a file) and print its public half as
    /// the line the trusted-keys file holds.
    Keygen {
        /// The key id (letters, digits, `-` and `_`).
        #[arg(long)]
        kid: String,
        /// Where the private key goes.
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Sign the manifest of the given files (their names are the file names).
    Sign {
        /// The private key file.
        #[arg(long)]
        key: std::path::PathBuf,
        /// The key id.
        #[arg(long)]
        kid: String,
        /// The release version.
        #[arg(long)]
        version: String,
        /// The source commit (hex).
        #[arg(long)]
        commit: String,
        /// When it was made (Unix seconds); now when absent.
        #[arg(long)]
        created_at: Option<i64>,
        /// Where the signed manifest is written.
        #[arg(long)]
        out: std::path::PathBuf,
        /// The release files.
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
    },
    /// Check a signed manifest and every file of it, changing nothing.
    Verify {
        /// The signed manifest.
        #[arg(long)]
        manifest: std::path::PathBuf,
        /// The trusted keys (lines of `kid base64-public-key`).
        #[arg(long)]
        keys: std::path::PathBuf,
        /// The folder with the files.
        #[arg(long)]
        dir: std::path::PathBuf,
    },
    /// Check, then install a release into a folder; the files it replaces are kept as
    /// known-good, and anything that goes wrong leaves the folder as it was.
    Install {
        /// The signed manifest.
        #[arg(long)]
        manifest: std::path::PathBuf,
        /// The trusted keys.
        #[arg(long)]
        keys: std::path::PathBuf,
        /// The folder with the downloaded files.
        #[arg(long)]
        from: std::path::PathBuf,
        /// The folder the files are installed into.
        #[arg(long)]
        to: std::path::PathBuf,
    },
    /// Put the known-good files back over the installed ones.
    Rollback {
        /// The folder the release was installed into.
        #[arg(long)]
        to: std::path::PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum SystemBackupAction {
    /// Decrypt a system backup (`system.tar.zst.enc`, fetched from the destination) into a
    /// new folder: `database.dump` (for `pg_restore`), `config/control.toml`, `secrets/`
    /// and `pki/`. Needs the master key the panel had when the backup was made; it is not
    /// in the backup.
    Open {
        /// The file with the master key (its text, as in `secrets.master_key_file`).
        #[arg(long)]
        master_key_file: PathBuf,
        /// The `system.tar.zst.enc` file.
        #[arg(long)]
        input: PathBuf,
        /// A folder that does not exist yet (made private, mode 0700).
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum CaAction {
    /// Create the CA (key `node_ca.key` mode 0600, certificate `node_ca.crt`).
    /// Refuses to overwrite an existing CA: replacing it would orphan every
    /// enrolled node.
    Init {
        /// Directory for the CA files (created with mode 0700 if missing).
        #[arg(long, default_value = DEFAULT_CA_DIR)]
        dir: PathBuf,
        /// Common name of the CA certificate.
        #[arg(long, default_value = "Built Panel Node CA")]
        name: String,
        /// Validity of the CA certificate in years (1..=20).
        #[arg(long, default_value_t = 10)]
        years: u32,
    },
    /// Show the CA's name, fingerprint and expiry.
    Show {
        /// Directory holding the CA files.
        #[arg(long, default_value = DEFAULT_CA_DIR)]
        dir: PathBuf,
    },
    /// Issue a server certificate for the control plane's agent port.
    IssueServer {
        /// Directory holding the CA files.
        #[arg(long, default_value = DEFAULT_CA_DIR)]
        dir: PathBuf,
        /// DNS name or IP address of the server; repeat for several. The
        /// first becomes the common name.
        #[arg(long = "dns", required = true)]
        names: Vec<String>,
        /// Directory to write `server.crt` and `server.key` to.
        #[arg(long)]
        out_dir: PathBuf,
        /// Validity in days (1..=400).
        #[arg(long, default_value_t = 365)]
        days: u32,
    },
}

#[derive(Debug, Subcommand)]
enum MigrateAction {
    /// List migrations and whether each is applied.
    Info,
}

#[derive(Debug, Subcommand)]
enum AdminAction {
    /// Create an administrator account. Prompts for the password (not echoed).
    Create {
        /// Login name: 3-32 characters, a-z, 0-9, '_' or '-', starting with a letter.
        #[arg(long)]
        username: String,
        /// Contact email address.
        #[arg(long)]
        email: String,
        /// Read the password from the first line of stdin instead of prompting
        /// (for automation). Passwords are never accepted as arguments.
        #[arg(long)]
        password_stdin: bool,
        /// Role to grant. The default gives full access; use `admin` or
        /// `support` for restricted administrators.
        #[arg(long, default_value = "super_admin")]
        role: String,
        /// Make two-factor authentication mandatory for this account: until it
        /// enrols a second factor, it can only use its own security pages.
        #[arg(long)]
        require_mfa: bool,
    },
    /// Let the administrator `root` log in with this server's root password (like WHM).
    /// Creates that administrator (full access) or turns it back on; the password is checked
    /// by the builtpanel-root-auth helper (auth.root_login_socket in the configuration).
    RootLogin {
        /// Contact email of the `root` administrator (used only when it is created).
        #[arg(long, required_unless_present = "disable")]
        email: Option<String>,
        /// Turn the root login off (the `root` administrator is disabled, not deleted).
        #[arg(long, conflicts_with = "email")]
        disable: bool,
    },
}

/// A failure with its exit code. Messages never contain secrets.
#[derive(Debug)]
struct Failure(u8, String);

impl Failure {
    fn exit(self) -> ExitCode {
        eprintln!("bpctl: {}", self.1);
        ExitCode::from(self.0)
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Migrate { action } => migrate(action).await,
        Command::Admin {
            action:
                AdminAction::Create {
                    username,
                    email,
                    password_stdin,
                    role,
                    require_mfa,
                },
        } => admin_create(&username, &email, password_stdin, &role, require_mfa).await,
        Command::Admin {
            action: AdminAction::RootLogin { email, disable },
        } => admin_root_login(email.as_deref(), disable).await,
        Command::Ca { action } => ca(action),
        Command::SystemBackup { action } => system_backup(&action),
        Command::Release { action } => release(action),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) => f.exit(),
    }
}

fn release_failure(e: &impl std::fmt::Display) -> Failure {
    Failure(EXIT_DATAERR, e.to_string())
}

#[allow(clippy::print_stdout)]
fn release(action: ReleaseAction) -> Result<(), Failure> {
    use builtpanel_license::{
        FileSigningKey, PublicKeys, ReleaseManifest, SignedManifest, SigningKey as _, update,
    };
    let read = |path: &std::path::Path| {
        std::fs::read(path)
            .map_err(|e| Failure(EXIT_IOERR, format!("{}: {}", path.display(), e.kind())))
    };
    let trusted = |path: &std::path::Path| -> Result<PublicKeys, Failure> {
        let text = String::from_utf8(read(path)?)
            .map_err(|_| Failure(EXIT_DATAERR, "the keys file is not text".to_owned()))?;
        PublicKeys::parse(&text).map_err(|e| release_failure(&e))
    };
    let signed_of = |path: &std::path::Path| -> Result<SignedManifest, Failure> {
        serde_json::from_slice(&read(path)?)
            .map_err(|_| Failure(EXIT_DATAERR, "the manifest is not readable".to_owned()))
    };
    match action {
        ReleaseAction::Keygen { kid, out } => {
            let key =
                FileSigningKey::generate_to_file(&out, &kid).map_err(|e| release_failure(&e))?;
            let mut keys = PublicKeys::new();
            keys = keys
                .with(&kid, key.public_key())
                .map_err(|e| release_failure(&e))?;
            print!("{}", keys.to_text());
            Ok(())
        }
        ReleaseAction::Sign {
            key,
            kid,
            version,
            commit,
            created_at,
            out,
            files,
        } => {
            let key = FileSigningKey::load(&key, &kid).map_err(|e| release_failure(&e))?;
            let mut contents = Vec::new();
            for path in &files {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| Failure(EXIT_USAGE, "a file has no usable name".to_owned()))?;
                contents.push((name.to_owned(), read(path)?));
            }
            let now = i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()),
            )
            .unwrap_or(0);
            let manifest =
                ReleaseManifest::build(&version, &commit, created_at.unwrap_or(now), &contents)
                    .map_err(|e| release_failure(&e))?;
            let signed = SignedManifest::sign(&manifest, &key).map_err(|e| release_failure(&e))?;
            let json = serde_json::to_vec_pretty(&signed)
                .map_err(|e| Failure(EXIT_SOFTWARE, e.to_string()))?;
            std::fs::write(&out, json)
                .map_err(|e| Failure(EXIT_IOERR, format!("{}: {}", out.display(), e.kind())))?;
            println!(
                "signed {} file(s) of release {version} with {kid}",
                manifest.components.len()
            );
            Ok(())
        }
        ReleaseAction::Verify {
            manifest,
            keys,
            dir,
        } => {
            let (m, _) = update::check_download(&signed_of(&manifest)?, &trusted(&keys)?, &dir)
                .map_err(|e| release_failure(&e))?;
            println!(
                "release {} ({}) is good: {} file(s) match",
                m.version,
                m.commit,
                m.components.len()
            );
            Ok(())
        }
        ReleaseAction::Install {
            manifest,
            keys,
            from,
            to,
        } => {
            let m = update::install(&signed_of(&manifest)?, &trusted(&keys)?, &from, &to)
                .map_err(|e| release_failure(&e))?;
            println!("installed release {} into {}", m.version, to.display());
            Ok(())
        }
        ReleaseAction::Rollback { to } => {
            let names = update::rollback(&to).map_err(|e| release_failure(&e))?;
            println!("put back {} file(s): {}", names.len(), names.join(", "));
            Ok(())
        }
    }
}

#[allow(clippy::print_stdout)]
fn system_backup(action: &SystemBackupAction) -> Result<(), Failure> {
    let SystemBackupAction::Open {
        master_key_file,
        input,
        output,
    } = action;
    if output.exists() {
        return Err(Failure(
            EXIT_DATAERR,
            "the output folder exists already: give a new one".into(),
        ));
    }
    let text = std::fs::read_to_string(master_key_file).map_err(|e| {
        Failure(
            EXIT_IOERR,
            format!("cannot read the master key file: {}", e.kind()),
        )
    })?;
    let key = builtpanel_system_backup::derive_key(&text);
    let file = std::fs::File::open(input)
        .map_err(|e| Failure(EXIT_IOERR, format!("cannot read the backup: {}", e.kind())))?;
    let names = builtpanel_system_backup::unpack(&key, std::io::BufReader::new(file), output)
        .map_err(|e| Failure(EXIT_DATAERR, e.to_string()))?;
    for name in names {
        println!("{}", output.join(name).display());
    }
    Ok(())
}

fn pki_failure(err: builtpanel_pki::PkiError) -> Failure {
    use builtpanel_pki::PkiError as E;
    let code = match &err {
        E::AlreadyInitialised(_) => EXIT_DATAERR,
        E::InvalidInput(_) => EXIT_USAGE,
        E::InvalidCa(_) | E::InsecureKeyPermissions { .. } => EXIT_CONFIG,
        E::Io { .. } => EXIT_IOERR,
        _ => EXIT_SOFTWARE,
    };
    Failure(code, err.to_string())
}

fn print_ca(ca: &builtpanel_pki::CertificateAuthority) {
    println!("name:        {}", ca.common_name());
    println!("fingerprint: sha256:{}", ca.fingerprint_sha256());
    println!(
        "valid:       {} to {}",
        builtpanel_pki::format_rfc3339(ca.not_before()),
        builtpanel_pki::format_rfc3339(ca.not_after())
    );
}

/// `bpctl ca ...`: no database needed, only the CA files.
fn ca(action: CaAction) -> Result<(), Failure> {
    let now = time::OffsetDateTime::now_utc();
    match action {
        CaAction::Init { dir, name, years } => {
            if !(1..=20).contains(&years) {
                return Err(Failure(EXIT_USAGE, "--years must be 1..=20".to_owned()));
            }
            let ca = builtpanel_pki::init_ca_dir(
                &dir,
                &name,
                now,
                time::Duration::days(i64::from(years) * 365),
            )
            .map_err(pki_failure)?;
            println!("created the node CA in {}", dir.display());
            println!(
                "certificate: {}",
                dir.join(builtpanel_pki::CA_CERT_FILE).display()
            );
            println!(
                "private key: {} (mode 0600)",
                dir.join(builtpanel_pki::CA_KEY_FILE).display()
            );
            print_ca(&ca);
            println!(
                "Back the private key up somewhere safe and separate: it cannot be recovered, and losing it means re-enrolling every node."
            );
            Ok(())
        }
        CaAction::Show { dir } => {
            print_ca(&builtpanel_pki::load_ca_dir(&dir, now).map_err(pki_failure)?);
            Ok(())
        }
        CaAction::IssueServer {
            dir,
            names,
            out_dir,
            days,
        } => {
            if !(1..=400).contains(&days) {
                return Err(Failure(EXIT_USAGE, "--days must be 1..=400".to_owned()));
            }
            let ca = builtpanel_pki::load_ca_dir(&dir, now).map_err(pki_failure)?;
            let (issued, key) = ca
                .issue_server_certificate(&names, now, time::Duration::days(i64::from(days)))
                .map_err(pki_failure)?;
            if !out_dir.exists() {
                std::fs::create_dir_all(&out_dir)
                    .map_err(|e| Failure(EXIT_IOERR, format!("{}: {e}", out_dir.display())))?;
            }
            let (cert_path, key_path) = (out_dir.join("server.crt"), out_dir.join("server.key"));
            builtpanel_pki::write_private_file(
                &key_path,
                secrecy::ExposeSecret::expose_secret(&key),
            )
            .map_err(pki_failure)?;
            builtpanel_pki::write_public_file(&cert_path, &issued.cert_pem).map_err(pki_failure)?;
            println!("certificate: {}", cert_path.display());
            println!("private key: {} (mode 0600)", key_path.display());
            println!("subject:     {}", issued.subject_cn);
            println!("fingerprint: sha256:{}", issued.fingerprint_sha256);
            println!(
                "expires:     {}",
                builtpanel_pki::format_rfc3339(issued.not_after)
            );
            Ok(())
        }
    }
}

fn load_config() -> Result<ControlConfig, Failure> {
    ControlConfig::load().map_err(|e| Failure(EXIT_CONFIG, e.to_string()))
}

async fn connect(config: &ControlConfig) -> Result<PgPool, Failure> {
    let url = config
        .database_url()
        .map_err(|e| Failure(EXIT_CONFIG, e.to_string()))?;
    let settings = PoolSettings {
        max_connections: 1,
        application_name: "bpctl".to_owned(),
    };
    builtpanel_database::connect(&url, &settings)
        .await
        .map_err(|e| Failure(EXIT_UNAVAILABLE, format!("cannot connect to database: {e}")))
}

async fn migrate(action: Option<MigrateAction>) -> Result<(), Failure> {
    let config = load_config()?;
    let pool = connect(&config).await?;

    if action.is_none() {
        builtpanel_database::migrate(&pool)
            .await
            .map_err(|e| Failure(EXIT_UNAVAILABLE, format!("migration failed: {e}")))?;
        println!("migrations applied");
    }

    let rows = builtpanel_database::migration_status(&pool)
        .await
        .map_err(|e| {
            Failure(
                EXIT_UNAVAILABLE,
                format!("cannot read migration status: {e}"),
            )
        })?;
    for m in rows {
        let mark = if m.applied { "applied" } else { "pending" };
        println!("{:>16}  {:<8} {}", m.version, mark, m.description);
    }
    Ok(())
}

/// Reads the password either from stdin (one line) or from the terminal twice.
fn read_password(from_stdin: bool) -> Result<SecretString, Failure> {
    if from_stdin {
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| Failure(EXIT_USAGE, format!("cannot read password from stdin: {e}")))?;
        let trimmed = line.trim_end_matches(['\r', '\n']).to_owned();
        return Ok(SecretString::from(trimmed));
    }
    let first = rpassword::prompt_password("Password: ")
        .map_err(|e| Failure(EXIT_USAGE, format!("cannot read password: {e}")))?;
    let second = rpassword::prompt_password("Repeat password: ")
        .map_err(|e| Failure(EXIT_USAGE, format!("cannot read password: {e}")))?;
    if first != second {
        return Err(Failure(EXIT_USAGE, "passwords do not match".to_owned()));
    }
    Ok(SecretString::from(first))
}

async fn admin_create(
    username: &str,
    email: &str,
    password_stdin: bool,
    role: &str,
    require_mfa: bool,
) -> Result<(), Failure> {
    // Validate identifiers before asking for the password.
    let username =
        Username::parse(username).map_err(|e| Failure(EXIT_USAGE, format!("--username: {e}")))?;
    let email = Email::parse(email).map_err(|e| Failure(EXIT_USAGE, format!("--email: {e}")))?;

    let config = load_config()?;
    let password = read_password(password_stdin)?;
    if let Err(violations) =
        PasswordPolicy::default().check(&password, username.as_str(), email.as_str())
    {
        let reasons: Vec<String> = violations.iter().map(|v| v.message()).collect();
        return Err(Failure(
            EXIT_USAGE,
            format!("password rejected: {}", reasons.join(" ")),
        ));
    }

    let hasher = PasswordHasher::new(Argon2Params {
        memory_kib: config.auth.argon2_memory_kib,
        iterations: config.auth.argon2_iterations,
        parallelism: config.auth.argon2_parallelism,
    })
    .map_err(|e| Failure(EXIT_CONFIG, e.to_string()))?;
    let hash = hasher
        .hash_async(password)
        .await
        .map_err(|e| Failure(EXIT_SOFTWARE, e.to_string()))?;

    let pool = connect(&config).await?;
    let db_err = |e: builtpanel_database::sqlx::Error| {
        Failure(EXIT_UNAVAILABLE, format!("database error: {e}"))
    };
    // Checked before the account is created so a typo cannot leave an
    // administrator without any role.
    if !builtpanel_database::rbac::role_exists(&pool, role)
        .await
        .map_err(db_err)?
    {
        return Err(Failure(
            EXIT_USAGE,
            format!("--role: no role named '{role}'"),
        ));
    }
    let existing_admins = builtpanel_database::accounts::count_accounts(&pool, AccountKind::Admin)
        .await
        .map_err(|e| Failure(EXIT_UNAVAILABLE, format!("database error: {e}")))?;

    let id = builtpanel_database::accounts::insert_account(
        &pool,
        &NewAccount {
            kind: AccountKind::Admin,
            username: &username,
            email: &email,
            password_hash: &hash,
        },
    )
    .await
    .map_err(|e| match e {
        InsertAccountError::UsernameTaken | InsertAccountError::EmailTaken => {
            Failure(EXIT_DATAERR, e.to_string())
        }
        InsertAccountError::Database(db) => {
            Failure(EXIT_UNAVAILABLE, format!("database error: {db}"))
        }
    })?;

    let first = if existing_admins == 0 {
        " (first administrator)"
    } else {
        ""
    };
    match builtpanel_database::rbac::assign_role(&pool, id, role, None)
        .await
        .map_err(db_err)?
    {
        builtpanel_database::rbac::AssignRole::UnknownRole => {
            return Err(Failure(
                EXIT_SOFTWARE,
                format!(
                    "account {id} was created but role '{role}' vanished; grant a role before use"
                ),
            ));
        }
        builtpanel_database::rbac::AssignRole::Assigned
        | builtpanel_database::rbac::AssignRole::AlreadyAssigned => {}
    }
    if require_mfa {
        let mut conn = pool.acquire().await.map_err(db_err)?;
        builtpanel_database::admins::update_admin(&mut conn, id, None, None, Some(true))
            .await
            .map_err(|e| Failure(EXIT_UNAVAILABLE, format!("database error: {e}")))?;
    }
    let mfa = if require_mfa { " (2FA required)" } else { "" };
    println!("created admin '{username}' <{email}> id={id} role={role}{first}{mfa}");
    Ok(())
}

async fn admin_root_login(email: Option<&str>, disable: bool) -> Result<(), Failure> {
    let email = email
        .map(|e| Email::parse(e).map_err(|e| Failure(EXIT_USAGE, format!("--email: {e}"))))
        .transpose()?;
    let config = load_config()?;
    let pool = connect(&config).await?;
    let db_err = |e: builtpanel_database::sqlx::Error| {
        Failure(EXIT_UNAVAILABLE, format!("database error: {e}"))
    };
    if disable {
        let was = builtpanel_database::accounts::disable_system_root(&pool)
            .await
            .map_err(db_err)?;
        println!(
            "{}",
            if was {
                "root login turned off (the root administrator is disabled)"
            } else {
                "root login was not set up"
            }
        );
        return Ok(());
    }
    let Some(email) = email else {
        return Err(Failure(EXIT_USAGE, "give --email or --disable".to_owned()));
    };
    let (id, created) = builtpanel_database::accounts::enable_system_root(&pool, &email)
        .await
        .map_err(|e| match e {
            InsertAccountError::EmailTaken => Failure(
                EXIT_DATAERR,
                format!("--email: {email} is already used by another account"),
            ),
            InsertAccountError::UsernameTaken => Failure(
                EXIT_DATAERR,
                "an account named root exists and is not an administrator".to_owned(),
            ),
            InsertAccountError::Database(db) => {
                Failure(EXIT_UNAVAILABLE, format!("database error: {db}"))
            }
        })?;
    match builtpanel_database::rbac::assign_role(&pool, id, "super_admin", None)
        .await
        .map_err(db_err)?
    {
        builtpanel_database::rbac::AssignRole::UnknownRole => {
            return Err(Failure(
                EXIT_SOFTWARE,
                "the built-in role super_admin is missing; run `bpctl migrate`".to_owned(),
            ));
        }
        builtpanel_database::rbac::AssignRole::Assigned
        | builtpanel_database::rbac::AssignRole::AlreadyAssigned => {}
    }
    if config.auth.root_login_socket.is_none() {
        eprintln!(
            "bpctl: warning: auth.root_login_socket is not set in the configuration, so root cannot log in yet"
        );
    }
    println!(
        "root login {} (id={id}): log in as `root` with this server's root password",
        if created { "set up" } else { "turned on" }
    );
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("bpctl-{tag}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn ca_init_issue_and_show_work_end_to_end_and_never_overwrite() {
        let base = temp_dir("ca");
        let (dir, out) = (base.join("secrets"), base.join("tls"));
        let init = |years| {
            ca(CaAction::Init {
                dir: dir.clone(),
                name: "Test CA".into(),
                years,
            })
        };
        // Bad input is a usage error and creates nothing.
        for years in [0, 21] {
            assert_eq!(init(years).unwrap_err().0, EXIT_USAGE, "{years}");
        }
        assert!(!dir.exists());
        init(10).unwrap();
        assert!(dir.join(builtpanel_pki::CA_KEY_FILE).exists());
        // A second init is a data conflict and leaves the key alone.
        let key_before = std::fs::read(dir.join(builtpanel_pki::CA_KEY_FILE)).unwrap();
        assert_eq!(init(10).unwrap_err().0, EXIT_DATAERR);
        assert_eq!(
            std::fs::read(dir.join(builtpanel_pki::CA_KEY_FILE)).unwrap(),
            key_before
        );

        ca(CaAction::Show { dir: dir.clone() }).unwrap();
        let issue = |days, names: Vec<String>| {
            ca(CaAction::IssueServer {
                dir: dir.clone(),
                names,
                out_dir: out.clone(),
                days,
            })
        };
        assert_eq!(
            issue(0, vec!["a.example.com".into()]).unwrap_err().0,
            EXIT_USAGE
        );
        assert_eq!(
            issue(30, vec!["*.example.com".into()]).unwrap_err().0,
            EXIT_USAGE
        );
        assert!(
            !out.join("server.key").exists(),
            "nothing written on bad input"
        );
        issue(30, vec!["control.example.com".into(), "10.0.0.5".into()]).unwrap();
        assert!(out.join("server.crt").exists() && out.join("server.key").exists());
        // Never replaces existing server files either.
        assert_eq!(
            issue(30, vec!["control.example.com".into()]).unwrap_err().0,
            EXIT_IOERR
        );
        // No CA, no certificates.
        let missing = ca(CaAction::Show {
            dir: base.join("nowhere"),
        })
        .unwrap_err();
        assert_eq!(missing.0, EXIT_IOERR);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn password_is_never_a_cli_argument() {
        let cmd = Cli::command();
        let admin = cmd.find_subcommand("admin").expect("admin");
        let create = admin.find_subcommand("create").expect("create");
        assert!(
            create
                .get_arguments()
                .all(|a| a.get_id().as_str() != "password"),
            "a --password flag would expose secrets via ps/shell history"
        );
    }

    #[tokio::test]
    async fn invalid_username_is_rejected_before_any_io() {
        let Err(f) = admin_create("No Spaces", "a@b.co", true, "super_admin", false).await else {
            unreachable!("must fail");
        };
        assert_eq!(f.0, EXIT_USAGE);
        assert!(f.1.contains("--username"));
    }
}
