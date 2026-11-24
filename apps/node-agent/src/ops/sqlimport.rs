//! Running an uploaded SQL dump in a tenant's database (T07.07).
//!
//! The control plane keeps the upload and gives the node a credential to fetch it.
//! The node downloads it (and can pick a broken download up where it stopped),
//! checks its size and SHA-256, and only then runs anything. The dump runs as a
//! temporary database account made for that one import, with rights in that one
//! database only: whatever the dump says (`USE mysql`, `GRANT`, `CREATE DATABASE`,
//! `DROP` of someone else's database) is refused by the server, exactly as it would
//! be for the tenant's own user. The account is removed at the end, whatever the
//! outcome. The file is read as a stream, gzip decompressed on the way if it is
//! compressed, cut into statements ([`super::sqlsplit`]) and run one by one; the
//! first statement the server refuses stops it, and says which.

use std::{
    collections::BTreeMap,
    io::{Read, Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{AgentError, ErrorCode, Operation, Output};
use mysql_async::prelude::Queryable as _;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::{
    db,
    sqlsplit::{SplitError, Splitter},
    tenant::TenantContext,
};

/// Largest statement run (the server's `max_allowed_packet` is set to match).
const MAX_STATEMENT_BYTES: usize = 64 << 20;
/// Largest dump taken, in bytes (the control plane's setting is lower).
const MAX_DUMP_BYTES: u64 = 8 << 30;
/// Longest a whole import may take.
const MAX_DURATION: Duration = Duration::from_secs(6 * 3600);
/// How many finished imports the node remembers.
const REMEMBERED: usize = 200;

/// Where an import stands.
#[derive(Debug, Clone, Default)]
pub(super) struct Progress {
    pub(super) stage: &'static str,
    pub(super) done: u64,
    pub(super) total: u64,
    pub(super) statements: u64,
    pub(super) error: Option<String>,
}

static PROGRESS: Mutex<BTreeMap<Uuid, Progress>> = Mutex::new(BTreeMap::new());

pub(super) fn update(id: Uuid, f: impl FnOnce(&mut Progress)) {
    if let Ok(mut all) = PROGRESS.lock() {
        if all.len() > REMEMBERED && !all.contains_key(&id) {
            // Forget the oldest finished one.
            if let Some(old) = all
                .iter()
                .find(|(_, p)| matches!(p.stage, "done" | "failed"))
                .map(|(k, _)| *k)
            {
                all.remove(&old);
            }
        }
        f(all.entry(id).or_default());
    }
}

/// Where downloads are kept: next to the other state the agent owns.
pub(super) fn imports_dir(context: &TenantContext) -> PathBuf {
    context.vhosts_root.with_file_name("imports")
}

pub(super) fn invalid(m: &str) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, m.to_owned())
}

pub(super) fn failed(what: &str) -> AgentError {
    AgentError::new(ErrorCode::Internal, what.to_owned())
}

/// Reads the bytes it is given and counts them.
pub(super) struct Counting<R> {
    inner: R,
    import_id: Uuid,
    read: u64,
}

impl<R> Counting<R> {
    pub(super) fn new(inner: R, import_id: Uuid) -> Self {
        Self {
            inner,
            import_id,
            read: 0,
        }
    }
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        update(self.import_id, |p| p.done = self.read);
        Ok(n)
    }
}

/// `report_import_progress`.
///
/// # Errors
/// A wrong operation.
pub async fn report_import_progress(
    _context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::ReportImportProgress { import_id } = op else {
        return Err(super::wrong_operation());
    };
    let p = PROGRESS
        .lock()
        .ok()
        .and_then(|all| all.get(&import_id).cloned());
    Ok(match p {
        Some(p) => Output::ImportProgress {
            stage: p.stage.to_owned(),
            bytes_done: p.done,
            bytes_total: p.total,
            statements: p.statements,
            error: p.error,
        },
        None => Output::ImportProgress {
            stage: "unknown".into(),
            bytes_done: 0,
            bytes_total: 0,
            statements: 0,
            error: None,
        },
    })
}

