//! The database studio for PostgreSQL (see [`super::studio`] for the MariaDB side).
//!
//! A request makes a short-lived login role that is a member of the database's `all`
//! group and nothing else, connects as it to that one database, does the work and drops
//! the role (what it made goes to the group, as after an import). Table and column names
//! are quoted; values are written as escaped string literals, which the server reads as
//! the column's own type, and the answer is read in the text protocol, so every value
//! arrives as the text the server would print.

use std::time::Instant;

use builtpanel_node_protocol::{
    AgentError,
    studio::{
        Assignment, ColumnInfo, Filter, FilterOp, Grid, IndexInfo, KeyPart, MAX_CELL_BYTES,
        MAX_PAGE_ROWS, MAX_QUERY_ROWS, StudioErrorCode, StudioRequest, StudioResult, TableInfo,
    },
};
use tokio_postgres::{Client, SimpleQueryMessage};

use super::{
    pg,
    sqlimport::{failed, random_password},
    tenant::TenantContext,
};

/// Longest a statement may run.
const STATEMENT_TIMEOUT: &str = "20s";
/// What an answer may hold in all (a frame is 1 MiB, with the envelope around it).
const ANSWER_BUDGET: usize = 700 * 1024;

fn error(code: StudioErrorCode, message: impl Into<String>) -> StudioResult {
    StudioResult::Error {
        code,
        message: message.into(),
    }
}

fn invalid(message: &str) -> StudioResult {
    error(StudioErrorCode::Invalid, message)
}

/// What the server said, on one line and clipped.
fn words(err: &tokio_postgres::Error) -> String {
    match err.as_db_error() {
        Some(db) => {
            let text: String = db
                .message()
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect();
            format!("error {}: {text}", db.code().code())
        }
        None => "the connection to the database server failed".to_owned(),
    }
}

fn sql_failed(err: &tokio_postgres::Error) -> StudioResult {
    error(StudioErrorCode::Sql, words(err))
}

/// `"name"` for a table or column name.
fn quote(name: &str) -> Result<String, StudioResult> {
    if name.is_empty()
        || name.chars().count() > 63
        || name.chars().any(|c| c.is_control() || c == '\0')
    {
        return Err(invalid("A table or column name is not valid."));
    }
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}

