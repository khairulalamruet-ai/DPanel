//! The operations ledger service (T02.10).
//!
//! Everything the control plane asks a node to do goes through
//! [`Operations::enqueue`]: the typed [`Operation`] is stored first, with an
//! idempotency key, and the dispatcher (T02.11) sends it later. Enqueueing
//! never needs the node to be online.
//!
//! * **Idempotent:** the same key for the same node and the same operation
//!   returns the row that already exists, however often and however
//!   concurrently it is asked. The same key for a *different* operation is an
//!   error, never a silent swap.
//! * **Bounded:** keys, lifetime and attempts are validated; a revoked node
//!   takes no new work.
//! * **Orderly:** status moves along `pending → dispatched → succeeded |
//!   failed | retryable` only; the database enforces it.

use std::time::Duration;

pub use builtpanel_database::operations::{Enqueued, Operation as Row, Status};
use builtpanel_database::{
    nodes,
    operations::{self as db, EnqueueError},
    sqlx::PgPool,
};
use builtpanel_node_protocol::{
    MAX_ENVELOPE_LIFETIME_SECS, Operation, OperationEnvelope, PROTOCOL_VERSION,
};
use uuid::Uuid;

/// Longest idempotency key.
pub const MAX_KEY_LEN: usize = 128;
/// Default time an operation stays worth sending.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);
/// Default number of deliveries.
pub const DEFAULT_MAX_ATTEMPTS: u8 = 3;

/// What to enqueue.
#[derive(Debug, Clone)]
pub struct Request {
    /// The node that runs it.
    pub node_id: Uuid,
    /// What to do.
    pub op: Operation,
    /// Same key on the same node = same request. 1-128 characters of
    /// `A-Z a-z 0-9 . _ : -`.
    pub idempotency_key: String,
    /// Tenant it is about.
    pub tenant_id: Option<Uuid>,
    /// Account that asked.
    pub created_by: Option<Uuid>,
    /// How long it stays worth sending (1 s up to the protocol's one hour).
    pub ttl: Duration,
    /// Most deliveries (1..=20).
    pub max_attempts: u8,
}

impl Request {
    /// A request with the default lifetime and attempts.
    #[must_use]
    pub fn new(node_id: Uuid, op: Operation, idempotency_key: impl Into<String>) -> Self {
        Self {
            node_id,
            op,
            idempotency_key: idempotency_key.into(),
            tenant_id: None,
            created_by: None,
            ttl: DEFAULT_TTL,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        }
    }
}

