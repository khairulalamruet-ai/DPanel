//! The event bus (T03.05).
//!
//! **Publishing** is one call inside the transaction that makes the event
//! true:
//!
//! ```ignore
//! let mut tx = pool.begin().await?;
//! delete_tenant(&mut tx, id).await?;
//! builtpanel_events::publish(&mut tx, &Event::TenantDeleted { tenant_id: id }).await?;
//! tx.commit().await?;   // both happen, or neither
//! ```
//!
//! A rollback takes the event with it: nothing is ever announced that did not
//! happen. Nothing that happened goes unannounced either, because the row is
//! durable before the transaction returns.
//!
//! **Delivering** is the [`Relay`]: it takes undelivered events oldest first
//! and calls [`Module::on_event`](builtpanel_kernel::Module::on_event) of every
//! active module, dependencies first. If a module fails or takes too long, the
//! event is tried again later with a growing delay (all modules see it again,
//! which is why handlers must be safe to repeat); after too many failures it is
//! set aside as dead and reported, and the relay moves on. Several relays can
//! run at once: an event is held by one of them at a time.

use std::{sync::Arc, time::Duration};

use builtpanel_database::{
    outbox,
    sqlx::{self, PgConnection, PgPool},
};
use builtpanel_kernel::{Event, Module};
use tokio::sync::watch;
use uuid::Uuid;

/// Why an event could not be written.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    /// The event could not be turned into JSON.
    #[error("cannot serialize the event: {0}")]
    Serialize(String),
    /// The database refused it.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Writes `event` to the outbox on `conn`, which must be the connection of the
/// transaction that makes the event true.
///
/// # Errors
/// [`PublishError`]; the caller's transaction should then be rolled back.
pub async fn publish(conn: &mut PgConnection, event: &Event) -> Result<Uuid, PublishError> {
    let payload =
        serde_json::to_value(event).map_err(|e| PublishError::Serialize(e.to_string()))?;
    Ok(outbox::publish(conn, event.type_name(), &payload).await?)
}

/// Timing and limits of the relay.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Pause between passes when there is nothing to do.
    pub poll: Duration,
    /// Most events taken per pass.
    pub batch: i64,
    /// Deliveries tried before an event is set aside as dead.
    pub max_attempts: i32,
    /// Wait before the first retry; doubles each time.
    pub retry_base: Duration,
    /// Longest wait between retries.
    pub retry_cap: Duration,
    /// Longest one module may take to handle one event.
    pub handler_timeout: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(500),
            batch: 50,
            max_attempts: 10,
            retry_base: Duration::from_secs(2),
            retry_cap: Duration::from_secs(300),
            handler_timeout: Duration::from_secs(30),
        }
    }
}

impl Settings {
    /// Wait after failure number `failures` (1 for the first).
    #[must_use]
    pub fn backoff(&self, failures: i32) -> Duration {
        let doublings = u32::try_from(failures.saturating_sub(1).clamp(0, 16)).unwrap_or(0);
        self.retry_base
            .saturating_mul(1_u32 << doublings)
            .min(self.retry_cap)
    }
}

/// What one pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PassStats {
    /// Events every module handled.
    pub delivered: usize,
    /// Events that will be tried again.
    pub failed: usize,
    /// Events given up on in this pass.
    pub dead: usize,
}

/// Which modules receive events right now (the active ones, dependencies first).
pub type ActiveModules = Arc<dyn Fn() -> Vec<Arc<dyn Module>> + Send + Sync>;

/// Delivers outbox events to modules.
#[derive(Clone)]
pub struct Relay {
    db: PgPool,
    modules: ActiveModules,
    settings: Settings,
}

impl Relay {
    /// A relay over this database for these modules.
    #[must_use]
    pub fn new(db: PgPool, modules: ActiveModules, settings: Settings) -> Self {
        Self {
            db,
            modules,
            settings,
        }
    }