/// A value as a string literal. With `standard_conforming_strings` on (set for the session)
/// only the quote itself needs doubling; a NUL cannot be in a value at all.
fn literal(value: &str) -> Result<String, StudioResult> {
    if value.contains('\0') {
        return Err(invalid("A value cannot hold a NUL character."));
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}

fn optional(value: Option<&String>) -> Result<String, StudioResult> {
    value.map_or_else(|| Ok("NULL".to_owned()), |v| literal(v))
}

/// The studio's work on one database, with a role of its own for it.
///
/// # Errors
/// [`AgentError`]: the server is not reachable (retryable).
pub(super) async fn run(
    context: &TenantContext,
    database: &str,
    request: StudioRequest,
) -> Result<StudioResult, AgentError> {
    pg::ensure_server(context).await?;
    pg::identifier(database)?;
    let password =
        random_password().map_err(|e| failed(&format!("cannot make a password: {}", e.kind())))?;
    let mut raw = [0u8; 6];
    std::io::Read::read_exact(
        &mut std::fs::File::open("/dev/urandom").map_err(io)?,
        &mut raw,
    )
    .map_err(io)?;
    let account = format!(
        "dhst_{}",
        raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let admin = pg::connect(context, "postgres").await?;
    pg::create_temporary_role(
        context,
        &admin,
        &account,
        &password,
        &format!("{database}__all"),
        "0.0.0.0/0",
    )
    .await?;
    let outcome = async {
        let client = pg::connect_as(&password, &account, database, pg::server_port(context)?).await?;
        client
            .batch_execute(&format!(
                "SET statement_timeout = '{STATEMENT_TIMEOUT}'; SET standard_conforming_strings = on; SET client_min_messages = warning"
            ))
            .await
            .map_err(|e| pg::sql_error(&e))?;
        Ok::<_, AgentError>(dispatch(&client, request).await)
    }
    .await;
    // Whatever happened: what the role made goes to the group, and the role is dropped.
    let cleaned = pg::finish_temporary_role(context, database, &account, true).await;
    let refreshed = pg::refresh_privileges(context, database).await;
    let result = outcome?;
    cleaned?;
    refreshed?;
    Ok(result)
}

fn io(e: std::io::Error) -> AgentError {
    failed(&format!("cannot prepare the session: {}", e.kind()))
}

async fn dispatch(client: &Client, request: StudioRequest) -> StudioResult {
    match request {
        StudioRequest::Tables => tables(client).await,
        StudioRequest::Describe { table } => describe(client, &table).await,
        StudioRequest::Rows {
            table,
            offset,
            limit,
            order_by,
            descending,
            filters,
        } => {
            rows(
                client,
                &table,
                offset,
                limit,
                order_by.as_deref(),
                descending,
                &filters,
            )
            .await
        }
        StudioRequest::Query { sql } => query(client, &sql).await,
        StudioRequest::InsertRow { table, values } => insert(client, &table, &values).await,
        StudioRequest::UpdateRow { table, key, values } => {
            update(client, &table, &key, &values).await
        }
        StudioRequest::DeleteRow { table, key } => delete(client, &table, &key).await,
    }
}

/// Text rows of a simple query: `(column names, rows, affected, more rows existed)`.
async fn simple(
    client: &Client,
    sql: &str,
    max_rows: usize,
) -> Result<(Vec<String>, Vec<Vec<Option<String>>>, u64, bool), tokio_postgres::Error> {
    let mut columns: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    let mut affected = 0;
    let mut more = false;
    let mut first_set = true;
    for message in client.simple_query(sql).await? {
        match message {
            SimpleQueryMessage::Row(row) => {
                if !first_set {
                    continue;
                }
                if columns.is_empty() {
                    columns = row.columns().iter().map(|c| c.name().to_owned()).collect();
                }
                if rows.len() >= max_rows {
                    more = true;
                    continue;
                }
                rows.push(
                    (0..row.len())
                        .map(|i| row.get(i).map(str::to_owned))
                        .collect(),
                );
            }
            SimpleQueryMessage::CommandComplete(n) => {
                affected = n;
                first_set = false;
            }
            _ => {}
        }
    }
    Ok((columns, rows, affected, more))
}

/// Rows to a grid, cells cut and the whole kept within the answer's budget.
fn grid(columns: Vec<String>, rows: Vec<Vec<Option<String>>>, more: bool) -> Grid {
    let mut out = Vec::new();
    let mut spent = 0usize;
    let mut truncated = more;
    for row in rows {
        let line: Vec<Option<String>> = row
            .into_iter()
            .map(|cell| {
                cell.map(|text| {
                    if text.len() > MAX_CELL_BYTES {
                        truncated = true;
                        let mut end = MAX_CELL_BYTES;
                        while !text.is_char_boundary(end) {
                            end -= 1;
                        }
                        text[..end].to_owned()
                    } else {
                        text
                    }
                })
            })
            .collect();
        spent += line
            .iter()
            .map(|c| c.as_ref().map_or(1, String::len) + 4)
            .sum::<usize>();
        if spent > ANSWER_BUDGET {
            truncated = true;
            break;
        }
        out.push(line);
    }
    Grid {
        columns,
        rows: out,
        truncated,
    }
}

async fn tables(client: &Client) -> StudioResult {
    let found = simple(
        client,
        "SELECT c.relname, CASE WHEN c.relkind IN ('v','m') THEN 'view' ELSE 'table' END, \
                GREATEST(c.reltuples, 0)::bigint::text, pg_total_relation_size(c.oid)::text \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND c.relkind IN ('r','p','v','m') ORDER BY c.relname",
        10_000,
    )
    .await;
    match found {
        Ok((_, rows, _, _)) => StudioResult::Tables {
            tables: rows
                .into_iter()
                .map(|r| {
                    let get = |i: usize| r.get(i).cloned().flatten().unwrap_or_default();
                    TableInfo {
                        name: get(0),
                        kind: get(1),
                        rows: get(2).parse().unwrap_or(0),
                        size_bytes: get(3).parse().unwrap_or(0),
                    }
                })
                .collect(),
        },
        Err(e) => sql_failed(&e),
    }
}

/// The table's oid as a `regclass` expression for a quoted name in `public`.
fn regclass(table: &str) -> Result<String, StudioResult> {
    Ok(format!(
        "to_regclass({})",
        literal(&format!("public.{}", quote(table)?))?
    ))
}

async fn describe(client: &Client, table: &str) -> StudioResult {
    let class = match regclass(table) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let columns = simple(
        client,
        &format!(
            "SELECT a.attname, format_type(a.atttypid, a.atttypmod), \
                    CASE WHEN a.attnotnull THEN 'f' ELSE 't' END, \
                    CASE WHEN EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid AND i.indisprimary AND a.attnum = ANY(i.indkey)) THEN 't' ELSE 'f' END, \
                    pg_get_expr(d.adbin, d.adrelid), \
                    CASE WHEN a.attidentity <> '' OR a.attgenerated <> '' OR pg_get_expr(d.adbin, d.adrelid) LIKE 'nextval(%' THEN 't' ELSE 'f' END \
             FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE a.attrelid = {class} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
        ),
        1000,
    )
    .await;
    let columns: Vec<ColumnInfo> = match columns {
        Ok((_, rows, _, _)) if rows.is_empty() => {
            return error(StudioErrorCode::NotFound, "No such table.");
        }
        Ok((_, rows, _, _)) => rows
            .into_iter()
            .map(|r| {
                let text = |i: usize| r.get(i).cloned().flatten();
                ColumnInfo {
                    name: text(0).unwrap_or_default(),
                    data_type: text(1).unwrap_or_default(),
                    nullable: text(2).as_deref() == Some("t"),
                    primary_key: text(3).as_deref() == Some("t"),
                    default: text(4),
                    generated: text(5).as_deref() == Some("t"),
                }
            })
            .collect(),
        Err(e) => return sql_failed(&e),
    };
    let indexes = simple(
        client,
        &format!(
            "SELECT ic.relname, CASE WHEN i.indisunique THEN 't' ELSE 'f' END, \
                    (SELECT string_agg(pg_get_indexdef(i.indexrelid, k, true), E'\\x1f' ORDER BY k) \
                       FROM generate_series(1, i.indnkeyatts) k), \
                    pg_get_indexdef(i.indexrelid) \
             FROM pg_index i JOIN pg_class ic ON ic.oid = i.indexrelid \
             WHERE i.indrelid = {class} ORDER BY ic.relname"
        ),
        1000,
    )
    .await;
    let (indexes, index_sql): (Vec<IndexInfo>, Vec<String>) = match indexes {
        Ok((_, rows, _, _)) => rows
            .into_iter()
            .map(|r| {
                let text = |i: usize| r.get(i).cloned().flatten().unwrap_or_default();
                (
                    IndexInfo {
                        name: text(0),
                        columns: text(2).split('\x1f').map(str::to_owned).collect(),
                        unique: text(1) == "t",
                    },
                    text(3),
                )
            })
            .unzip(),
        Err(e) => return sql_failed(&e),
    };
    // PostgreSQL has no `SHOW CREATE TABLE`: it is written out from what the catalog says.
    let mut create = format!("CREATE TABLE {} (\n", quote(table).unwrap_or_default());
    let mut parts: Vec<String> = columns
        .iter()
        .map(|c| {
            format!(
                "  {} {}{}{}",
                quote(&c.name).unwrap_or_default(),
                c.data_type,
                if c.nullable { "" } else { " NOT NULL" },
                c.default
                    .as_ref()
                    .map_or_else(String::new, |d| format!(" DEFAULT {d}"))
            )
        })
        .collect();
    let primary: Vec<String> = columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| quote(&c.name).unwrap_or_default())
        .collect();
    if !primary.is_empty() {
        parts.push(format!("  PRIMARY KEY ({})", primary.join(", ")));
    }
    create.push_str(&parts.join(",\n"));
    create.push_str("\n);");
    for sql in index_sql {
        create.push_str(&format!("\n{sql};"));
    }
    StudioResult::Described {
        columns,
        indexes,
        create_sql: create,
    }
}

fn conditions(filters: &[Filter]) -> Result<String, StudioResult> {
    let mut parts = Vec::new();
    for filter in filters.iter().take(20) {
        let column = quote(&filter.column)?;
        let (sql, wants) = match filter.op {
            FilterOp::Eq => ("=", true),
            FilterOp::Ne => ("<>", true),
            FilterOp::Lt => ("<", true),
            FilterOp::Le => ("<=", true),
            FilterOp::Gt => (">", true),
            FilterOp::Ge => (">=", true),
            FilterOp::Like => ("LIKE", true),
            FilterOp::IsNull => ("IS NULL", false),
            FilterOp::NotNull => ("IS NOT NULL", false),
        };
        if wants {
            let Some(value) = &filter.value else {
                return Err(invalid("A condition needs a value."));
            };
            // `LIKE` wants text: other types are shown as text first.
            let left = if filter.op == FilterOp::Like {
                format!("{column}::text")
            } else {
                column
            };
            parts.push(format!("{left} {sql} {}", literal(value)?));
        } else {
            parts.push(format!("{column} {sql}"));
        }
    }
    Ok(if parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", parts.join(" AND "))
    })
}