/// Why a request was refused.
#[derive(Debug, thiserror::Error)]
pub enum OperationsError {
    /// The key is empty, too long, or has characters outside the allowed set.
    #[error("invalid idempotency key")]
    InvalidKey,
    /// The lifetime is outside 1 second .. 1 hour.
    #[error("invalid lifetime")]
    InvalidTtl,
    /// The attempts are outside 1..=20.
    #[error("invalid number of attempts")]
    InvalidAttempts,
    /// No such node.
    #[error("unknown node")]
    UnknownNode,
    /// The node is revoked and takes no new work.
    #[error("the node is revoked")]
    NodeRevoked,
    /// The key was used before for a different operation.
    #[error("the idempotency key was already used for a different operation")]
    KeyReused,
    /// The operation could not be turned into JSON.
    #[error("cannot serialize the operation: {0}")]
    Serialize(String),
    /// Database error.
    #[error(transparent)]
    Database(#[from] builtpanel_database::sqlx::Error),
}

fn valid_key(key: &str) -> bool {
    (1..=MAX_KEY_LEN).contains(&key.len())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

/// The ledger.
#[derive(Clone)]
pub struct Operations {
    db: PgPool,
}

impl Operations {
    /// A ledger on this pool.
    #[must_use]
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// The database pool the ledger uses.
    #[must_use]
    pub fn db(&self) -> &PgPool {
        &self.db
    }

    /// Adds the operation to the ledger as `pending`, or returns the row from
    /// an earlier identical request.
    ///
    /// # Errors
    /// [`OperationsError`]: invalid input, unknown or revoked node, a key that
    /// belongs to a different operation, or database errors.
    pub async fn enqueue(&self, request: &Request) -> Result<Enqueued, OperationsError> {
        if !valid_key(&request.idempotency_key) {
            return Err(OperationsError::InvalidKey);
        }
        let ttl = i32::try_from(request.ttl.as_secs())
            .ok()
            .filter(|s| (1..=MAX_ENVELOPE_LIFETIME_SECS as i32).contains(s))
            .ok_or(OperationsError::InvalidTtl)?;
        if !(1..=20).contains(&request.max_attempts) {
            return Err(OperationsError::InvalidAttempts);
        }
        match nodes::status_of(&self.db, request.node_id).await? {
            None => return Err(OperationsError::UnknownNode),
            Some(status) if status == "revoked" => return Err(OperationsError::NodeRevoked),
            Some(_) => {}
        }
        let payload = serde_json::to_value(&request.op)
            .map_err(|e| OperationsError::Serialize(e.to_string()))?;
        db::enqueue(
            &self.db,
            &db::NewOperation {
                node_id: request.node_id,
                kind: request.op.type_name(),
                tenant_id: request.tenant_id,
                payload: &payload,
                idempotency_key: &request.idempotency_key,
                created_by: request.created_by,
                ttl_secs: ttl,
                max_attempts: i32::from(request.max_attempts),
            },
        )
        .await
        .map_err(|e| match e {
            EnqueueError::KeyReused => OperationsError::KeyReused,
            EnqueueError::UnknownNode => OperationsError::UnknownNode,
            EnqueueError::Database(e) => OperationsError::Database(e),
        })
    }

    /// One operation.
    ///
    /// # Errors
    /// Database errors.
    pub async fn get(&self, id: Uuid) -> Result<Option<Row>, OperationsError> {
        Ok(db::get(&self.db, id).await?)
    }

    /// Recent operations, newest first, optionally of one node, status or kind.
    ///
    /// # Errors
    /// Database errors.
    pub async fn recent(
        &self,
        node_id: Option<Uuid>,
        status: Option<Status>,
        kind: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Row>, OperationsError> {
        Ok(db::list_recent(&self.db, node_id, status, kind, limit.clamp(1, 500)).await?)
    }

    /// Takes due operations of these nodes for delivery (marks them `dispatched`).
    ///
    /// # Errors
    /// Database errors.
    pub async fn claim_due(&self, nodes: &[Uuid], limit: i64) -> Result<Vec<Row>, OperationsError> {
        Ok(db::claim_due(&self.db, nodes, limit.clamp(1, 500)).await?)
    }

    /// Puts an operation that could not be delivered back to `pending`.
    ///
    /// # Errors
    /// Database errors.
    pub async fn release_undelivered(&self, id: Uuid) -> Result<bool, OperationsError> {
        Ok(db::release_undelivered(&self.db, id).await?)
    }

    /// Records the agent's successful answer.
    ///
    /// # Errors
    /// Database errors.
    pub async fn succeed(
        &self,
        id: Uuid,
        output: &serde_json::Value,
    ) -> Result<bool, OperationsError> {
        Ok(db::succeed(&self.db, id, output).await?)
    }

    /// Records a failure; `retry_in` makes it retryable while attempts and time allow.
    ///
    /// # Errors
    /// Database errors.
    pub async fn fail(
        &self,
        id: Uuid,
        error: &serde_json::Value,
        retry_in: Option<Duration>,
    ) -> Result<Option<Status>, OperationsError> {
        let secs = retry_in.map(|d| i32::try_from(d.as_secs()).unwrap_or(i32::MAX));
        Ok(db::fail(&self.db, id, error, secs).await?)
    }

    /// Handles operations that were sent and never answered within `timeout`.
    ///
    /// # Errors
    /// Database errors.
    pub async fn time_out_dispatched(
        &self,
        timeout: Duration,
        retry_in: Duration,
    ) -> Result<Vec<(Uuid, Status)>, OperationsError> {
        let secs = |d: Duration| i32::try_from(d.as_secs()).unwrap_or(i32::MAX);
        Ok(db::time_out_dispatched(&self.db, secs(timeout), secs(retry_in)).await?)
    }

    /// Fails operations that expired before they could be sent.
    ///
    /// # Errors
    /// Database errors.
    pub async fn expire_overdue(&self) -> Result<u64, OperationsError> {
        Ok(db::expire_overdue(&self.db).await?)
    }
}

/// The wire envelope for a ledger row, stamped at `now` (Unix seconds). Its
/// lifetime ends at the row's expiry but is capped by the protocol maximum.
///
/// # Errors
/// The stored payload is not a known operation (a ledger row written by a
/// newer control plane), which the caller should treat as a permanent failure.
pub fn envelope(row: &Row, now: i64) -> Result<OperationEnvelope, serde_json::Error> {
    let op: Operation = serde_json::from_value(row.payload.clone())?;
    Ok(OperationEnvelope {
        op_id: row.id,
        idempotency_key: row.idempotency_key.clone(),
        tenant_id: row.tenant_id,
        issued_at: now,
        expires_at: row.expires_at.min(now + MAX_ENVELOPE_LIFETIME_SECS),
        protocol_version: PROTOCOL_VERSION,
        op,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_short_and_plain() {
        for ok in ["a", "ping-1", "site.example.com:create_1", &"k".repeat(128)] {
            assert!(valid_key(ok), "{ok}");
        }
        for bad in ["", " ", "a b", "a/b", "ä", "a\n", &"k".repeat(129)] {
            assert!(!valid_key(bad), "{bad:?}");
        }
    }
}
