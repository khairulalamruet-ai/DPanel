//! Client accounts as the administrator sees them: an account together with its
//! tenant, plan and node (T04.12).

use builtpanel_domain::{Email, TenantStatus};
use sqlx::{PgConnection, PgPool, types::Uuid};

/// One client account with what it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// Account id (the user's id everywhere in the API).
    pub id: Uuid,
    /// Login name.
    pub username: String,
    /// Email address.
    pub email: String,
    /// Interface language.
    pub locale: String,
    /// `active`, `suspended` or `disabled`.
    pub account_status: String,
    /// Creation time (Unix seconds).
    pub created_at: i64,
    /// Its tenant, if it has one.
    pub tenant: Option<UserTenant>,
}

/// The tenant of a [`User`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTenant {
    /// Tenant id.
    pub id: Uuid,
    /// System user name on the node.
    pub username: String,
    /// Life-cycle status.
    pub status: TenantStatus,
    /// Plan id.
    pub plan_id: Uuid,
    /// Plan name.
    pub plan_name: String,
    /// Node id.
    pub node_id: Uuid,
    /// Node name.
    pub node_name: String,
    /// Numeric user id on the node.
    pub uid: i32,
    /// When the data will be purged (Unix seconds), once deleted.
    pub purge_after: Option<i64>,
}

/// What to look for in [`list`].
#[derive(Debug, Clone, Default)]
pub struct Filter<'a> {
    /// Tenant status.
    pub status: Option<TenantStatus>,
    /// Plan.
    pub plan_id: Option<Uuid>,
    /// Node.
    pub node_id: Option<Uuid>,
    /// Text to find in the login name, email or system name.
    pub search: Option<&'a str>,
}

struct Row {
    id: Uuid,
    username: String,
    email: String,
    locale: String,
    account_status: String,
    created_at: i64,
    tenant_id: Option<Uuid>,
    tenant_username: Option<String>,
    tenant_status: Option<String>,
    plan_id: Option<Uuid>,
    plan_name: Option<String>,
    node_id: Option<Uuid>,
    node_name: Option<String>,
    uid: Option<i32>,
    purge_after: Option<i64>,
}

impl From<Row> for User {
    fn from(r: Row) -> Self {
        let tenant = match (r.tenant_id, r.tenant_username, r.plan_id, r.node_id, r.uid) {
            (Some(id), Some(username), Some(plan_id), Some(node_id), Some(uid)) => {
                Some(UserTenant {
                    id,
                    username,
                    status: r
                        .tenant_status
                        .as_deref()
                        .and_then(|s| TenantStatus::parse(s).ok())
                        .unwrap_or(TenantStatus::Provisioning),
                    plan_id,
                    plan_name: r.plan_name.unwrap_or_default(),
                    node_id,
                    node_name: r.node_name.unwrap_or_default(),
                    uid,
                    purge_after: r.purge_after,
                })
            }
            _ => None,
        };
        Self {
            id: r.id,
            username: r.username,
            email: r.email,
            locale: r.locale,
            account_status: r.account_status,
            created_at: r.created_at,
            tenant,
        }
    }
}

/// Turns user text into a `LIKE` pattern that matches it literally anywhere.
fn like_pattern(text: &str) -> String {
    let mut out = String::from("%");
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Client accounts, newest first, and how many match in all.
///
/// # Errors
/// Database errors.
pub async fn list(
    pool: &PgPool,
    filter: &Filter<'_>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<User>, i64), sqlx::Error> {
    let status = filter.status.map(TenantStatus::as_str);
    let pattern = filter.search.map(like_pattern);
    let total = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!"
           FROM accounts a LEFT JOIN tenants t ON t.owner_account_id = a.id
           WHERE a.kind = 'client'
             AND ($1::text IS NULL OR t.status = $1)
             AND ($2::uuid IS NULL OR t.plan_id = $2)
             AND ($3::uuid IS NULL OR t.node_id = $3)
             AND ($4::text IS NULL OR a.username ILIKE $4 OR a.email ILIKE $4 OR t.username ILIKE $4)"#,
        status,
        filter.plan_id,
        filter.node_id,
        pattern,
    )
    .fetch_one(pool)
    .await?;
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT a.id, a.username, a.email, a.locale, a.status::text AS "account_status!",
                  extract(epoch FROM a.created_at)::bigint AS "created_at!",
                  t.id AS "tenant_id?", t.username AS "tenant_username?", t.status AS "tenant_status?",
                  t.plan_id AS "plan_id?", p.name AS "plan_name?",
                  t.node_id AS "node_id?", n.name AS "node_name?", t.uid AS "uid?",
                  extract(epoch FROM t.purge_after)::bigint AS "purge_after?"
           FROM accounts a
           LEFT JOIN tenants t ON t.owner_account_id = a.id
           LEFT JOIN plans p ON p.id = t.plan_id
           LEFT JOIN nodes n ON n.id = t.node_id
           WHERE a.kind = 'client'
             AND ($1::text IS NULL OR t.status = $1)
             AND ($2::uuid IS NULL OR t.plan_id = $2)
             AND ($3::uuid IS NULL OR t.node_id = $3)
             AND ($4::text IS NULL OR a.username ILIKE $4 OR a.email ILIKE $4 OR t.username ILIKE $4)
           ORDER BY a.created_at DESC, a.id DESC
           LIMIT $5 OFFSET $6"#,
        status,
        filter.plan_id,
        filter.node_id,
        pattern,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await?;
    Ok((rows.into_iter().map(User::from).collect(), total))
}