/// Downloads the dump into `path`, picking up a partial file, and returns when the
/// whole `size` is there.
pub(super) fn download(
    url: &str,
    token: &str,
    size: u64,
    path: &Path,
    import_id: Uuid,
) -> Result<(), AgentError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .max_redirects(0)
        .build()
        .into();
    let have = std::fs::metadata(path).map_or(0, |m| m.len()).min(size);
    let mut request = agent
        .get(url)
        .header("Authorization", &format!("Bearer {token}"));
    if have > 0 {
        request = request.header("Range", &format!("bytes={have}-"));
    }
    let mut response = request
        .call()
        .map_err(|_| failed("the dump could not be fetched from the control plane").retryable())?;
    let status = response.status().as_u16();
    let start = match (have, status) {
        (0, 200) | (_, 200) => 0,
        (h, 206) => h,
        (_, 401 | 403 | 404 | 410) => {
            return Err(failed("the control plane refused the download"));
        }
        _ => return Err(failed("the control plane did not give the dump").retryable()),
    };
    use std::os::unix::fs::OpenOptionsExt as _;
    // Readable by the agent's group: the privileged helper reads what is fetched for
    // a tenant's files.
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(start == 0)
        .mode(0o660)
        .open(path)
        .map_err(|e| failed(&format!("cannot keep the dump: {}", e.kind())).retryable())?;
    out.seek(SeekFrom::Start(start))
        .map_err(|e| failed(&format!("cannot keep the dump: {}", e.kind())))?;
    let mut body = response.body_mut().as_reader();
    let mut written = start;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = body
            .read(&mut buf)
            .map_err(|_| failed("the download broke off").retryable())?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > size {
            return Err(failed("the control plane sent more than was announced"));
        }
        out.write_all(&buf[..n])
            .map_err(|e| failed(&format!("cannot keep the dump: {}", e.kind())).retryable())?;
        update(import_id, |p| p.done = written);
    }
    out.sync_all().ok();
    if written != size {
        return Err(failed("the download broke off").retryable());
    }
    Ok(())
}

pub(super) fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// What the server said, clipped and on one line.
pub(super) fn server_words(err: &mysql_async::Error) -> String {
    match err {
        mysql_async::Error::Server(e) => {
            let text: String = e
                .message
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect();
            format!("error {}: {text}", e.code)
        }
        _ => "the connection to the database server failed".to_owned(),
    }
}

