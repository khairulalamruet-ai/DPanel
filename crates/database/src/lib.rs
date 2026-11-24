//! PostgreSQL pool, transaction helper, migrations and health ping.
//!
//! * [`connect_lazy`] builds a pool without touching the network, so services
//!   start even when the database is down; readiness reports the outage.
//! * [`MIGRATOR`] embeds `migrations/` at compile time; [`migrate`] applies it.
//! * [`in_transaction`] commits on `Ok` and rolls back on `Err` (or drop).
//!
//! Queries use the compile-time checked `sqlx::query!` family. Builds need
//! either `DATABASE_URL` pointing at a migrated database, or the committed
//! `.sqlx/` offline cache (refresh with `cargo sqlx prepare --workspace`).

pub mod accounts;
pub mod admins;
pub mod apps;
pub mod backups;
pub mod cache_instances;
pub mod certificates;
pub mod container_images;
pub mod cron_jobs;
pub mod databases;
pub mod dns;
pub mod domain_policies;
pub mod domains;
pub mod error_pages;
pub mod exports;
pub mod feature_sets;
pub mod file_transfers;
pub mod ftp_accounts;
pub mod git_deploys;
pub mod images;
pub mod imports;
pub mod ip_blocks;
pub mod license;
pub mod mail;
pub mod malware;
pub mod mfa;
pub mod migration_transfers;
pub mod modules;
pub mod node_enrollments;
pub mod node_firewalls;
pub mod node_services;
pub mod nodes;
pub mod notifications;
pub mod operations;
pub mod outbox;
pub mod page_cache;
pub mod permissions;
pub mod php;
pub mod plans;
pub mod previews;
pub mod rbac;
pub mod redirects;
pub mod resellers;
pub mod routes;
pub mod runtime_apps;
pub mod server_migrations;
pub mod settings;
pub mod skeleton;
pub mod tenant_db;
pub mod tenants;
pub mod transfers;
pub mod usage;
pub mod user_agents;
pub mod users;
pub mod vhost_templates;
pub mod waf;
pub mod wp_advisories;

use std::{future::Future, pin::Pin, time::Duration};

use secrecy::{ExposeSecret, SecretString};
use sqlx::{
    PgConnection, PgPool,
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
};

pub use sqlx;

/// Migrations from the workspace `migrations/` directory, embedded at build time.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// How long a request may wait for a pooled connection before failing.
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
/// Idle connections are closed after this long.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Errors from this crate.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    /// The URL could not be parsed. The URL itself is never included.
    #[error("invalid database URL: {0}")]
    InvalidUrl(String),
    /// Driver or query error.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    /// Migration error.
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Pool settings.
#[derive(Debug, Clone)]
pub struct PoolSettings {
    /// Maximum open connections.
    pub max_connections: u32,
    /// Reported to PostgreSQL as `application_name` (visible in `pg_stat_activity`).
    pub application_name: String,
}

fn connect_options(url: &SecretString, app: &str) -> Result<PgConnectOptions, DatabaseError> {
    let opts: PgConnectOptions = url.expose_secret().parse().map_err(|e: sqlx::Error| {
        // sqlx parse errors do not echo the password, but keep the message generic anyway.
        DatabaseError::InvalidUrl(match e {
            sqlx::Error::Configuration(_) => "malformed connection string".to_owned(),
            other => other.to_string(),
        })
    })?;
    Ok(opts.application_name(app))
}

/// Creates a pool without connecting. Connections open on first use.
///
/// # Errors
/// Only if the URL cannot be parsed.
pub fn connect_lazy(url: &SecretString, settings: &PoolSettings) -> Result<PgPool, DatabaseError> {
    let opts = connect_options(url, &settings.application_name)?;
    Ok(PgPoolOptions::new()
        .max_connections(settings.max_connections)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .idle_timeout(IDLE_TIMEOUT)
        .connect_lazy_with(opts))
}

/// Creates a pool and opens one connection immediately (used by `bpctl`).
///
/// # Errors
/// URL parse errors or connection failures.
pub async fn connect(url: &SecretString, settings: &PoolSettings) -> Result<PgPool, DatabaseError> {
    let opts = connect_options(url, &settings.application_name)?;
    Ok(PgPoolOptions::new()
        .max_connections(settings.max_connections)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_with(opts)
        .await?)
}

