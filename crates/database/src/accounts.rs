//! Account persistence.

use builtpanel_domain::{AccountKind, Email, Username};
use sqlx::{PgConnection, PgPool, types::Uuid};

/// Mirrors the `account_kind` PostgreSQL enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, serde::Serialize)]
#[sqlx(type_name = "account_kind", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum DbAccountKind {
    /// Panel operator.
    Admin,
    /// Reseller.
    Reseller,
    /// Hosting customer.
    Client,
}

impl From<AccountKind> for DbAccountKind {
    fn from(k: AccountKind) -> Self {
        match k {
            AccountKind::Admin => Self::Admin,
            AccountKind::Reseller => Self::Reseller,
            AccountKind::Client => Self::Client,
        }
    }
}

/// Input for [`insert_account`]. The password must already be hashed.
#[derive(Debug, Clone)]
pub struct NewAccount<'a> {
    /// Account kind.
    pub kind: AccountKind,
    /// Validated username.
    pub username: &'a Username,
    /// Validated email.
    pub email: &'a Email,
    /// Argon2id PHC string.
    pub password_hash: &'a str,
}

/// Why an account could not be created.
#[derive(Debug, thiserror::Error)]
pub enum InsertAccountError {
    /// Another account already uses this username.
    #[error("username is already taken")]
    UsernameTaken,
    /// Another account already uses this email.
    #[error("email is already in use")]
    EmailTaken,
    /// Any other database error.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Inserts an account and returns its id.
///
/// # Errors
/// [`InsertAccountError::UsernameTaken`] / [`InsertAccountError::EmailTaken`] on
/// unique violations, otherwise the database error.
pub async fn insert_account(
    pool: &PgPool,
    new: &NewAccount<'_>,
) -> Result<Uuid, InsertAccountError> {
    let mut conn = pool.acquire().await?;
    insert_account_in(&mut conn, new).await
}

/// Like [`insert_account`], on a plain connection so it can run inside a
/// transaction (`&mut *tx`).
///
/// # Errors
/// Same as [`insert_account`].
pub async fn insert_account_in(
    conn: &mut PgConnection,
    new: &NewAccount<'_>,
) -> Result<Uuid, InsertAccountError> {
    let kind = DbAccountKind::from(new.kind);
    sqlx::query_scalar!(
        r#"INSERT INTO accounts (kind, username, email, password_hash)
           VALUES ($1, $2, $3, $4)
           RETURNING id"#,
        kind as DbAccountKind,
        new.username.as_str(),
        new.email.as_str(),
        new.password_hash,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(
        |err| match err.as_database_error().and_then(|e| e.constraint()) {
            Some("accounts_username_key") => InsertAccountError::UsernameTaken,
            Some("accounts_email_key") => InsertAccountError::EmailTaken,
            _ => InsertAccountError::Database(err),
        },
    )
}

/// Number of accounts of the given kind (used to detect first-run bootstrap).
///
/// # Errors
/// Database errors.
pub async fn count_accounts(pool: &PgPool, kind: AccountKind) -> Result<i64, sqlx::Error> {
    let kind = DbAccountKind::from(kind);
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM accounts WHERE kind = $1"#,
        kind as DbAccountKind
    )
    .fetch_one(pool)
    .await
}

/// Mirrors the `account_status` PostgreSQL enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, serde::Serialize)]
#[sqlx(type_name = "account_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum DbAccountStatus {
    /// May log in.
    Active,
    /// Temporarily blocked (e.g. unpaid).
    Suspended,
    /// Permanently blocked.
    Disabled,
}

/// Fields needed to authenticate. Contains the password hash: never serialise or log.
pub struct LoginAccount {
    /// Account id.
    pub id: Uuid,
    /// Status.
    pub status: DbAccountStatus,
    /// Argon2id PHC string.
    pub password_hash: String,
}

impl std::fmt::Debug for LoginAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginAccount")
            .field("id", &self.id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// Public profile of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountProfile {
    /// Account id.
    pub id: Uuid,
    /// Kind.
    pub kind: DbAccountKind,
    /// Username.
    pub username: String,
    /// Email.
    pub email: String,
    /// Status.
    pub status: DbAccountStatus,
    /// UI locale.
    pub locale: String,
}

/// Finds an account by normalised username or email for login.
///
/// # Errors
/// Database errors.
pub async fn find_for_login(
    pool: &PgPool,
    username: Option<&Username>,
    email: Option<&Email>,
) -> Result<Option<LoginAccount>, sqlx::Error> {
    sqlx::query_as!(
        LoginAccount,
        r#"SELECT id, effective_account_status(id) AS "status!: DbAccountStatus", password_hash
           FROM accounts
           WHERE ($1::text IS NOT NULL AND username = $1)
              OR ($2::text IS NOT NULL AND email = $2)
           LIMIT 1"#,
        username.map(Username::as_str),
        email.map(Email::as_str),
    )
    .fetch_optional(pool)
    .await
}

