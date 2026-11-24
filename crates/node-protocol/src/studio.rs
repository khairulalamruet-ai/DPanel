//! The database studio's requests and answers (a web client for a tenant's own database).
//!
//! One request is done with a short-lived database account that has rights on that one
//! database and nothing else, so what a client types can never leave its own database.
//! Table and column names are identifiers the agent checks and quotes; values travel as
//! text (`None` is SQL `NULL`), and binary values arrive as `0x...` hex.

use serde::{Deserialize, Serialize};

/// Most rows one page of a table gives.
pub const MAX_PAGE_ROWS: u32 = 500;
/// Most rows a query's answer keeps (more are cut and flagged `truncated`).
pub const MAX_QUERY_ROWS: usize = 1000;
/// Longest SQL text one request carries.
pub const MAX_SQL_BYTES: usize = 64 * 1024;
/// Most text one value keeps in an answer (the rest is cut and flagged).
pub const MAX_CELL_BYTES: usize = 8 * 1024;

/// How a column of a table is compared with a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `LIKE` (the value carries its own `%`)
    Like,
    /// `IS NULL` (no value)
    IsNull,
    /// `IS NOT NULL` (no value)
    NotNull,
}

/// One condition of a table view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Filter {
    /// The column.
    pub column: String,
    /// How to compare.
    pub op: FilterOp,
    /// The value (not used by `is_null` and `not_null`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// A column set to a value (`None` is `NULL`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Assignment {
    /// The column.
    pub column: String,
    /// The value as text.
    pub value: Option<String>,
}

/// One part of a primary key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KeyPart {
    /// The column.
    pub column: String,
    /// Its value as text.
    pub value: String,
}

/// What a studio asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StudioRequest {
    /// The tables and views of the database.
    Tables,
    /// The columns, keys and indexes of one table.
    Describe {
        /// The table.
        table: String,
    },
    /// A page of a table.
    Rows {
        /// The table.
        table: String,
        /// Rows to skip.
        offset: u32,
        /// Rows to give (1..=[`MAX_PAGE_ROWS`]).
        limit: u32,
        /// Sort by this column.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        order_by: Option<String>,
        /// Descending when sorted.
        #[serde(default)]
        descending: bool,
        /// Conditions, all of which must hold.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        filters: Vec<Filter>,
    },
    /// Runs one SQL statement the client typed.
    Query {
        /// The statement.
        sql: String,
    },
    /// Adds a row (columns not named take their default).
    InsertRow {
        /// The table.
        table: String,
        /// The values to put in.
        values: Vec<Assignment>,
    },
    /// Changes the one row whose primary key is `key`.
    UpdateRow {
        /// The table.
        table: String,
        /// The primary key.
        key: Vec<KeyPart>,
        /// What to set.
        values: Vec<Assignment>,
    },
    /// Deletes the one row whose primary key is `key`.
    DeleteRow {
        /// The table.
        table: String,
        /// The primary key.
        key: Vec<KeyPart>,
    },
}

/// A table or view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TableInfo {
    /// Its name.
    pub name: String,
    /// `table` or `view`.
    pub kind: String,
    /// About how many rows (an estimate the server keeps).
    pub rows: u64,
    /// Data and index size, in bytes.
    pub size_bytes: u64,
}

/// One column of a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ColumnInfo {
    /// Its name.
    pub name: String,
    /// Its type as the server writes it (`varchar(255)`).
    pub data_type: String,
    /// May hold `NULL`.
    pub nullable: bool,
    /// Part of the primary key.
    pub primary_key: bool,
    /// The default, as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Filled in by the server (auto increment, generated).
    #[serde(default)]
    pub generated: bool,
}

/// One index of a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IndexInfo {
    /// Its name.
    pub name: String,
    /// Its columns, in order.
    pub columns: Vec<String>,
    /// No two rows may share its values.
    pub unique: bool,
}

/// A result set: column names and rows of text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Grid {
    /// The column names.
    pub columns: Vec<String>,
    /// The rows; `None` is `NULL`.
    pub rows: Vec<Vec<Option<String>>>,
    /// More rows existed than were kept, or a value was cut.
    #[serde(default)]
    pub truncated: bool,
}

/// Why a request failed (it is the answer, not a transport error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum StudioErrorCode {
    /// The request is not valid (a bad name, no primary key, too long).
    Invalid,
    /// The server refused the statement (its words are in the message).
    Sql,
    /// The table or column is not there.
    NotFound,
    /// The answer would not fit.
    TooLarge,
}

/// What a studio request came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StudioResult {
    /// The tables.
    Tables {
        /// Them, by name.
        tables: Vec<TableInfo>,
    },
    /// A table's structure.
    Described {
        /// The columns, in order.
        columns: Vec<ColumnInfo>,
        /// The indexes.
        indexes: Vec<IndexInfo>,
        /// The statement that makes the table.
        create_sql: String,
    },
    /// A page of rows.
    Rows {
        /// The page.
        grid: Grid,
        /// How many rows match (counted, not estimated).
        total: u64,
    },
    /// A statement's outcome.
    Query {
        /// Rows, when the statement gave a result set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grid: Option<Grid>,
        /// Rows it changed, when it did not give a result set.
        affected: u64,
        /// How long it ran, in milliseconds.
        elapsed_ms: u64,
    },
    /// A row was added, changed or deleted.
    Changed {
        /// How many rows (0 when the key matched none).
        affected: u64,
    },
    /// It could not be done.
    Error {
        /// Which kind.
        code: StudioErrorCode,
        /// In words (the server's, for `sql`).
        message: String,
    },
}
