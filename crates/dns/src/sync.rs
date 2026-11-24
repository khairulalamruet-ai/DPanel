//! Making the DNS server match the panel's zones (T05.09).
//!
//! The panel's tables are the truth. A zone that is `pending` (new, or changed
//! since the last push) is pushed: the zone is created on the server if it is
//! not there and its records are replaced by the panel's. Success makes it
//! `active`. A server that cannot be reached leaves the zone `pending` (the next
//! pass tries again); one that refuses the data makes it `error` with the
//! server's words in `last_error` (tried again after a while, and at once when
//! the zone changes). Zones deleted in the panel are removed from the server
//! from the list the database keeps (`dns_zone_removals`).
//!
//! A push marks the zone `active` only if the zone has not changed since the
//! push began, so an edit made meanwhile is never lost.

use std::{sync::Arc, time::Duration};

use builtpanel_database::{dns as store, sqlx::PgPool};
use tokio::sync::watch;
use uuid::Uuid;

use crate::{DnsError, DnsProvider, ZoneSpec};

/// Where the zones' name servers come from, asked on every pass so a change of
/// the administrator's settings is picked up.
pub trait NameserverSource: Send + Sync + 'static {
    /// The name servers, absolute (`ns1.example.net.`), in the order wanted.
    fn nameservers(&self) -> crate::BoxFuture<'_, Vec<String>>;
}

/// A fixed list.
impl NameserverSource for Vec<String> {
    fn nameservers(&self) -> crate::BoxFuture<'_, Vec<String>> {
        Box::pin(async move { self.clone() })
    }
}

/// How many zones one pass handles.
const BATCH: i64 = 100;

/// What one pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Zones now up to date.
    pub synced: usize,
    /// Zones that could not be pushed.
    pub failed: usize,
    /// Zones removed from the server.
    pub removed: usize,
}

/// Pushes the panel's zones to a DNS server.
#[derive(Clone)]
pub struct ZoneSync {
    db: PgPool,
    provider: Arc<dyn DnsProvider>,
    nameservers: Arc<dyn NameserverSource>,
    retry_errors_after: Duration,
}

impl ZoneSync {
    /// A sync of `db` to `provider`; the zones get `nameservers` (absolute
    /// names) as their apex name servers, new ones when created and existing
    /// ones as soon as the list changes.
    #[must_use]
    pub fn new(
        db: PgPool,
        provider: Arc<dyn DnsProvider>,
        nameservers: Arc<dyn NameserverSource>,
    ) -> Self {
        Self {
            db,
            provider,
            nameservers,
            retry_errors_after: Duration::from_secs(15 * 60),
        }
    }

    /// Pushes one zone now, whatever its status.
    ///
    /// # Errors
    /// The server's error (already recorded on the zone) or a database error.
    pub async fn sync_zone(&self, id: Uuid) -> Result<(), SyncError> {
        let nameservers = self.nameservers.nameservers().await;
        self.sync_zone_with(id, &nameservers).await
    }

    async fn sync_zone_with(&self, id: Uuid, nameservers: &[String]) -> Result<(), SyncError> {
        let digest = nameservers.join(",");
        let Some(zone) = store::zone(&self.db, id).await? else {
            return Ok(());
        };
        let records: Vec<_> = store::records(&self.db, zone.id)
            .await?
            .into_iter()
            .map(|r| r.record)
            .collect();
        let pushed = async {
            self.provider
                .ensure_zone(&ZoneSpec {
                    name: zone.name.clone(),
                    nameservers: nameservers.to_vec(),
                })
                .await?;
            self.provider.replace_records(&zone.name, &records).await
        }
        .await;
        match pushed {
            Ok(()) => {
                store::mark_zone(
                    &self.db,
                    zone.id,
                    &zone.version,
                    "active",
                    None,
                    Some(&digest),
                )
                .await?;
                Ok(())
            }
            Err(error) => {
                // Not reachable: still to do. Refused: needs a look.
                let status = if error.is_retryable() {
                    "pending"
                } else {
                    "error"
                };
                store::mark_zone(
                    &self.db,
                    zone.id,
                    &zone.version,
                    status,
                    Some(&error.to_string()),
                    None,
                )
                .await?;
                Err(SyncError::Dns(error))
            }
        }
    }

    /// One pass: pushes the zones that need it, then removes the deleted ones.
    pub async fn run_once(&self) -> Stats {
        let mut stats = Stats::default();
        let retry = i32::try_from(self.retry_errors_after.as_secs()).unwrap_or(i32::MAX);
        let nameservers = self.nameservers.nameservers().await;
        match store::zones_to_sync(&self.db, retry, &nameservers.join(","), BATCH).await {
            Ok(zones) => {
                for zone in zones {
                    match self.sync_zone_with(zone.id, &nameservers).await {
                        Ok(()) => stats.synced += 1,
                        Err(error) => {
                            tracing::warn!(zone = %zone.name, %error, "zone not pushed to the DNS server");
                            stats.failed += 1;
                            // A server that is down is down for every zone: stop and wait.
                            if matches!(error, SyncError::Dns(ref e) if e.is_retryable()) {
                                return stats;
                            }
                        }
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "could not list zones to push"),
        }
        match store::queued_removals(&self.db, BATCH).await {
            Ok(names) => {
                for name in names {
                    match self.remove(&name).await {
                        Ok(true) => stats.removed += 1,
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(zone = %name, %error, "zone not removed from the DNS server");
                            if matches!(error, SyncError::Dns(ref e) if e.is_retryable()) {
                                break;
                            }
                        }
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "could not list zones to remove"),
        }
        stats
    }

    /// Removes a deleted zone from the server, unless it exists again.
    async fn remove(&self, name: &str) -> Result<bool, SyncError> {
        if store::zone_name_exists(&self.db, name).await? {
            store::finish_removal(&self.db, name).await?;
            return Ok(false);
        }
        self.provider.delete_zone(name).await?;
        store::finish_removal(&self.db, name).await?;
        Ok(true)
    }

    /// Runs passes every `every` until `shutdown` turns `true`.
    pub async fn run(self, every: Duration, mut shutdown: watch::Receiver<bool>) {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stopped(&mut shutdown) => return,
                _ = tick.tick() => {
                    let stats = self.run_once().await;
                    if stats != Stats::default() {
                        tracing::info!(?stats, "DNS sync pass");
                    }
                }
            }
        }
    }
}

/// Resolves when shutdown is requested (the guard `wait_for` returns is dropped
/// before this future yields, so it can be sent between threads).
async fn stopped(shutdown: &mut watch::Receiver<bool>) {
    let _ = shutdown.wait_for(|stop| *stop).await;
}

/// Why a sync failed.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// The DNS server's answer.
    #[error(transparent)]
    Dns(#[from] DnsError),
    /// The database.
    #[error(transparent)]
    Database(#[from] builtpanel_database::sqlx::Error),
}