/// Loads the public profile of an account.
///
/// # Errors
/// Database errors.
pub async fn find_profile(pool: &PgPool, id: Uuid) -> Result<Option<AccountProfile>, sqlx::Error> {
    sqlx::query_as!(
        AccountProfile,
        r#"SELECT id, kind AS "kind: DbAccountKind", username, email,
                  effective_account_status(id) AS "status!: DbAccountStatus", locale
           FROM accounts WHERE id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await
}

/// Loads an account's status and password hash by id, for re-authenticating
/// an already signed-in user (enabling/disabling 2FA, sudo mode).
///
/// # Errors
/// Database errors.
pub async fn find_for_reauth(pool: &PgPool, id: Uuid) -> Result<Option<LoginAccount>, sqlx::Error> {
    sqlx::query_as!(
        LoginAccount,
        r#"SELECT id, effective_account_status(id) AS "status!: DbAccountStatus", password_hash
           FROM accounts WHERE id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await
}

/// The `password_hash` of the administrator named `root`: it has no panel password, the
/// server's own root password is checked instead (by the `builtpanel-root-auth` helper).
pub const SYSTEM_ROOT_PASSWORD: &str = "!system:root";

/// The administrator `root` that logs in with the server's root password, if it is set up.
///
/// # Errors
/// Database errors.
pub async fn find_system_root(pool: &PgPool) -> Result<Option<LoginAccount>, sqlx::Error> {
    sqlx::query_as!(
        LoginAccount,
        r#"SELECT id, effective_account_status(id) AS "status!: DbAccountStatus", password_hash
           FROM accounts
           WHERE username = 'root' AND kind = 'admin' AND password_hash = '!system:root'"#,
    )
    .fetch_optional(pool)
    .await
}

/// Creates the administrator `root` (or turns it back on) with no panel password of its own.
/// Returns its id and whether it was created now.
///
/// # Errors
/// [`InsertAccountError::EmailTaken`] when another account uses the email; database errors.
pub async fn enable_system_root(
    pool: &PgPool,
    email: &Email,
) -> Result<(Uuid, bool), InsertAccountError> {
    let row = sqlx::query!(
        r#"INSERT INTO accounts (kind, username, email, password_hash)
           VALUES ('admin', 'root', $1, '!system:root')
           ON CONFLICT (username) DO UPDATE
               SET password_hash = '!system:root', status = 'active'
               WHERE accounts.kind = 'admin'
           RETURNING id, (xmax = 0) AS "created!""#,
        email.as_str(),
    )
    .fetch_optional(pool)
    .await
    .map_err(
        |err| match err.as_database_error().and_then(|e| e.constraint()) {
            Some("accounts_email_key") => InsertAccountError::EmailTaken,
            _ => InsertAccountError::Database(err),
        },
    )?;
    row.map(|r| (r.id, r.created))
        .ok_or(InsertAccountError::UsernameTaken)
}

/// Turns off the login with the server's root password. Returns whether there was one.
///
/// # Errors
/// Database errors.
pub async fn disable_system_root(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE accounts SET status = 'disabled'
         WHERE username = 'root' AND kind = 'admin' AND password_hash = '!system:root'"
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Replaces the stored password hash (used when upgrading hash parameters).
///
/// # Errors
/// Database errors.
pub async fn update_password_hash(
    pool: &PgPool,
    id: Uuid,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE accounts SET password_hash = $2 WHERE id = $1",
        id,
        password_hash
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Outcome recorded in `login_events.reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginOutcome {
    /// Logged in.
    Ok,
    /// Unknown user or wrong password.
    BadCredentials,
    /// Correct password, account suspended.
    AccountSuspended,
    /// Correct password, account disabled.
    AccountDisabled,
    /// Rejected by rate limiting.
    RateLimited,
    /// Rejected by lockout.
    LockedOut,
    /// Password accepted; a second factor is now required.
    MfaRequired,
    /// Second factor wrong, replayed or expired.
    MfaFailed,
}

impl LoginOutcome {
    /// Value stored in `login_events.reason`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::BadCredentials => "bad_credentials",
            Self::AccountSuspended => "account_suspended",
            Self::AccountDisabled => "account_disabled",
            Self::RateLimited => "rate_limited",
            Self::LockedOut => "locked_out",
            Self::MfaRequired => "mfa_required",
            Self::MfaFailed => "mfa_failed",
        }
    }
}

/// One login attempt to record.
#[derive(Debug, Clone)]
pub struct LoginEvent<'a> {
    /// Matched account, if any.
    pub account_id: Option<Uuid>,
    /// What the client typed (truncated to 254 characters).
    pub attempted: &'a str,
    /// Client IP.
    pub ip: std::net::IpAddr,
    /// Client user agent (truncated to 512 characters).
    pub user_agent: Option<&'a str>,
    /// Outcome.
    pub outcome: LoginOutcome,
}

/// Records a login attempt.
///
/// # Errors
/// Database errors.
pub async fn insert_login_event(pool: &PgPool, ev: &LoginEvent<'_>) -> Result<(), sqlx::Error> {
    let attempted: String = ev.attempted.chars().take(254).collect();
    let user_agent: Option<String> = ev.user_agent.map(|ua| ua.chars().take(512).collect());
    sqlx::query!(
        r#"INSERT INTO login_events (account_id, username_attempted, ip, user_agent, success, reason)
           VALUES ($1, $2, $3::text::inet, $4, $5, $6)"#,
        ev.account_id,
        attempted,
        ev.ip.to_string(),
        user_agent,
        matches!(ev.outcome, LoginOutcome::Ok),
        ev.outcome.as_str(),
    )
    .execute(pool)
    .await?;
    Ok(())
}