/// Runs `SELECT 1`. Used by readiness probes.
///
/// # Errors
/// Any connection or query failure.
pub async fn ping(pool: &PgPool) -> Result<(), DatabaseError> {
    sqlx::query!("SELECT 1 AS one").fetch_one(pool).await?;
    Ok(())
}

/// Applies all pending migrations.
///
/// # Errors
/// Connection or migration failures (including the PostgreSQL 18 version check).
pub async fn migrate(pool: &PgPool) -> Result<(), DatabaseError> {
    MIGRATOR.run(pool).await?;
    Ok(())
}

/// One embedded migration and whether it has been applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStatus {
    /// Migration version (timestamp prefix).
    pub version: i64,
    /// Description from the file name.
    pub description: String,
    /// True if recorded in `_sqlx_migrations` as successful.
    pub applied: bool,
}

/// Lists embedded migrations with their applied state.
///
/// # Errors
/// Connection or query failures.
pub async fn migration_status(pool: &PgPool) -> Result<Vec<MigrationStatus>, DatabaseError> {
    // Compile-time checked against the schema (see `.sqlx/` for offline builds).
    let table_exists = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM pg_tables
               WHERE schemaname = current_schema() AND tablename = '_sqlx_migrations'
           ) AS "exists!""#
    )
    .fetch_one(pool)
    .await?;
    let applied: Vec<i64> = if table_exists {
        sqlx::query_scalar!("SELECT version FROM _sqlx_migrations WHERE success ORDER BY version")
            .fetch_all(pool)
            .await?
    } else {
        Vec::new()
    };
    Ok(MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration())
        .map(|m| MigrationStatus {
            version: m.version,
            description: m.description.to_string(),
            applied: applied.contains(&m.version),
        })
        .collect())
}

/// Boxed future borrowed from the transaction connection.
pub type TxFuture<'c, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'c>>;

/// Runs `f` inside a transaction: commit on `Ok`, roll back on `Err`.
///
/// ```ignore
/// let id = in_transaction(&pool, |conn| Box::pin(async move {
///     sqlx::query_scalar("INSERT ... RETURNING id").fetch_one(&mut *conn).await
/// })).await?;
/// ```
///
/// # Errors
/// The closure's error, or a begin/commit failure converted with `From<sqlx::Error>`.
pub async fn in_transaction<T, E, F>(pool: &PgPool, f: F) -> Result<T, E>
where
    F: for<'c> FnOnce(&'c mut PgConnection) -> TxFuture<'c, T, E>,
    E: From<sqlx::Error>,
{
    let mut tx = pool.begin().await?;
    match f(&mut tx).await {
        Ok(value) => {
            tx.commit().await?;
            Ok(value)
        }
        Err(err) => {
            if let Err(rollback) = tx.rollback().await {
                tracing::warn!(error = %rollback, "transaction rollback failed");
            }
            Err(err)
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn settings() -> PoolSettings {
        PoolSettings {
            max_connections: 2,
            application_name: "test".to_owned(),
        }
    }

    #[test]
    fn embedded_migrations_are_present_and_reversible() {
        let ups: Vec<_> = MIGRATOR
            .iter()
            .filter(|m| m.migration_type.is_up_migration())
            .collect();
        let downs = MIGRATOR
            .iter()
            .filter(|m| m.migration_type.is_down_migration())
            .count();
        assert!(!ups.is_empty());
        assert_eq!(
            ups.len(),
            downs,
            "every up migration needs a down migration"
        );
        assert_eq!(ups[0].description, "schema info");
    }

    #[tokio::test]
    async fn lazy_pool_does_not_connect_and_ping_fails_cleanly_when_db_is_down() {
        // Port 1 on localhost: nothing listens there.
        let url = SecretString::from("postgres://u:topsecret@127.0.0.1:1/db".to_owned());
        let pool = connect_lazy(&url, &settings()).expect("lazy pool builds without network");
        let err = ping(&pool).await.expect_err("nothing listens on port 1");
        assert!(!err.to_string().contains("topsecret"), "{err}");
    }

    #[test]
    fn malformed_url_is_rejected_without_echoing_it() {
        let url = SecretString::from("not a url with secret pw".to_owned());
        let err = connect_lazy(&url, &settings()).expect_err("must fail");
        assert!(!err.to_string().contains("secret pw"), "{err}");
    }
}