/// One client account.
///
/// # Errors
/// Database errors.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<User>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        r#"SELECT a.id, a.username, a.email, a.locale, a.status::text AS "account_status!",
                  extract(epoch FROM a.created_at)::bigint AS "created_at!",
                  t.id AS "tenant_id?", t.username AS "tenant_username?", t.status AS "tenant_status?",
                  t.plan_id AS "plan_id?", p.name AS "plan_name?",
                  t.node_id AS "node_id?", n.name AS "node_name?", t.uid AS "uid?",
                  extract(epoch FROM t.purge_after)::bigint AS "purge_after?"
           FROM accounts a
           LEFT JOIN tenants t ON t.owner_account_id = a.id
           LEFT JOIN plans p ON p.id = t.plan_id
           LEFT JOIN nodes n ON n.id = t.node_id
           WHERE a.kind = 'client' AND a.id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(User::from))
}

/// The online node with the fewest tenants (ties: the oldest node), or `None`
/// when no node is online.
///
/// # Errors
/// Database errors.
pub async fn pick_node(conn: &mut PgConnection) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT n.id AS "id!"
           FROM nodes n
           WHERE n.status = 'online'
           ORDER BY (SELECT count(*) FROM tenants t WHERE t.node_id = n.id), n.created_at, n.id
           LIMIT 1"#
    )
    .fetch_optional(conn)
    .await
}

/// Why changing an account failed.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// Another account uses this email.
    #[error("email is already in use")]
    EmailTaken,
    /// Any other database error.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Changes a client account's email and/or language. Returns whether the
/// account exists.
///
/// # Errors
/// [`UpdateError`].
pub async fn update(
    conn: &mut PgConnection,
    id: Uuid,
    email: Option<&Email>,
    locale: Option<&str>,
) -> Result<bool, UpdateError> {
    let done = sqlx::query!(
        r#"UPDATE accounts
           SET email = COALESCE($2, email), locale = COALESCE($3, locale)
           WHERE id = $1 AND kind = 'client'"#,
        id,
        email.map(Email::as_str),
        locale,
    )
    .execute(conn)
    .await
    .map_err(
        |err| match err.as_database_error().and_then(|e| e.constraint()) {
            Some("accounts_email_key") => UpdateError::EmailTaken,
            _ => UpdateError::Database(err),
        },
    )?;
    Ok(done.rows_affected() == 1)
}

/// Sets a client account's status (`active`, `suspended`, `disabled`).
/// Returns whether the account exists.
///
/// # Errors
/// Database errors.
pub async fn set_account_status(
    conn: &mut PgConnection,
    id: Uuid,
    status: &str,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE accounts SET status = $2::text::account_status WHERE id = $1 AND kind = 'client'",
        id,
        status,
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Replaces a client account's password hash. Returns whether it exists.
///
/// # Errors
/// Database errors.
pub async fn set_password_hash(
    conn: &mut PgConnection,
    id: Uuid,
    password_hash: &str,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE accounts SET password_hash = $2, password_changed_at = now()
         WHERE id = $1 AND kind = 'client'",
        id,
        password_hash,
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Moves a tenant to another plan. Returns whether the tenant exists.
///
/// # Errors
/// Database errors (an unknown plan is a foreign-key violation).
pub async fn set_plan(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    plan_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE tenants SET plan_id = $2 WHERE id = $1",
        tenant_id,
        plan_id,
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Whether a node reports that it can enforce storage limits.
///
/// # Errors
/// Database errors.
pub async fn node_enforces_storage_quota(
    conn: &mut PgConnection,
    node_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let found = sqlx::query_scalar!(
        r#"SELECT (capabilities ->> 'storage_quota') IS NOT NULL AS "quota!"
           FROM nodes WHERE id = $1"#,
        node_id,
    )
    .fetch_optional(conn)
    .await?;
    Ok(found.unwrap_or(false))
}
