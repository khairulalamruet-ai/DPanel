//! Second-factor persistence (`account_mfa`, `mfa_backup_codes`).
//!
//! Secrets arrive here already encrypted; this module never sees plaintext
//! TOTP secrets or backup codes (only SHA-256 hashes of the latter).

use sqlx::{PgPool, types::Uuid};

/// A stored TOTP factor.
#[derive(Clone)]
pub struct TotpFactor {
    /// Encrypted secret (see `builtpanel_auth_core::secretbox`).
    pub secret_enc: Vec<u8>,
    /// Whether enrolment was confirmed; unconfirmed factors do not gate login.
    pub confirmed: bool,
    /// Newest accepted time step.
    pub last_used_step: Option<i64>,
}

impl std::fmt::Debug for TotpFactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TotpFactor")
            .field("confirmed", &self.confirmed)
            .field("last_used_step", &self.last_used_step)
            .finish_non_exhaustive()
    }
}

/// Loads the account's TOTP factor, confirmed or pending.
///
/// # Errors
/// Database errors.
pub async fn find_totp(pool: &PgPool, account_id: Uuid) -> Result<Option<TotpFactor>, sqlx::Error> {
    sqlx::query_as!(
        TotpFactor,
        r#"SELECT secret_enc, confirmed_at IS NOT NULL AS "confirmed!", last_used_step
           FROM account_mfa WHERE account_id = $1 AND kind = 'totp'"#,
        account_id
    )
    .fetch_optional(pool)
    .await
}

/// Whether the account has a confirmed TOTP factor.
///
/// # Errors
/// Database errors.
pub async fn totp_enabled(pool: &PgPool, account_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM account_mfa
                          WHERE account_id = $1 AND kind = 'totp' AND confirmed_at IS NOT NULL) AS "enabled!""#,
        account_id
    )
    .fetch_one(pool)
    .await
}

/// Stores a new pending TOTP secret, replacing an earlier pending one.
/// Returns `false` (and changes nothing) when a confirmed factor exists.
///
/// # Errors
/// Database errors.
pub async fn begin_totp_enrollment(
    pool: &PgPool,
    account_id: Uuid,
    secret_enc: &[u8],
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"INSERT INTO account_mfa (account_id, kind, secret_enc)
           VALUES ($1, 'totp', $2)
           ON CONFLICT (account_id, kind) DO UPDATE
               SET secret_enc = EXCLUDED.secret_enc, last_used_step = NULL
               WHERE account_mfa.confirmed_at IS NULL"#,
        account_id,
        secret_enc
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Confirms a pending factor, records the step of the confirming code and
/// replaces the account's backup codes, all in one transaction. Returns
/// `false` when there was no pending factor (nothing is changed).
///
/// # Errors
/// Database errors.
pub async fn confirm_totp(
    pool: &PgPool,
    account_id: Uuid,
    step: i64,
    backup_code_hashes: &[[u8; 32]],
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let confirmed = sqlx::query!(
        r#"UPDATE account_mfa SET confirmed_at = now(), last_used_step = $2
           WHERE account_id = $1 AND kind = 'totp' AND confirmed_at IS NULL"#,
        account_id,
        step
    )
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    if !confirmed {
        return Ok(false);
    }
    sqlx::query!(
        "DELETE FROM mfa_backup_codes WHERE account_id = $1",
        account_id
    )
    .execute(&mut *tx)
    .await?;
    let hashes: Vec<Vec<u8>> = backup_code_hashes.iter().map(|h| h.to_vec()).collect();
    sqlx::query!(
        "INSERT INTO mfa_backup_codes (account_id, code_sha256) SELECT $1, unnest($2::bytea[])",
        account_id,
        &hashes
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Marks a TOTP time step as used. Returns `false` when that step (or a newer
/// one) was already accepted — the code is a replay and must be refused. The
/// comparison and update are a single statement, so two concurrent requests
/// with the same code cannot both succeed.
///
/// # Errors
/// Database errors.
pub async fn accept_totp_step(
    pool: &PgPool,
    account_id: Uuid,
    step: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"UPDATE account_mfa SET last_used_step = $2
           WHERE account_id = $1 AND kind = 'totp' AND confirmed_at IS NOT NULL
             AND (last_used_step IS NULL OR last_used_step < $2)"#,
        account_id,
        step
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Spends a backup code. Returns `false` when it does not exist or was
/// already used; atomic, so a code can never be spent twice.
///
/// # Errors
/// Database errors.
pub async fn consume_backup_code(
    pool: &PgPool,
    account_id: Uuid,
    code_sha256: &[u8; 32],
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"UPDATE mfa_backup_codes SET used_at = now()
           WHERE account_id = $1 AND code_sha256 = $2 AND used_at IS NULL"#,
        account_id,
        &code_sha256[..]
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Number of backup codes still unused.
///
/// # Errors
/// Database errors.
pub async fn unused_backup_codes(pool: &PgPool, account_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM mfa_backup_codes WHERE account_id = $1 AND used_at IS NULL"#,
        account_id
    )
    .fetch_one(pool)
    .await
}

/// Removes the TOTP factor (confirmed or pending) and all backup codes.
/// Returns whether a factor existed.
///
/// # Errors
/// Database errors.
pub async fn remove_totp(pool: &PgPool, account_id: Uuid) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let removed = sqlx::query!(
        "DELETE FROM account_mfa WHERE account_id = $1 AND kind = 'totp'",
        account_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    sqlx::query!(
        "DELETE FROM mfa_backup_codes WHERE account_id = $1",
        account_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(removed)
}
