//! Running an uploaded dump in a tenant's PostgreSQL database (T07.12), the counterpart of
//! [`super::sqlimport`] for MariaDB: the same download, the same checks, the same progress.
//!
//! The dump runs as a temporary account made for that one import, a member of the database's
//! `__all` role and nothing else: whatever the dump says (`\connect` is refused outright,
//! `CREATE ROLE`, `ALTER SYSTEM`, `COPY … FROM PROGRAM`, `DROP DATABASE` of another) is refused
//! by the server as it would be for the tenant's own user. The account is removed at the end,
//! whatever the outcome, and what it made is handed to the database's `__all` role, so every
//! user with full access can manage it. Statements that only say who owns something or who
//! may use it (`ALTER … OWNER TO`, `GRANT`, `REVOKE`, `COMMENT ON EXTENSION`, `CREATE
//! EXTENSION`) are skipped when the server refuses them: a dump from another server names
//! roles that are not here, and refusing to start over that would make most dumps useless.
//! The first other statement the server refuses stops the import and is named.

use std::{io::Read as _, path::Path};

use builtpanel_node_protocol::AgentError;
use futures_util::SinkExt as _;
use uuid::Uuid;

use super::{
    pg,
    pgsplit::{Item, PgSplitter, SplitError},
    sqlimport::{Counting, failed, random_password, update},
    tenant::TenantContext,
};

/// Largest statement run.
const MAX_STATEMENT_BYTES: usize = 64 << 20;

/// What the server said, on one line and clipped.
fn words(err: &tokio_postgres::Error) -> String {
    match err.as_db_error() {
        Some(e) => {
            let text: String = e
                .message()
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect();
            format!("error {}: {text}", e.code().code())
        }
        None => "the connection to the database server failed".to_owned(),
    }
}

fn split_error(e: &SplitError, after: u64) -> AgentError {
    failed(&format!("after statement {after}: {e}"))
}

/// Whether a statement only concerns ownership, privileges or extensions, so that the server
/// refusing it is no reason to stop.
fn skippable(statement: &str) -> bool {
    let mut text = statement;
    // Leading comments.
    loop {
        text = text.trim_start();
        if let Some(r) = text.strip_prefix("--") {
            text = r.split_once('\n').map_or("", |(_, t)| t);
        } else if text.starts_with("/*") {
            match text.find("*/") {
                Some(i) => text = &text[i + 2..],
                None => return false,
            }
        } else {
            break;
        }
    }
    let head: String = text
        .chars()
        .take(120)
        .collect::<String>()
        .to_ascii_lowercase();
    let words: Vec<&str> = head.split_whitespace().collect();
    match words.as_slice() {
        ["grant" | "revoke", ..] | ["create", "extension", ..] => true,
        ["comment", "on", "extension" | "schema", ..] => true,
        ["alter", "default", "privileges", ..] => true,
        ["alter", ..] => words.windows(2).any(|w| w == ["owner", "to"]),
        _ => false,
    }
}