/// A random password for the temporary account.
pub(super) fn random_password() -> std::io::Result<String> {
    let mut bytes = [0u8; 24];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    // Letters, digits and a symbol, so it passes the server's own policy if any.
    Ok(format!(
        "A-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

/// `import_sql`.
///
/// # Errors
/// [`AgentError`]: bad arguments, or a dump that does not match what was announced
/// (not retryable); the download breaking off or the server not reachable
/// (retryable); the first statement the server refused (not retryable, with its
/// words).
pub async fn import_sql(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::ImportSql {
        username,
        database,
        import_id,
        url,
        size_bytes,
        sha256,
        gzip,
        token,
        engine,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
    // Arguments first: nothing is touched for a bad request.
    TenantUsername::parse(&username).map_err(|e| invalid(&e.to_string()))?;
    builtpanel_domain::databases::check_identifier(&database)
        .map_err(|e| invalid(&e.to_string()))?;
    if engine == builtpanel_node_protocol::DbEngine::Postgres {
        super::pg::identifier(&database)?;
    }
    let local = url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost")
        || url.starts_with("http://[::1]");
    let plain_url = url.len() <= 400
        && url.is_ascii()
        && !url.chars().any(|c| c.is_control() || c == ' ')
        && (url.starts_with("https://") || local);
    if !plain_url {
        return Err(invalid("the download address must be https"));
    }
    if size_bytes == 0 || size_bytes > MAX_DUMP_BYTES {
        return Err(invalid("the dump's size is not acceptable"));
    }
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(invalid(
            "the dump's SHA-256 is not 64 lower-case hex digits",
        ));
    }
    let token = token
        .ok_or_else(|| invalid("no credential for the download"))?
        .0;

    update(import_id, |p| {
        *p = Progress {
            stage: "downloading",
            total: size_bytes,
            ..Progress::default()
        };
    });
    let result = tokio::time::timeout(
        MAX_DURATION,
        run(
            &context, engine, &database, import_id, &url, &token, size_bytes, &sha256, gzip,
        ),
    )
    .await
    .unwrap_or_else(|_| Err(failed("the import took longer than six hours")));
    match &result {
        Ok(statements) => update(import_id, |p| {
            p.stage = "done";
            p.statements = *statements;
        }),
        Err(e) => update(import_id, |p| {
            p.stage = "failed";
            p.error = Some(e.message.clone());
        }),
    }
    result.map(|statements| Output::SqlImported { statements })
}

#[allow(clippy::too_many_arguments)]
async fn run(
    context: &TenantContext,
    engine: builtpanel_node_protocol::DbEngine,
    database: &str,
    import_id: Uuid,
    url: &str,
    token: &str,
    size: u64,
    sha256: &str,
    gzip: bool,
) -> Result<u64, AgentError> {
    let dir = imports_dir(context);
    std::fs::create_dir_all(&dir).map_err(|e| {
        failed(&format!(
            "cannot prepare the import directory: {}",
            e.kind()
        ))
        .retryable()
    })?;
    let path = dir.join(format!("{}.dump", import_id.simple()));
    let outcome = run_with_file(
        context, engine, database, import_id, url, token, size, sha256, gzip, &path,
    )
    .await;
    // The file is only kept while a retry may need it.
    let keep = matches!(&outcome, Err(e) if e.retryable);
    if !keep {
        let _ = std::fs::remove_file(&path);
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn run_with_file(
    context: &TenantContext,
    engine: builtpanel_node_protocol::DbEngine,
    database: &str,
    import_id: Uuid,
    url: &str,
    token: &str,
    size: u64,
    sha256: &str,
    gzip: bool,
    path: &Path,
) -> Result<u64, AgentError> {
    // 1. The download (blocking code, on its own thread).
    {
        let (url, token, path) = (url.to_owned(), token.to_owned(), path.to_owned());
        tokio::task::spawn_blocking(move || download(&url, &token, size, &path, import_id))
            .await
            .map_err(|_| failed("the download was cancelled").retryable())??;
    }
    // 2. What arrived is what was announced.
    update(import_id, |p| p.stage = "checking");
    let found = {
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || sha256_of(&path))
            .await
            .map_err(|_| failed("the check was cancelled"))?
            .map_err(|e| failed(&format!("cannot read the dump: {}", e.kind())))?
    };
    if found != sha256 {
        let _ = std::fs::remove_file(path);
        return Err(failed(
            "the dump does not match its SHA-256: nothing was run",
        ));
    }
    match engine {
        builtpanel_node_protocol::DbEngine::Mysql => {
            import_file(context, database, import_id, path, gzip).await
        }
        builtpanel_node_protocol::DbEngine::Postgres => {
            super::pgimport::import_file(context, database, import_id, path, gzip).await
        }
    }
}

/// Runs a dump that is in a file (plain or gzip) into `database`, as a temporary account
/// that is locked again afterwards. Shared by the SQL import and the backup restore.
pub(super) async fn import_file(
    context: &TenantContext,
    database: &str,
    import_id: Uuid,
    path: &Path,
    gzip: bool,
) -> Result<u64, AgentError> {
    // 3. The temporary account, with rights in this one database.
    db::ensure_server(context).await?;
    let password =
        random_password().map_err(|e| failed(&format!("cannot make a password: {}", e.kind())))?;
    // One account per database that stays (locked) after the import: the triggers,
    // views and routines of the dump are defined by it, and they must keep working
    // when the import is over. A hyphen cannot be in a name a client chooses, so
    // no client's own user can be this account.
    let account = definer_account(database);
    let quoted_db = format!("`{database}`");
    {
        let mut admin = db::root_connection(context).await?;
        let hash: Option<String> = admin
            .exec_first("SELECT PASSWORD(?)", (&password,))
            .await
            .map_err(|e| failed(&server_words(&e)))?;
        let hash = hash
            .filter(|h| {
                h.len() == 41 && h.starts_with('*') && h[1..].bytes().all(|b| b.is_ascii_hexdigit())
            })
            .ok_or_else(|| failed("the database server gave no password hash"))?;
        admin
            .query_drop(format!(
                "CREATE USER IF NOT EXISTS '{account}'@'%' IDENTIFIED BY PASSWORD '{hash}' WITH MAX_USER_CONNECTIONS 2"
            ))
            .await
            .map_err(|e| failed(&server_words(&e)))?;
        admin
            .query_drop(format!(
                "ALTER USER '{account}'@'%' IDENTIFIED BY PASSWORD '{hash}' ACCOUNT UNLOCK"
            ))
            .await
            .map_err(|e| failed(&server_words(&e)))?;
        let privileges = builtpanel_domain::databases::Access::All
            .privileges()
            .join(", ");
        admin
            .query_drop(format!(
                "GRANT {privileges} ON {quoted_db}.* TO '{account}'@'%'"
            ))
            .await
            .map_err(|e| failed(&server_words(&e)))?;
        let _ = admin.disconnect().await;
    }
    // 4. Run it; whatever happens, the account goes.
    update(import_id, |p| {
        p.stage = "importing";
        p.done = 0;
    });
    let ran = {
        let (path, db_name, account2, password2) = (
            path.to_owned(),
            database.to_owned(),
            account.clone(),
            password.clone(),
        );
        let handle = tokio::runtime::Handle::current();
        let port = db::server_port(context)?;
        tokio::task::spawn_blocking(move || {
            execute(
                &handle, port, &path, &db_name, &account2, &password2, gzip, import_id,
            )
        })
        .await
        .unwrap_or_else(|_| Err(failed("the import was cancelled")))
    };
    // Locked again with a password nobody knows; the account stays as the definer.
    if let Ok(mut admin) = db::root_connection(context).await {
        let unknown = random_password().unwrap_or_default();
        let hash: Option<String> = admin
            .exec_first("SELECT PASSWORD(?)", (&unknown,))
            .await
            .ok()
            .flatten();
        if let Some(hash) = hash.filter(|h| h.len() == 41 && h.starts_with('*')) {
            let _ = admin
                .query_drop(format!(
                    "ALTER USER '{account}'@'%' IDENTIFIED BY PASSWORD '{hash}' ACCOUNT LOCK"
                ))
                .await;
        } else {
            let _ = admin
                .query_drop(format!("ALTER USER '{account}'@'%' ACCOUNT LOCK"))
                .await;
        }
        let _ = admin.disconnect().await;
    }
    ran
}

/// The account that defines the triggers, views and routines of a database's dump
/// (see [`strip_definer`]). `database` is a checked plain identifier.
#[must_use]
pub fn definer_account(database: &str) -> String {
    format!("dhdef-{database}")
}

/// Takes the `DEFINER=user@host` clause out of a `CREATE` statement (plain, or
/// inside a `/*!50017 ... */` version comment, as `mysqldump` writes it), so that
/// the object is defined by the account that runs the import; a dump may name
/// any user, and naming one needs a right the import account must not have.
/// Anything that is not such a clause is returned as it was.
#[must_use]
pub fn strip_definer(statement: &str) -> std::borrow::Cow<'_, str> {
    let head_len = statement
        .char_indices()
        .nth(400)
        .map_or(statement.len(), |(i, _)| i);
    let head = &statement[..head_len];
    let lower = head.to_ascii_lowercase();
    let Some(at) = lower.find("definer") else {
        return std::borrow::Cow::Borrowed(statement);
    };
    // Before it only the words of the start of a `CREATE` (no quotes, no
    // parentheses): the word is not part of a body or a value.
    let before = &lower[..at];
    let plain = before.chars().all(|c| {
        c.is_ascii_alphanumeric() || c.is_whitespace() || matches!(c, '=' | '/' | '*' | '!' | '_')
    });
    let trimmed = before.trim_start_matches(|c: char| c.is_whitespace());
    let starts = trimmed.starts_with("create")
        || (trimmed.starts_with("/*!") && trimmed.contains("create"))
        || trimmed.starts_with("alter");
    if !plain || !starts {
        return std::borrow::Cow::Borrowed(statement);
    }
    let rest = &statement[at + "definer".len()..];
    let bytes = rest.as_bytes();
    let mut i = 0;
    let skip_ws = |i: &mut usize| {
        while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
            *i += 1;
        }
    };
    skip_ws(&mut i);
    if bytes.get(i) != Some(&b'=') {
        return std::borrow::Cow::Borrowed(statement);
    }
    i += 1;
    skip_ws(&mut i);
    // A name: quoted with `, ' or ", or a plain word.
    let name = |i: &mut usize| -> bool {
        match bytes.get(*i) {
            Some(&q @ (b'`' | b'\'' | b'"')) => {
                *i += 1;
                while *i < bytes.len() {
                    if bytes[*i] == q {
                        if bytes.get(*i + 1) == Some(&q) {
                            *i += 2;
                            continue;
                        }
                        *i += 1;
                        return true;
                    }
                    *i += 1;
                }
                false
            }
            Some(_) => {
                let start = *i;
                while *i < bytes.len()
                    && (bytes[*i].is_ascii_alphanumeric()
                        || matches!(bytes[*i], b'_' | b'-' | b'.' | b'%' | b'$'))
                {
                    *i += 1;
                }
                *i > start
            }
            None => false,
        }
    };
    let current = rest[i..].to_ascii_lowercase();
    if current.starts_with("current_user") {
        i += "current_user".len();
        skip_ws(&mut i);
        if bytes.get(i) == Some(&b'(') {
            i += 1;
            skip_ws(&mut i);
            if bytes.get(i) == Some(&b')') {
                i += 1;
            }
        }
    } else {
        if !name(&mut i) {
            return std::borrow::Cow::Borrowed(statement);
        }
        skip_ws(&mut i);
        if bytes.get(i) == Some(&b'@') {
            i += 1;
            skip_ws(&mut i);
            if !name(&mut i) {
                return std::borrow::Cow::Borrowed(statement);
            }
        }
    }
    let mut out = String::with_capacity(statement.len());
    out.push_str(&statement[..at]);
    out.push_str(&rest[i..]);
    std::borrow::Cow::Owned(out)
}

#[allow(clippy::too_many_arguments)]
fn execute(
    handle: &tokio::runtime::Handle,
    port: u16,
    path: &Path,
    database: &str,
    account: &str,
    password: &str,
    gzip: bool,
    import_id: Uuid,
) -> Result<u64, AgentError> {
    let file = std::fs::File::open(path)
        .map_err(|e| failed(&format!("cannot read the dump: {}", e.kind())))?;
    let counting = Counting {
        inner: file,
        import_id,
        read: 0,
    };
    let mut reader: Box<dyn Read> = if gzip {
        Box::new(flate2::read::MultiGzDecoder::new(counting))
    } else {
        Box::new(counting)
    };
    let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(port)
        .user(Some(account))
        .pass(Some(password))
        .db_name(Some(database))
        .into();
    let mut conn = handle
        .block_on(mysql_async::Conn::new(opts))
        .map_err(|e| failed(&server_words(&e)).retryable())?;
    let mut splitter = Splitter::new(MAX_STATEMENT_BYTES);
    let mut buf = vec![0u8; 256 * 1024];
    let count = std::cell::Cell::new(0u64);
    let run_all =
        |splitter: &mut Splitter, conn: &mut mysql_async::Conn| -> Result<(), AgentError> {
            loop {
                match splitter.next_statement() {
                    Ok(Some(statement)) => {
                        count.set(count.get() + 1);
                        if let Err(e) =
                            handle.block_on(conn.query_drop(strip_definer(&statement).as_ref()))
                        {
                            return Err(failed(&format!(
                                "statement {}: {}",
                                count.get(),
                                server_words(&e)
                            )));
                        }
                        update(import_id, |p| p.statements = count.get());
                    }
                    Ok(None) => return Ok(()),
                    Err(e) => return Err(split_error(&e, count.get())),
                }
            }
        };
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            if gzip {
                failed("the dump is not valid gzip data")
            } else {
                failed(&format!("cannot read the dump: {}", e.kind()))
            }
        })?;
        if n == 0 {
            break;
        }
        splitter.push(&buf[..n]);
        run_all(&mut splitter, &mut conn)?;
    }
    splitter
        .finish()
        .map_err(|e| split_error(&e, count.get()))?;
    run_all(&mut splitter, &mut conn)?;
    let _ = handle.block_on(conn.disconnect());
    Ok(count.get())
}