    /// Runs until `shutdown` turns `true`.
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        loop {
            let idle = match self.run_once().await {
                Ok(stats) => stats == PassStats::default(),
                Err(err) => {
                    tracing::warn!(error = %err, "event relay could not reach the database");
                    true
                }
            };
            if idle {
                tokio::select! {
                    () = stopped(&mut shutdown) => return,
                    () = tokio::time::sleep(self.settings.poll) => {}
                }
            } else if *shutdown.borrow() {
                return;
            }
        }
    }

    /// One pass over the events that are due.
    ///
    /// # Errors
    /// Database errors; events of the pass stay due and are taken again.
    pub async fn run_once(&self) -> Result<PassStats, sqlx::Error> {
        let (mut tx, pending) = outbox::claim(&self.db, self.settings.batch).await?;
        let mut stats = PassStats::default();
        if pending.is_empty() {
            return Ok(stats);
        }
        let modules = (self.modules)();
        for row in pending {
            match self.deliver(&row, &modules).await {
                Ok(()) => {
                    outbox::mark_delivered(&mut tx, row.id).await?;
                    stats.delivered += 1;
                }
                Err(reason) => {
                    let wait = self.settings.backoff(row.attempts + 1);
                    let dead = outbox::mark_failed(
                        &mut tx,
                        row.id,
                        &reason,
                        i32::try_from(wait.as_secs()).unwrap_or(i32::MAX),
                        self.settings.max_attempts,
                    )
                    .await?;
                    if dead {
                        stats.dead += 1;
                        tracing::error!(event_id = %row.id, event_type = %row.event_type, %reason, "event given up on after too many failed deliveries");
                    } else {
                        stats.failed += 1;
                        tracing::warn!(event_id = %row.id, event_type = %row.event_type, %reason, attempt = row.attempts + 1, retry_in = ?wait, "event delivery failed");
                    }
                }
            }
        }
        tx.commit().await?;
        Ok(stats)
    }

    /// Hands one event to every module. The first problem stops it.
    async fn deliver(
        &self,
        row: &outbox::Pending,
        modules: &[Arc<dyn Module>],
    ) -> Result<(), String> {
        let event: Event = serde_json::from_value(row.payload.clone())
            .map_err(|_| format!("this build does not understand `{}` events", row.event_type))?;
        for module in modules {
            let name = &module.manifest().module.name;
            match tokio::time::timeout(self.settings.handler_timeout, module.on_event(&event)).await
            {
                Ok(Ok(())) => {}
                Ok(Err(err)) => return Err(format!("module `{name}`: {err}")),
                Err(_) => return Err(format!("module `{name}` did not finish in time")),
            }
        }
        Ok(())
    }
}

async fn stopped(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn retry_waits_double_up_to_the_cap() {
        let s = Settings::default();
        let waits: Vec<u64> = (1..=10).map(|n| s.backoff(n).as_secs()).collect();
        assert_eq!(waits, [2, 4, 8, 16, 32, 64, 128, 256, 300, 300]);
        assert_eq!(s.backoff(0), s.retry_base);
        assert_eq!(s.backoff(i32::MAX), s.retry_cap);
    }

    #[test]
    fn events_have_stable_wire_names() {
        let id = Uuid::nil();
        for (event, name) in [
            (
                Event::ModuleEnabled { name: "dns".into() },
                "module_enabled",
            ),
            (
                Event::ModuleDisabled { name: "dns".into() },
                "module_disabled",
            ),
            (Event::TenantCreated { tenant_id: id }, "tenant_created"),
            (Event::TenantDeleted { tenant_id: id }, "tenant_deleted"),
            (
                Event::DomainAdded {
                    tenant_id: id,
                    domain_id: id,
                    name: "example.com".into(),
                },
                "domain_added",
            ),
        ] {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["type"], name);
            assert_eq!(event.type_name(), name);
            assert_eq!(serde_json::from_value::<Event>(json).unwrap(), event);
        }
        assert!(serde_json::from_str::<Event>(r#"{"type":"moon_landed"}"#).is_err());
    }
}
