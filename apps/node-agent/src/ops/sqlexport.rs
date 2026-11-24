//! `export_sql` (T07.08): one database dumped to a gzip file and sent to the
//! control plane in chunks.
//!
//! The dump is made by `mariadb-dump` inside the database container (a fixed
//! command, see [`builtpanel_docker_manager::DockerManager::dump_database`]) as a
//! temporary account that can only read that one database; the account and the
//! local file are gone when this ends, whatever the outcome.

use std::{
    io::{Read, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use builtpanel_domain::TenantUsername;
use builtpanel_node_protocol::{AgentError, Operation, Output};
use mysql_async::prelude::Queryable as _;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::{
    db,
    sqlimport::{failed, invalid, random_password, server_words},
    tenant::{TenantContext, docker_error},
};

/// Largest chunk sent (the control plane takes 8 MiB).
const CHUNK_BYTES: usize = 8 << 20;
/// Largest compressed dump kept.
const MAX_EXPORT_BYTES: u64 = 8 << 30;
/// Longest the whole thing may take.
pub(super) const MAX_DURATION: Duration = Duration::from_secs(4 * 3600);

fn exports_dir(context: &TenantContext) -> PathBuf {
    context.vhosts_root.with_file_name("exports")
}

/// `export_sql`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable), the dump failing with the
/// server's words, or the upload failing (retryable).
pub async fn export_sql(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::ExportSql {
        username,
        database,
        export_id,
        url,
        token,
        engine,
        ..
    } = op
    else {
        return Err(super::wrong_operation());
    };
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
        && !url.ends_with('/')
        && (url.starts_with("https://") || local);
    if !plain_url {
        return Err(invalid("the upload address must be https"));
    }
    let token = token
        .ok_or_else(|| invalid("no credential for the upload"))?
        .0;
    let dir = exports_dir(&context);
    std::fs::create_dir_all(&dir)
        .map_err(|e| failed(&format!("cannot keep the dump: {}", e.kind())).retryable())?;
    let path = dir.join(format!("{}.sql.gz", export_id.simple()));
    let result = tokio::time::timeout(
        MAX_DURATION,
        run(&context, engine, &database, export_id, &url, &token, &path),
    )
    .await
    .unwrap_or_else(|_| Err(failed("the export took longer than four hours")));
    let _ = std::fs::remove_file(&path);
    result.map(|size_bytes| Output::SqlExported { size_bytes })
}

/// Dumps one database of the server this task works on into a gzip file, as a temporary
/// account that can only read it (removed again, whatever the outcome).
///
/// # Errors
/// The server failing, or the disk.
pub(super) async fn dump_to_file(
    context: &TenantContext,
    engine: builtpanel_node_protocol::DbEngine,
    database: &str,
    tag: &str,
    path: &Path,
) -> Result<(), AgentError> {
    if engine == builtpanel_node_protocol::DbEngine::Postgres {
        // A temporary account that can only read this database, for the dump only.
        super::pg::ensure_server(context).await?;
        let password = random_password()
            .map_err(|e| failed(&format!("cannot make a password: {}", e.kind())))?;
        let account = format!("{database}__exp");
        let admin = super::pg::connect(context, "postgres").await?;
        super::pg::create_temporary_role(
            context,
            &admin,
            &account,
            &password,
            &format!("{database}__ro"),
            "127.0.0.1/32",
        )
        .await?;
        let dumped = dump(context, engine, database, &account, &password, path).await;
        let _ = super::pg::finish_temporary_role(context, database, &account, false).await;
        dumped?;
    } else {
        let (account, password) = reader_account(context, database, tag).await?;
        let dumped = dump(context, engine, database, &account, &password, path).await;
        drop_account(context, &account).await;
        dumped?;
    }
    Ok(())
}

async fn run(
    context: &TenantContext,
    engine: builtpanel_node_protocol::DbEngine,
    database: &str,
    export_id: Uuid,
    url: &str,
    token: &str,
    path: &Path,
) -> Result<u64, AgentError> {
    dump_to_file(
        context,
        engine,
        database,
        &export_id.simple().to_string()[..12],
        path,
    )
    .await?;
    let (url, token, path) = (url.to_owned(), token.to_owned(), path.to_owned());
    tokio::task::spawn_blocking(move || upload(&url, &token, &path))
        .await
        .map_err(|_| failed("the upload was cancelled"))?
}

/// Makes a temporary account that can only read `database` (and see how its routines are
/// made), and returns its name and password. [`drop_account`] removes it.
///
/// # Errors
/// The database server failing.
pub(super) async fn reader_account(
    context: &TenantContext,
    database: &str,
    tag: &str,
) -> Result<(String, String), AgentError> {
    db::ensure_server(context).await?;
    let password =
        random_password().map_err(|e| failed(&format!("cannot make a password: {}", e.kind())))?;
    let account = format!("dhexport_{tag}");
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
    for sql in [
        format!("DROP USER IF EXISTS '{account}'@'%'"),
        format!(
            "CREATE USER '{account}'@'%' IDENTIFIED BY PASSWORD '{hash}' WITH MAX_USER_CONNECTIONS 2"
        ),
        format!(
            "GRANT SELECT, SHOW VIEW, TRIGGER, EVENT, EXECUTE ON `{database}`.* TO '{account}'@'%'"
        ),
        // Reading a routine's definition is a global right (MariaDB 11.3+); the account
        // has no other right outside this database and lives for the dump only.
        format!("GRANT SHOW CREATE ROUTINE ON *.* TO '{account}'@'%'"),
    ] {
        admin
            .query_drop(sql)
            .await
            .map_err(|e| failed(&server_words(&e)))?;
    }
    let _ = admin.disconnect().await;
    Ok((account, password))
}

/// Removes an account made by [`reader_account`].
pub(super) async fn drop_account(context: &TenantContext, account: &str) {
    if let Ok(mut admin) = db::root_connection(context).await {
        let _ = admin
            .query_drop(format!("DROP USER IF EXISTS '{account}'@'%'"))
            .await;
        let _ = admin.disconnect().await;
    }
}

/// Runs the dump into the gzip file at `path`.
async fn dump(
    context: &TenantContext,
    engine: builtpanel_node_protocol::DbEngine,
    database: &str,
    account: &str,
    password: &str,
    path: &Path,
) -> Result<(), AgentError> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    let file = std::fs::File::create(path)
        .map_err(|e| failed(&format!("cannot keep the dump: {}", e.kind())).retryable())?;
    let writer = tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
        let mut gz = flate2::write::GzEncoder::new(
            std::io::BufWriter::with_capacity(1 << 20, file),
            flate2::Compression::new(5),
        );
        let mut raw = 0u64;
        while let Some(piece) = rx.blocking_recv() {
            raw += piece.len() as u64;
            gz.write_all(&piece)?;
            // The compressed size cannot be known cheaply; the raw size bounds it
            // well enough for a runaway dump.
            if raw > 64 * MAX_EXPORT_BYTES {
                return Err(std::io::Error::other("the dump is far too big"));
            }
        }
        gz.finish()?.flush()?;
        Ok(raw)
    });
    let ran = if engine == builtpanel_node_protocol::DbEngine::Postgres {
        let container = builtpanel_docker_manager::Name::parse(&super::pg::container_name())
            .map_err(|e| failed(&e.to_string()))?;
        context
            .docker
            .dump_pg_database(&container, database, account, password, tx, MAX_DURATION)
            .await
    } else {
        let container = builtpanel_docker_manager::Name::parse(&db::container_name())
            .map_err(|e| failed(&e.to_string()))?;
        context
            .docker
            .dump_database(&container, database, account, password, tx, MAX_DURATION)
            .await
    };
    let written = writer
        .await
        .map_err(|_| failed("the dump was cancelled"))?
        .map_err(|e| failed(&format!("cannot keep the dump: {e}")).retryable());
    let (code, stderr) = ran.map_err(|e| docker_error(&e))?;
    if code != 0 {
        let said: String = stderr.lines().take(3).collect::<Vec<_>>().join(" ");
        return Err(failed(&format!(
            "the dump failed (exit {code}): {}",
            said.chars().take(300).collect::<String>()
        )));
    }
    written?;
    Ok(())
}

