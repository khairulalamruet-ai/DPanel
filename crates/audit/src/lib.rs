//! Append-only audit log.
//!
//! Every state-changing action and every security-relevant event (login,
//! failed login, logout, permission changes, ...) is written to `audit_logs`.
//!
//! * **Append-only**: the table rejects UPDATE, DELETE and TRUNCATE with a
//!   database error (see the migration); the application role is limited to
//!   SELECT and INSERT.
//! * **Context is automatic**: [`insert`] fills in the correlation ID and the
//!   actor's IP from the request scope set up by
//!   [`builtpanel_http_kit::request_id::correlation_id_layer`] and
//!   [`builtpanel_http_kit::client_ip::client_ip_layer`], so handlers only say
//!   *who did what to which object*.
//! * **Two ways to write**:
//!   * [`AuditWriter::record_or_log`] — best effort. A failure is logged and
//!     the request carries on. Used for authentication events, where the
//!     action (e.g. a failed login) must not be undone by a logging problem.
//!   * [`insert`] on a transaction — for administrative changes: the change
//!     and its audit row commit or roll back together, so there is never a
//!     change without a record.
//! * **Never put secrets in `metadata`**: no passwords, tokens, codes or
//!   key material; the log is readable by administrators and kept for years.
//!
//! The read API for administrators lives in [`api`].

pub mod api;

use std::net::IpAddr;

use builtpanel_http_kit::{client_ip, request_id};
use serde_json::{Value, json};
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

pub use api::{AuditApi, router};

/// Metadata larger than this (serialized) is replaced by a marker, so an
/// oversized value can never make the write fail.
const MAX_METADATA_BYTES: usize = 8000;

/// How an audited action ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The action happened.
    Success,
    /// The action was attempted and failed (wrong password, invalid input, ...).
    Failure,
    /// The action was refused by policy (no permission, lockout, rate limit).
    Denied,
}

impl Outcome {
    /// Database / wire representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Denied => "denied",
        }
    }
}

/// One audit record. Build with [`AuditEntry::new`] and the chained setters.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    action: &'static str,
    outcome: Outcome,
    actor_id: Option<Uuid>,
    actor_ip: Option<IpAddr>,
    target_type: Option<String>,
    target_id: Option<String>,
    tenant_id: Option<Uuid>,
    node_id: Option<Uuid>,
    metadata: Value,
}

impl AuditEntry {
    /// `action` is a dotted key such as `auth.login` or `admin.role_assigned`.
    #[must_use]
    pub fn new(action: &'static str, outcome: Outcome) -> Self {
        debug_assert!(
            builtpanel_rbac::is_valid_key(action),
            "audit action {action:?} must be dotted lowercase segments"
        );
        Self {
            action,
            outcome,
            actor_id: None,
            actor_ip: None,
            target_type: None,
            target_id: None,
            tenant_id: None,
            node_id: None,
            metadata: json!({}),
        }
    }

    /// Who did it. Leave unset for unauthenticated actions.
    #[must_use]
    pub fn actor(mut self, account_id: Uuid) -> Self {
        self.actor_id = Some(account_id);
        self
    }

    /// Overrides the actor's IP; by default it comes from the request scope.
    #[must_use]
    pub fn actor_ip(mut self, ip: IpAddr) -> Self {
        self.actor_ip = Some(ip);
        self
    }

    /// What it was done to, e.g. `("session", id)` or `("role", name)`.
    #[must_use]
    pub fn target(mut self, kind: &str, id: impl ToString) -> Self {
        self.target_type = Some(kind.chars().take(64).collect());
        self.target_id = Some(id.to_string().chars().take(128).collect());
        self
    }

    /// The tenant the action belongs to.
    #[must_use]
    pub fn tenant(mut self, tenant_id: Uuid) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    /// The node the action ran on.
    #[must_use]
    pub fn node(mut self, node_id: Uuid) -> Self {
        self.node_id = Some(node_id);
        self
    }

    /// Extra, non-secret details. Must be a JSON object; anything else is
    /// stored as `{"value": ...}`.
    #[must_use]
    pub fn metadata(mut self, metadata: Value) -> Self {
        self.metadata = if metadata.is_object() {
            metadata
        } else {
            json!({ "value": metadata })
        };
        self
    }

    /// The action key.
    #[must_use]
    pub fn action(&self) -> &'static str {
        self.action
    }

    /// The metadata as stored: an object that is guaranteed to fit.
    fn stored_metadata(&self) -> Value {
        match serde_json::to_string(&self.metadata) {
            Ok(text) if text.len() <= MAX_METADATA_BYTES => self.metadata.clone(),
            _ => json!({ "truncated": true }),
        }
    }
}

/// Writes one entry using any executor — a pool, or a transaction so the
/// audit row commits together with the change it describes.
///
/// # Errors
/// Database errors.
pub async fn insert<'e, E>(executor: E, entry: &AuditEntry) -> Result<(), sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let ip = entry
        .actor_ip
        .or_else(client_ip::current)
        .map(|ip| ip.to_string());
    sqlx::query!(
        r#"INSERT INTO audit_logs
               (actor_id, actor_ip, action, target_type, target_id, tenant_id, node_id,
                outcome, correlation_id, metadata)
           VALUES ($1, $2::text::inet, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        entry.actor_id,
        ip,
        entry.action,
        entry.target_type,
        entry.target_id,
        entry.tenant_id,
        entry.node_id,
        entry.outcome.as_str(),
        request_id::current(),
        entry.stored_metadata(),
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Writes audit entries through the connection pool. Cheap to clone.
#[derive(Debug, Clone)]
pub struct AuditWriter {
    pool: PgPool,
}

impl AuditWriter {
    /// New writer.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Writes an entry and reports failure to the caller.
    ///
    /// # Errors
    /// Database errors.
    pub async fn record(&self, entry: &AuditEntry) -> Result<(), sqlx::Error> {
        insert(&self.pool, entry).await
    }

    /// Writes an entry; a failure is logged (never silently dropped) and does
    /// not affect the caller. For events whose action must go ahead regardless.
    pub async fn record_or_log(&self, entry: AuditEntry) {
        if let Err(err) = self.record(&entry).await {
            tracing::error!(action = entry.action(), error = %err, "audit entry not written");
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_use_the_database_names() {
        assert_eq!(Outcome::Success.as_str(), "success");
        assert_eq!(Outcome::Failure.as_str(), "failure");
        assert_eq!(Outcome::Denied.as_str(), "denied");
    }

    #[test]
    fn metadata_is_always_an_object_that_fits() {
        let e = AuditEntry::new("auth.login", Outcome::Success).metadata(json!("plain"));
        assert_eq!(e.stored_metadata(), json!({"value": "plain"}));
        let e = AuditEntry::new("auth.login", Outcome::Success).metadata(json!({"a": 1}));
        assert_eq!(e.stored_metadata(), json!({"a": 1}));
        let huge = "x".repeat(MAX_METADATA_BYTES + 1);
        let e = AuditEntry::new("auth.login", Outcome::Success).metadata(json!({ "big": huge }));
        assert_eq!(e.stored_metadata(), json!({"truncated": true}));
    }

    #[test]
    fn targets_are_clamped_to_the_column_limits() {
        let e = AuditEntry::new("auth.login", Outcome::Success)
            .target(&"k".repeat(100), "i".repeat(300));
        assert_eq!(e.target_type.as_deref().map(str::len), Some(64));
        assert_eq!(e.target_id.as_deref().map(str::len), Some(128));
    }
}