async fn rows(
    client: &Client,
    table: &str,
    offset: u32,
    limit: u32,
    order_by: Option<&str>,
    descending: bool,
    filters: &[Filter],
) -> StudioResult {
    if limit == 0 || limit > MAX_PAGE_ROWS {
        return invalid("A page is 1 to 500 rows.");
    }
    let quoted = match quote(table) {
        Ok(q) => q,
        Err(e) => return e,
    };
    let clause = match conditions(filters) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let order = match order_by {
        Some(column) => match quote(column) {
            Ok(q) => format!(" ORDER BY {q} {}", if descending { "DESC" } else { "ASC" }),
            Err(e) => return e,
        },
        None => String::new(),
    };
    let total = match simple(client, &format!("SELECT count(*) FROM {quoted}{clause}"), 1).await {
        Ok((_, rows, _, _)) => rows
            .first()
            .and_then(|r| r.first().cloned().flatten())
            .and_then(|t| t.parse().ok())
            .unwrap_or(0),
        Err(e) => return sql_failed(&e),
    };
    match simple(
        client,
        &format!("SELECT * FROM {quoted}{clause}{order} LIMIT {limit} OFFSET {offset}"),
        MAX_PAGE_ROWS as usize,
    )
    .await
    {
        Ok((columns, found, _, _)) => StudioResult::Rows {
            grid: grid(columns, found, false),
            total,
        },
        Err(e) => sql_failed(&e),
    }
}