/// Sends the file in chunks, then says it is whole.
pub(super) fn upload(url: &str, token: &str, path: &Path) -> Result<u64, AgentError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(120)))
        .max_redirects(0)
        .build()
        .into();
    let bearer = format!("Bearer {token}");
    let mut file = std::fs::File::open(path)
        .map_err(|e| failed(&format!("cannot read the dump: {}", e.kind())))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; CHUNK_BYTES];
    let mut n = 0u32;
    loop {
        let mut filled = 0;
        while filled < buf.len() {
            let read = file
                .read(&mut buf[filled..])
                .map_err(|e| failed(&format!("cannot read the dump: {}", e.kind())))?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled == 0 {
            break;
        }
        hasher.update(&buf[..filled]);
        total += filled as u64;
        if total > MAX_EXPORT_BYTES {
            return Err(failed("the dump is bigger than the limit"));
        }
        send_chunk(
            &agent,
            &format!("{url}/chunks/{n}"),
            &bearer,
            &buf[..filled],
        )?;
        n += 1;
    }
    if total == 0 {
        return Err(failed("the dump is empty"));
    }
    let sha256: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let body = serde_json::json!({ "size_bytes": total, "sha256": sha256 }).to_string();
    let response = agent
        .post(&format!("{url}/complete"))
        .header("Authorization", &bearer)
        .header("Content-Type", "application/json")
        .send(body.as_bytes())
        .map_err(|_| failed("the control plane could not be reached").retryable())?;
    match response.status().as_u16() {
        200 | 202 | 204 => Ok(total),
        401 | 403 | 404 | 409 | 410 | 422 => {
            Err(failed("the control plane refused the finished dump"))
        }
        _ => Err(failed("the control plane did not take the finished dump").retryable()),
    }
}

fn send_chunk(agent: &ureq::Agent, url: &str, bearer: &str, data: &[u8]) -> Result<(), AgentError> {
    let mut last = failed("the control plane could not be reached").retryable();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2 * attempt));
        }
        if let Ok(response) = agent
            .put(url)
            .header("Authorization", bearer)
            .header("Content-Type", "application/octet-stream")
            .send(data)
        {
            match response.status().as_u16() {
                200 | 204 => return Ok(()),
                401 | 403 | 404 | 409 | 410 | 413 | 422 => {
                    return Err(failed("the control plane refused a piece of the dump"));
                }
                _ => {
                    last = failed("the control plane did not take a piece of the dump").retryable()
                }
            }
        }
    }
    Err(last)
}
