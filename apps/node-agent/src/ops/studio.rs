//! `studio_op`: the database studio's requests on a tenant's own database (MariaDB).
//!
//! Every request makes a short-lived account that has rights on that one database,
//! connects as it, does the work and drops it. What a client types therefore runs with
//! the same power as the client's own database user: it cannot reach another database,
//! the server's settings or other accounts. Table and column names are quoted here,
//! values travel as bound parameters, and an update or delete needs the table's whole
//! primary key, so it can only ever touch one row.

use std::{sync::Arc, time::Instant};

use builtpanel_node_protocol::{
    AgentError, DbEngine, Operation, Output,
    studio::{
        Assignment, ColumnInfo, Filter, FilterOp, Grid, IndexInfo, KeyPart, MAX_CELL_BYTES,
        MAX_PAGE_ROWS, MAX_QUERY_ROWS, MAX_SQL_BYTES, StudioErrorCode, StudioRequest, StudioResult,
        TableInfo,
    },
};
use mysql_async::{
    Column, Conn, Row, Value,
    consts::ColumnType,
    prelude::{Queryable as _, ToValue},
};

use super::{
    db,
    sqlimport::{failed, random_password, server_words},
    tenant::TenantContext,
};

/// How long an account may live if the agent dies before dropping it (the sweep of the
/// phpMyAdmin accounts takes the same names).
const ACCOUNT_SECS: i64 = 180;
/// Longest a statement may run, in seconds.
const STATEMENT_SECS: u32 = 20;
/// What an answer may hold in all (a frame is 1 MiB, with the envelope around it).
const ANSWER_BUDGET: usize = 700 * 1024;

/// name, type, rows, data size, index size
type TableRow = (String, String, Option<u64>, Option<u64>, Option<u64>);
/// name, type, nullable, key, default, extra
type ColumnRow = (String, String, String, String, Option<String>, String);

fn error(code: StudioErrorCode, message: impl Into<String>) -> StudioResult {
    StudioResult::Error {
        code,
        message: message.into(),
    }
}

fn invalid(message: &str) -> StudioResult {
    error(StudioErrorCode::Invalid, message)
}

fn sql_failed(err: &mysql_async::Error) -> StudioResult {
    error(StudioErrorCode::Sql, server_words(err))
}

/// `studio_op`.
///
/// # Errors
/// [`AgentError`]: the database server is not reachable (retryable). A request that
/// cannot be done is the answer, not an error.
pub async fn studio_op(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::StudioOp {
        database,
        engine,
        request,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    builtpanel_domain::databases::check_identifier(&database).map_err(|e| {
        AgentError::new(
            builtpanel_node_protocol::ErrorCode::InvalidArguments,
            e.to_string(),
        )
    })?;
    let result = match engine {
        DbEngine::Mysql => mysql(&context, &database, request).await?,
        DbEngine::Postgres => super::studio_pg::run(&context, &database, request).await?,
    };
    Ok(Output::Studio { result })
}

async fn mysql(
    context: &TenantContext,
    database: &str,
    request: StudioRequest,
) -> Result<StudioResult, AgentError> {
    if let StudioRequest::Query { sql } = &request
        && (sql.trim().is_empty() || sql.len() > MAX_SQL_BYTES)
    {
        return Ok(invalid("The statement is empty or too long."));
    }
    let (mut conn, account) = open(context, database).await?;
    let result = run(&mut conn, database, request).await;
    let _ = conn.disconnect().await;
    drop_account(context, &account).await;
    Ok(result)
}

/// The temporary account, and a connection as it.
async fn open(context: &TenantContext, database: &str) -> Result<(Conn, String), AgentError> {
    db::ensure_server(context).await?;
    let password =
        random_password().map_err(|e| failed(&format!("cannot make a password: {}", e.kind())))?;
    let mut raw = [0u8; 6];
    std::io::Read::read_exact(
        &mut std::fs::File::open("/dev/urandom").map_err(io)?,
        &mut raw,
    )
    .map_err(io)?;
    let account = format!(
        "dhpma-{}",
        raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    // Written down first: whatever happens next, the sweep finds it.
    let accounts = super::pma::accounts_dir(context);
    std::fs::create_dir_all(&accounts).map_err(io)?;
    std::fs::write(
        accounts.join(&account),
        (super::pma::now() + ACCOUNT_SECS).to_string(),
    )
    .map_err(io)?;
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
        let privileges = builtpanel_domain::databases::Access::All
            .privileges()
            .join(", ");
        for statement in [
            format!("DROP USER IF EXISTS '{account}'@'%'"),
            format!(
                "CREATE USER '{account}'@'%' IDENTIFIED BY PASSWORD '{hash}' WITH MAX_USER_CONNECTIONS 2"
            ),
            format!("GRANT {privileges} ON `{database}`.* TO '{account}'@'%'"),
        ] {
            admin
                .query_drop(statement)
                .await
                .map_err(|e| failed(&server_words(&e)))?;
        }
        let _ = admin.disconnect().await;
    }
    let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(db::server_port(context)?)
        .user(Some(account.as_str()))
        .pass(Some(password.as_str()))
        .db_name(Some(database))
        .into();
    let mut conn = Conn::new(opts)
        .await
        .map_err(|e| failed(&server_words(&e)).retryable())?;
    conn.query_drop(format!("SET SESSION max_statement_time = {STATEMENT_SECS}"))
        .await
        .map_err(|e| failed(&server_words(&e)))?;
    Ok((conn, account))
}

fn io(e: std::io::Error) -> AgentError {
    failed(&format!("cannot prepare the session: {}", e.kind()))
}

async fn drop_account(context: &TenantContext, account: &str) {
    if let Ok(mut admin) = db::root_connection(context).await {
        let _ = admin.query_drop(format!("KILL USER '{account}'@'%'")).await;
        if admin
            .query_drop(format!("DROP USER IF EXISTS '{account}'@'%'"))
            .await
            .is_ok()
        {
            let _ = std::fs::remove_file(super::pma::accounts_dir(context).join(account));
        }
        let _ = admin.disconnect().await;
    }
}

/// A name the server will take quoted: not empty, not long, nothing a quote could not hold.
fn quote(name: &str) -> Result<String, StudioResult> {
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(invalid("A table or column name is not valid."));
    }
    Ok(format!("`{}`", name.replace('`', "``")))
}