async fn query(client: &Client, sql: &str) -> StudioResult {
    let started = Instant::now();
    match simple(client, sql, MAX_QUERY_ROWS).await {
        Ok((columns, found, affected, more)) => StudioResult::Query {
            grid: (!columns.is_empty()).then(|| grid(columns, found, more)),
            affected,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        },
        Err(e) => sql_failed(&e),
    }
}

async fn change(client: &Client, sql: &str) -> StudioResult {
    match simple(client, sql, 0).await {
        Ok((_, _, affected, _)) => StudioResult::Changed { affected },
        Err(e) => sql_failed(&e),
    }
}

async fn insert(client: &Client, table: &str, values: &[Assignment]) -> StudioResult {
    let quoted = match quote(table) {
        Ok(q) => q,
        Err(e) => return e,
    };
    if values.is_empty() {
        return change(client, &format!("INSERT INTO {quoted} DEFAULT VALUES")).await;
    }
    let mut names = Vec::new();
    let mut literals = Vec::new();
    for item in values {
        match (quote(&item.column), optional(item.value.as_ref())) {
            (Ok(n), Ok(l)) => {
                names.push(n);
                literals.push(l);
            }
            (Err(e), _) | (_, Err(e)) => return e,
        }
    }
    change(
        client,
        &format!(
            "INSERT INTO {quoted} ({}) VALUES ({})",
            names.join(", "),
            literals.join(", ")
        ),
    )
    .await
}