fn split_error(e: &SplitError, after: u64) -> AgentError {
    failed(&format!("after statement {after}: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_temporary_password_is_long_and_mixed() {
        let p = random_password().unwrap();
        assert!(
            builtpanel_domain::databases::check_password(&p).is_ok(),
            "{p}"
        );
        assert_ne!(p, random_password().unwrap());
    }

    #[test]
    fn definers_are_taken_out_of_create_statements_only() {
        let cases = [
            (
                "CREATE DEFINER=`root`@`%` TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET NEW.a = 1",
                "CREATE  TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET NEW.a = 1",
            ),
            (
                "CREATE ALGORITHM=UNDEFINED DEFINER = 'bob'@'localhost' SQL SECURITY DEFINER VIEW v AS SELECT 1",
                "CREATE ALGORITHM=UNDEFINED  SQL SECURITY DEFINER VIEW v AS SELECT 1",
            ),
            (
                "/*!50003 CREATE*/ /*!50017 DEFINER=`root`@`localhost`*/ /*!50003 TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET NEW.a = 1*/",
                "/*!50003 CREATE*/ /*!50017 */ /*!50003 TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET NEW.a = 1*/",
            ),
            (
                "create definer=current_user() procedure p() select 1",
                "create  procedure p() select 1",
            ),
            (
                "CREATE DEFINER=CURRENT_USER FUNCTION f() RETURNS int RETURN 1",
                "CREATE FUNCTION f() RETURNS int RETURN 1",
            ),
            (
                "CREATE DEFINER=`a``b`@`h-1.example` EVENT e ON SCHEDULE EVERY 1 DAY DO SELECT 1",
                "CREATE  EVENT e ON SCHEDULE EVERY 1 DAY DO SELECT 1",
            ),
        ];
        for (before, after) in cases {
            assert_eq!(strip_definer(before), after, "{before}");
        }
        // Not a clause: a value, a body, another statement.
        for same in [
            "INSERT INTO t VALUES ('create definer=x@y')",
            "SELECT 'DEFINER=root@%'",
            "CREATE TABLE definer (a int)",
            "CREATE TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET NEW.a = 'definer=a@b'",
            "GRANT SELECT ON a.* TO definer",
        ] {
            assert_eq!(strip_definer(same), same, "{same}");
        }
    }
}