async fn run(conn: &mut Conn, database: &str, request: StudioRequest) -> StudioResult {
    match request {
        StudioRequest::Tables => tables(conn, database).await,
        StudioRequest::Describe { table } => describe(conn, database, &table).await,
        StudioRequest::Rows {
            table,
            offset,
            limit,
            order_by,
            descending,
            filters,
        } => {
            rows(
                conn,
                &table,
                offset,
                limit,
                order_by.as_deref(),
                descending,
                &filters,
            )
            .await
        }
        StudioRequest::Query { sql } => query(conn, &sql).await,
        StudioRequest::InsertRow { table, values } => insert(conn, &table, &values).await,
        StudioRequest::UpdateRow { table, key, values } => {
            update(conn, database, &table, &key, &values).await
        }
        StudioRequest::DeleteRow { table, key } => delete(conn, database, &table, &key).await,
    }
}

async fn tables(conn: &mut Conn, database: &str) -> StudioResult {
    let found: Result<Vec<TableRow>, _> = conn
        .exec(
            "SELECT TABLE_NAME, TABLE_TYPE, TABLE_ROWS, DATA_LENGTH, INDEX_LENGTH \
             FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME",
            (database,),
        )
        .await;
    match found {
        Ok(list) => StudioResult::Tables {
            tables: list
                .into_iter()
                .map(|(name, kind, rows, data, index)| TableInfo {
                    name,
                    kind: if kind == "VIEW" { "view" } else { "table" }.to_owned(),
                    rows: rows.unwrap_or(0),
                    size_bytes: data.unwrap_or(0).saturating_add(index.unwrap_or(0)),
                })
                .collect(),
        },
        Err(e) => sql_failed(&e),
    }
}

async fn describe(conn: &mut Conn, database: &str, table: &str) -> StudioResult {
    let quoted = match quote(table) {
        Ok(q) => q,
        Err(e) => return e,
    };
    let columns: Result<Vec<ColumnRow>, _> = conn
        .exec(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, EXTRA \
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
             ORDER BY ORDINAL_POSITION",
            (database, table),
        )
        .await;
    let columns = match columns {
        Ok(c) if c.is_empty() => return error(StudioErrorCode::NotFound, "No such table."),
        Ok(c) => c,
        Err(e) => return sql_failed(&e),
    };
    let stats: Result<Vec<(String, u8, String)>, _> = conn
        .exec(
            "SELECT INDEX_NAME, NON_UNIQUE, COLUMN_NAME FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY INDEX_NAME, SEQ_IN_INDEX",
            (database, table),
        )
        .await;
    let stats = match stats {
        Ok(s) => s,
        Err(e) => return sql_failed(&e),
    };
    let mut indexes: Vec<IndexInfo> = Vec::new();
    for (name, non_unique, column) in stats {
        match indexes.last_mut() {
            Some(last) if last.name == name => last.columns.push(column),
            _ => indexes.push(IndexInfo {
                name,
                columns: vec![column],
                unique: non_unique == 0,
            }),
        }
    }
    let create: Result<Option<(String, String)>, _> = conn
        .query_first(format!("SHOW CREATE TABLE {quoted}"))
        .await;
    let create_sql = match create {
        Ok(Some((_, sql))) => sql,
        Ok(None) => String::new(),
        Err(e) => return sql_failed(&e),
    };
    StudioResult::Described {
        columns: columns
            .into_iter()
            .map(
                |(name, data_type, nullable, key, default, extra)| ColumnInfo {
                    name,
                    data_type,
                    nullable: nullable == "YES",
                    primary_key: key == "PRI",
                    default,
                    generated: extra.contains("auto_increment") || extra.contains("GENERATED"),
                },
            )
            .collect(),
        indexes,
        create_sql,
    }
}