/// `WHERE` for a primary key, after checking that `key` is the table's whole key.
async fn key_clause(client: &Client, table: &str, key: &[KeyPart]) -> Result<String, StudioResult> {
    let class = regclass(table)?;
    let primary = simple(
        client,
        &format!(
            "SELECT a.attname FROM pg_index i JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
             WHERE i.indrelid = {class} AND i.indisprimary"
        ),
        100,
    )
    .await
    .map_err(|e| sql_failed(&e))?
    .1;
    let mut primary: Vec<String> = primary
        .into_iter()
        .filter_map(|r| r.into_iter().next().flatten())
        .collect();
    if primary.is_empty() {
        return Err(invalid(
            "This table has no primary key, so a single row cannot be told apart; use the SQL editor.",
        ));
    }
    let mut given: Vec<&str> = key.iter().map(|k| k.column.as_str()).collect();
    primary.sort();
    given.sort_unstable();
    if primary.iter().map(String::as_str).collect::<Vec<_>>() != given {
        return Err(invalid("The key must be the table's whole primary key."));
    }
    let mut parts = Vec::new();
    for part in key {
        parts.push(format!(
            "{} = {}",
            quote(&part.column)?,
            literal(&part.value)?
        ));
    }
    Ok(parts.join(" AND "))
}

async fn update(
    client: &Client,
    table: &str,
    key: &[KeyPart],
    values: &[Assignment],
) -> StudioResult {
    if values.is_empty() {
        return invalid("Nothing to change.");
    }
    let quoted = match quote(table) {
        Ok(q) => q,
        Err(e) => return e,
    };
    let clause = match key_clause(client, table, key).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut sets = Vec::new();
    for item in values {
        match (quote(&item.column), optional(item.value.as_ref())) {
            (Ok(n), Ok(l)) => sets.push(format!("{n} = {l}")),
            (Err(e), _) | (_, Err(e)) => return e,
        }
    }
    change(
        client,
        &format!("UPDATE {quoted} SET {} WHERE {clause}", sets.join(", ")),
    )
    .await
}

async fn delete(client: &Client, table: &str, key: &[KeyPart]) -> StudioResult {
    let quoted = match quote(table) {
        Ok(q) => q,
        Err(e) => return e,
    };
    let clause = match key_clause(client, table, key).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    change(client, &format!("DELETE FROM {quoted} WHERE {clause}")).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn names_and_values_cannot_end_their_quotes() {
        assert_eq!(quote("orders").unwrap(), "\"orders\"");
        assert_eq!(quote("a\"b").unwrap(), "\"a\"\"b\"");
        assert!(quote("").is_err() && quote("x\ny").is_err());
        assert_eq!(literal("it's").unwrap(), "'it''s'");
        assert_eq!(
            literal("x'; DROP TABLE t; --").unwrap(),
            "'x''; DROP TABLE t; --'"
        );
        assert!(literal("a\0b").is_err());
    }

    #[test]
    fn a_filter_value_is_a_literal_and_like_compares_text() {
        let clause = conditions(&[
            Filter {
                column: "id".into(),
                op: FilterOp::Gt,
                value: Some("1' OR '1'='1".into()),
            },
            Filter {
                column: "name".into(),
                op: FilterOp::Like,
                value: Some("a%".into()),
            },
            Filter {
                column: "note".into(),
                op: FilterOp::IsNull,
                value: None,
            },
        ])
        .unwrap();
        assert_eq!(
            clause,
            " WHERE \"id\" > '1'' OR ''1''=''1' AND \"name\"::text LIKE 'a%' AND \"note\" IS NULL"
        );
    }
}
