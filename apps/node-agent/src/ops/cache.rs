//! What the agent remembers about operations it has finished.
//!
//! One redb file (`ops.redb`, owner-only) maps an operation id to the result
//! that was sent for it. A result is written and synced to disk *before* it is
//! sent, so "the control plane has an answer" always implies "the agent will
//! give the same answer again".

use std::path::Path;

use builtpanel_node_protocol::OperationResult;
use redb::{
    Database, ReadableDatabase as _, ReadableTable as _, ReadableTableMetadata as _,
    TableDefinition,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// File name inside the agent's state directory.
pub const CACHE_FILE: &str = "ops.redb";
/// How long a result is kept. An operation cannot be re-sent after it expires
/// (at most an hour), so a day is ample.
pub const RETENTION_SECS: i64 = 24 * 3600;

const RESULTS: TableDefinition<'_, [u8; 16], &[u8]> = TableDefinition::new("results_v1");

/// Cache failures. The agent keeps working without the cache (it then cannot
/// promise exactly-once across restarts) and says so in its log.
#[derive(Debug, thiserror::Error)]
#[error("operation cache: {0}")]
pub struct CacheError(String);

fn err(e: impl std::fmt::Display) -> CacheError {
    CacheError(e.to_string())
}

#[derive(Serialize, Deserialize)]
struct Stored {
    /// When it was stored (Unix seconds).
    at: i64,
    result: OperationResult,
}

/// The result cache.
pub struct ResultCache {
    db: Database,
}

impl ResultCache {
    /// Opens (or creates) the cache file. On Unix the file is made owner-only.
    ///
    /// # Errors
    /// The file cannot be created, opened, or is not a cache.
    pub fn open(path: &Path) -> Result<Self, CacheError> {
        let db = Database::create(path).map_err(err)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(err)?;
        }
        Self::init(db)
    }

    /// A cache that lives in memory only (tests, and the fallback when the
    /// file cannot be used).
    ///
    /// # Errors
    /// Never in practice; the signature matches [`ResultCache::open`].
    pub fn in_memory() -> Result<Self, CacheError> {
        let db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(err)?;
        Self::init(db)
    }

    fn init(db: Database) -> Result<Self, CacheError> {
        // Create the table so that readers never see "no such table".
        let tx = db.begin_write().map_err(err)?;
        tx.open_table(RESULTS).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(Self { db })
    }

    /// The stored result for an operation, if there is one.
    ///
    /// # Errors
    /// Storage errors. A damaged entry reads as "not stored".
    pub fn get(&self, op_id: Uuid) -> Result<Option<OperationResult>, CacheError> {
        let tx = self.db.begin_read().map_err(err)?;
        let table = tx.open_table(RESULTS).map_err(err)?;
        let Some(value) = table.get(op_id.into_bytes()).map_err(err)? else {
            return Ok(None);
        };
        Ok(serde_json::from_slice::<Stored>(value.value())
            .ok()
            .map(|s| s.result))
    }

    /// Stores a result durably. An existing entry is kept: the first answer wins.
    /// Returns the result that is stored afterwards.
    ///
    /// # Errors
    /// Storage errors.
    pub fn put(&self, result: &OperationResult, now: i64) -> Result<OperationResult, CacheError> {
        let key = result.op_id.into_bytes();
        let tx = self.db.begin_write().map_err(err)?;
        let kept = {
            let mut table = tx.open_table(RESULTS).map_err(err)?;
            let existing = table
                .get(key)
                .map_err(err)?
                .and_then(|v| serde_json::from_slice::<Stored>(v.value()).ok());
            if let Some(existing) = existing {
                existing.result
            } else {
                let bytes = serde_json::to_vec(&Stored {
                    at: now,
                    result: result.clone(),
                })
                .map_err(err)?;
                table.insert(key, bytes.as_slice()).map_err(err)?;
                result.clone()
            }
        };
        tx.commit().map_err(err)?;
        Ok(kept)
    }

    /// Removes results older than [`RETENTION_SECS`]. Returns how many.
    ///
    /// # Errors
    /// Storage errors.
    pub fn prune(&self, now: i64) -> Result<usize, CacheError> {
        let tx = self.db.begin_write().map_err(err)?;
        let removed = {
            let mut table = tx.open_table(RESULTS).map_err(err)?;
            let mut old = Vec::new();
            for entry in table.iter().map_err(err)? {
                let (key, value) = entry.map_err(err)?;
                let keep = serde_json::from_slice::<Stored>(value.value())
                    .is_ok_and(|s| now.saturating_sub(s.at) < RETENTION_SECS);
                if !keep {
                    old.push(key.value());
                }
            }
            for key in &old {
                table.remove(*key).map_err(err)?;
            }
            old.len()
        };
        tx.commit().map_err(err)?;
        Ok(removed)
    }

    /// How many results are stored.
    ///
    /// # Errors
    /// Storage errors.
    pub fn len(&self) -> Result<u64, CacheError> {
        let tx = self.db.begin_read().map_err(err)?;
        let table = tx.open_table(RESULTS).map_err(err)?;
        table.len().map_err(err)
    }

    /// Whether nothing is stored.
    ///
    /// # Errors
    /// Storage errors.
    pub fn is_empty(&self) -> Result<bool, CacheError> {
        Ok(self.len()? == 0)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use builtpanel_node_protocol::{AgentError, ErrorCode, Output};

    use super::*;

    fn pong(id: Uuid, nonce: u64) -> OperationResult {
        OperationResult::success(id, Output::Pong { nonce })
    }

    #[test]
    fn the_first_answer_is_kept_and_survives_reopening() {
        let path =
            std::env::temp_dir().join(format!("builtpanel-ops-{}.redb", Uuid::now_v7().simple()));
        let id = Uuid::now_v7();
        {
            let cache = ResultCache::open(&path).unwrap();
            assert!(cache.is_empty().unwrap());
            assert_eq!(cache.get(id).unwrap(), None);
            assert_eq!(cache.put(&pong(id, 1), 100).unwrap(), pong(id, 1));
            // A different answer for the same operation does not replace the first.
            let other = OperationResult::failure(id, AgentError::new(ErrorCode::Internal, "x"));
            assert_eq!(cache.put(&other, 200).unwrap(), pong(id, 1));
            assert_eq!(cache.len().unwrap(), 1);
        }
        // As after an agent restart.
        let cache = ResultCache::open(&path).unwrap();
        assert_eq!(cache.get(id).unwrap(), Some(pong(id, 1)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        drop(cache);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn old_results_are_pruned_and_fresh_ones_stay() {
        let cache = ResultCache::in_memory().unwrap();
        let (old, fresh) = (Uuid::now_v7(), Uuid::now_v7());
        cache.put(&pong(old, 1), 1_000).unwrap();
        cache.put(&pong(fresh, 2), 1_000 + RETENTION_SECS).unwrap();
        assert_eq!(cache.prune(1_000 + RETENTION_SECS + 10).unwrap(), 1);
        assert_eq!(cache.get(old).unwrap(), None);
        assert_eq!(cache.get(fresh).unwrap(), Some(pong(fresh, 2)));
        assert_eq!(cache.prune(1_000 + RETENTION_SECS + 10).unwrap(), 0);
    }

    #[test]
    fn a_file_that_is_not_a_cache_is_an_error_not_a_panic() {
        let path = std::env::temp_dir().join(format!(
            "builtpanel-ops-bad-{}.redb",
            Uuid::now_v7().simple()
        ));
        std::fs::write(&path, b"this is not a database").unwrap();
        assert!(ResultCache::open(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