/// One cell as text (`None` is `NULL`), cut when it is long.
fn cell(column: &Column, value: Value, cut: &mut bool) -> Option<String> {
    let text = match value {
        Value::NULL => return None,
        Value::Bytes(bytes) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(e) => {
                let bytes = e.into_bytes();
                let shown: String = bytes
                    .iter()
                    .take(MAX_CELL_BYTES / 2)
                    .map(|b| format!("{b:02x}"))
                    .collect();
                if bytes.len() > MAX_CELL_BYTES / 2 {
                    *cut = true;
                }
                return Some(format!("0x{shown}"));
            }
        },
        Value::Int(n) => n.to_string(),
        Value::UInt(n) => n.to_string(),
        Value::Float(n) => n.to_string(),
        Value::Double(n) => n.to_string(),
        Value::Date(y, mo, d, h, mi, s, us) => {
            if column.column_type() == ColumnType::MYSQL_TYPE_DATE {
                format!("{y:04}-{mo:02}-{d:02}")
            } else if us == 0 {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
            } else {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{us:06}")
            }
        }
        Value::Time(negative, days, h, mi, s, us) => {
            let hours = u32::from(h) + days * 24;
            let sign = if negative { "-" } else { "" };
            if us == 0 {
                format!("{sign}{hours:02}:{mi:02}:{s:02}")
            } else {
                format!("{sign}{hours:02}:{mi:02}:{s:02}.{us:06}")
            }
        }
    };
    if text.len() > MAX_CELL_BYTES {
        *cut = true;
        let mut end = MAX_CELL_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        return Some(text[..end].to_owned());
    }
    Some(text)
}

/// Rows to a grid, within the answer's budget.
fn grid(columns: &[Column], rows: Vec<Row>, more: bool) -> Grid {
    let mut out = Vec::new();
    let mut spent = 0usize;
    let mut truncated = more;
    for row in rows {
        let mut line = Vec::with_capacity(columns.len());
        for (column, value) in columns.iter().zip(row.unwrap()) {
            let item = cell(column, value, &mut truncated);
            spent += item.as_ref().map_or(1, String::len) + 4;
            line.push(item);
        }
        if spent > ANSWER_BUDGET {
            truncated = true;
            break;
        }
        out.push(line);
    }
    Grid {
        columns: columns.iter().map(|c| c.name_str().into_owned()).collect(),
        rows: out,
        truncated,
    }
}

/// `WHERE` for the filters, and its values.
fn conditions(filters: &[Filter]) -> Result<(String, Vec<Value>), StudioResult> {
    let mut parts = Vec::new();
    let mut values = Vec::new();
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
            parts.push(format!("{column} {sql} ?"));
            values.push(value.to_value());
        } else {
            parts.push(format!("{column} {sql}"));
        }
    }
    let clause = if parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", parts.join(" AND "))
    };
    Ok((clause, values))
}