/// Runs a dump that is in a file (plain or gzip) into `database`, as a temporary account
/// that is removed afterwards. Returns how many statements ran.
///
/// # Errors
/// The server failing or refusing a statement, a dump that is not valid, or the disk.
pub(super) async fn import_file(
    context: &TenantContext,
    database: &str,
    import_id: Uuid,
    path: &Path,
    gzip: bool,
) -> Result<u64, AgentError> {
    pg::ensure_server(context).await?;
    pg::identifier(database)?;
    let password =
        random_password().map_err(|e| failed(&format!("cannot make a password: {}", e.kind())))?;
    let account = format!("{database}__imp");
    let port = pg::server_port(context)?;
    let admin = pg::connect(context, "postgres").await?;
    let created = pg::create_temporary_role(
        context,
        &admin,
        &account,
        &password,
        &format!("{database}__all"),
        "0.0.0.0/0",
    )
    .await;
    created?;
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
        tokio::task::spawn_blocking(move || {
            execute(
                &handle, port, &path, &db_name, &account2, &password2, gzip, import_id,
            )
        })
        .await
        .unwrap_or_else(|_| Err(failed("the import was cancelled")))
    };
    // Whatever happened: what the account made goes to the database's `all` role, the
    // groups get their privileges on it, and the account is removed.
    let cleaned = pg::finish_temporary_role(context, database, &account, true).await;
    let refreshed = pg::refresh_privileges(context, database).await;
    let statements = ran?;
    cleaned?;
    refreshed?;
    Ok(statements)
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
    let counting = Counting::new(file, import_id);
    let mut reader: Box<dyn std::io::Read> = if gzip {
        Box::new(flate2::read::MultiGzDecoder::new(counting))
    } else {
        Box::new(counting)
    };
    let client = handle
        .block_on(pg::connect_as(password, account, database, port))
        .map_err(|e| failed(&e.message).retryable())?;
    let mut splitter = PgSplitter::new(MAX_STATEMENT_BYTES);
    let mut count = 0u64;
    // The `COPY` that is being fed, if any.
    let mut copy: Option<std::pin::Pin<Box<tokio_postgres::CopyInSink<bytes::Bytes>>>> = None;
    let mut buf = vec![0u8; 256 * 1024];
    let drain = |splitter: &mut PgSplitter,
                 copy: &mut Option<
        std::pin::Pin<Box<tokio_postgres::CopyInSink<bytes::Bytes>>>,
    >,
                 count: &mut u64|
     -> Result<(), AgentError> {
        loop {
            let item = splitter.next_item().map_err(|e| split_error(&e, *count))?;
            let Some(item) = item else {
                return Ok(());
            };
            match item {
                Item::Statement(statement) => {
                    *count += 1;
                    if let Err(e) = handle.block_on(client.simple_query(&statement))
                        && !(skippable(&statement) && e.as_db_error().is_some())
                    {
                        return Err(failed(&format!("statement {}: {}", *count, words(&e))));
                    }
                    update(import_id, |p| p.statements = *count);
                }
                Item::CopyStart(statement) => {
                    *count += 1;
                    let sink = handle
                        .block_on(client.copy_in::<str, bytes::Bytes>(statement.as_str()))
                        .map_err(|e| failed(&format!("statement {}: {}", *count, words(&e))))?;
                    *copy = Some(Box::pin(sink));
                    update(import_id, |p| p.statements = *count);
                }
                Item::CopyData(rows) => {
                    if let Some(sink) = copy.as_mut() {
                        handle
                            .block_on(sink.send(bytes::Bytes::from(rows)))
                            .map_err(|e| failed(&format!("statement {}: {}", *count, words(&e))))?;
                    }
                }
                Item::CopyEnd => {
                    if let Some(mut sink) = copy.take() {
                        handle
                            .block_on(sink.as_mut().finish())
                            .map_err(|e| failed(&format!("statement {}: {}", *count, words(&e))))?;
                    }
                }
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
        drain(&mut splitter, &mut copy, &mut count)?;
    }
    splitter.finish().map_err(|e| split_error(&e, count))?;
    drain(&mut splitter, &mut copy, &mut count)?;
    Ok(count)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn only_statements_about_owners_privileges_and_extensions_may_fail_quietly() {
        for ok in [
            "ALTER TABLE public.t OWNER TO alice_web",
            "alter sequence s owner to x",
            "-- Name: t; Type: ACL\nGRANT SELECT ON TABLE t TO bob",
            "REVOKE ALL ON SCHEMA public FROM PUBLIC",
            "CREATE EXTENSION IF NOT EXISTS pgcrypto",
            "COMMENT ON EXTENSION plpgsql IS 'x'",
            "ALTER DEFAULT PRIVILEGES FOR ROLE a GRANT SELECT ON TABLES TO b",
            "/* c */ ALTER SCHEMA public OWNER TO pg_database_owner",
        ] {
            assert!(skippable(ok), "{ok}");
        }
        for stop in [
            "CREATE TABLE t (a int)",
            "ALTER TABLE t ADD COLUMN b int",
            "INSERT INTO t VALUES (1)",
            "DROP TABLE t",
            "SELECT 1",
            "COMMENT ON TABLE t IS 'x'",
        ] {
            assert!(!skippable(stop), "{stop}");
        }
    }
}