async fn rows(
    conn: &mut Conn,
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
    let (clause, values) = match conditions(filters) {
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
    let total: Result<Option<u64>, _> = conn
        .exec_first(
            format!("SELECT COUNT(*) FROM {quoted}{clause}"),
            values.clone(),
        )
        .await;
    let total = match total {
        Ok(t) => t.unwrap_or(0),
        Err(e) => return sql_failed(&e),
    };
    let page = conn
        .exec_iter(
            format!("SELECT * FROM {quoted}{clause}{order} LIMIT {limit} OFFSET {offset}"),
            values,
        )
        .await;
    let mut page = match page {
        Ok(p) => p,
        Err(e) => return sql_failed(&e),
    };
    let columns: Vec<Column> = page.columns_ref().to_vec();
    let collected: Result<Vec<Row>, _> = page.collect().await;
    match collected {
        Ok(found) => StudioResult::Rows {
            grid: grid(&columns, found, false),
            total,
        },
        Err(e) => sql_failed(&e),
    }
}

async fn query(conn: &mut Conn, sql: &str) -> StudioResult {
    let started = Instant::now();
    let mut result = match conn.query_iter(sql).await {
        Ok(r) => r,
        Err(e) => return sql_failed(&e),
    };
    let columns: Vec<Column> = result.columns_ref().to_vec();
    let mut found = Vec::new();
    let mut more = false;
    if !columns.is_empty() {
        loop {
            match result.next().await {
                Ok(Some(row)) => {
                    if found.len() >= MAX_QUERY_ROWS {
                        more = true;
                        break;
                    }
                    found.push(row);
                }
                Ok(None) => break,
                Err(e) => return sql_failed(&e),
            }
        }
    }
    let affected = result.affected_rows();
    if let Err(e) = result.drop_result().await {
        return sql_failed(&e);
    }
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    StudioResult::Query {
        grid: (!columns.is_empty()).then(|| grid(&columns, found, more)),
        affected,
        elapsed_ms,
    }
}

async fn insert(conn: &mut Conn, table: &str, values: &[Assignment]) -> StudioResult {
    let quoted = match quote(table) {
        Ok(q) => q,
        Err(e) => return e,
    };
    let (statement, params) = if values.is_empty() {
        (format!("INSERT INTO {quoted} () VALUES ()"), Vec::new())
    } else {
        let mut names = Vec::new();
        for item in values {
            match quote(&item.column) {
                Ok(q) => names.push(q),
                Err(e) => return e,
            }
        }
        let marks = vec!["?"; values.len()].join(", ");
        (
            format!(
                "INSERT INTO {quoted} ({}) VALUES ({marks})",
                names.join(", ")
            ),
            values
                .iter()
                .map(|a| a.value.as_ref().map_or(Value::NULL, ToValue::to_value))
                .collect(),
        )
    };
    run_change(conn, statement, params).await
}

async fn run_change(conn: &mut Conn, statement: String, params: Vec<Value>) -> StudioResult {
    match conn.exec_iter(statement, params).await {
        Ok(result) => {
            let affected = result.affected_rows();
            match result.drop_result().await {
                Ok(()) => StudioResult::Changed { affected },
                Err(e) => sql_failed(&e),
            }
        }
        Err(e) => sql_failed(&e),
    }
}

/// `WHERE` for a primary key, after checking that `key` is the table's whole key.
async fn key_clause(
    conn: &mut Conn,
    database: &str,
    table: &str,
    key: &[KeyPart],
) -> Result<(String, Vec<Value>), StudioResult> {
    let primary: Result<Vec<String>, _> = conn
        .exec(
            "SELECT COLUMN_NAME FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND COLUMN_KEY = 'PRI'",
            (database, table),
        )
        .await;
    let mut primary = primary.map_err(|e| sql_failed(&e))?;
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
    let mut values = Vec::new();
    for part in key {
        parts.push(format!("{} = ?", quote(&part.column)?));
        values.push(part.value.to_value());
    }
    Ok((parts.join(" AND "), values))
}

async fn update(
    conn: &mut Conn,
    database: &str,
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
    let (clause, key_values) = match key_clause(conn, database, table, key).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut sets = Vec::new();
    let mut params = Vec::new();
    for item in values {
        match quote(&item.column) {
            Ok(q) => sets.push(format!("{q} = ?")),
            Err(e) => return e,
        }
        params.push(item.value.as_ref().map_or(Value::NULL, ToValue::to_value));
    }
    params.extend(key_values);
    run_change(
        conn,
        format!(
            "UPDATE {quoted} SET {} WHERE {clause} LIMIT 1",
            sets.join(", ")
        ),
        params,
    )
    .await
}

async fn delete(conn: &mut Conn, database: &str, table: &str, key: &[KeyPart]) -> StudioResult {
    let quoted = match quote(table) {
        Ok(q) => q,
        Err(e) => return e,
    };
    let (clause, params) = match key_clause(conn, database, table, key).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    run_change(
        conn,
        format!("DELETE FROM {quoted} WHERE {clause} LIMIT 1"),
        params,
    )
    .await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn names_are_quoted_and_a_backtick_cannot_end_the_quote() {
        assert_eq!(quote("orders").unwrap(), "`orders`");
        assert_eq!(quote("a`b").unwrap(), "`a``b`");
        assert!(quote("").is_err());
        assert!(quote("bad\nname").is_err());
        assert!(quote(&"x".repeat(65)).is_err());
    }

    #[test]
    fn filters_become_placeholders_never_text() {
        let (clause, values) = conditions(&[
            Filter {
                column: "id".into(),
                op: FilterOp::Gt,
                value: Some("1; DROP TABLE x".into()),
            },
            Filter {
                column: "note".into(),
                op: FilterOp::IsNull,
                value: None,
            },
        ])
        .unwrap();
        assert_eq!(clause, " WHERE `id` > ? AND `note` IS NULL");
        assert_eq!(values.len(), 1);
        assert!(!clause.contains("DROP"));
        assert!(
            conditions(&[Filter {
                column: "id".into(),
                op: FilterOp::Eq,
                value: None
            }])
            .is_err()
        );
    }
}
