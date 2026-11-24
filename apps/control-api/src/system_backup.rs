//! Makes and removes the panel's own backups (T09.08).
//!
//! The runner of `builtpanel-mod-backups` only writes rows (a schedule that is due, or an
//! administrator's request, makes a `system` backup `queued`; retention makes one `deleting`).
//! This executor does the work **here**, on the control host, because the data (the control
//! database, the configuration, the node CA) is here and no node can reach it: `pg_dump`, the
//! archive of `builtpanel-system-backup`, then the destination.

use std::{path::PathBuf, sync::Arc};

use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_config::{CONFIG_PATH_ENV, ControlConfig, DEFAULT_CONTROL_CONFIG};
use builtpanel_database::{backups as store, sqlx::PgPool};
use builtpanel_system_backup::{Connection, Job, derive_key, run_job};
use secrecy::ExposeSecret as _;
use uuid::Uuid;

/// What the executor needs to know.
#[derive(Clone)]
pub struct Executor {
    db: PgPool,
    master_key: MasterKey,
    config: Arc<ControlConfig>,
    /// Where the dump and the archive wait while they are made (private).
    scratch: PathBuf,
}

impl Executor {
    /// An executor; `scratch` is a folder on a disk with room for the dump twice over.
    #[must_use]
    pub fn new(
        db: PgPool,
        master_key: MasterKey,
        config: Arc<ControlConfig>,
        scratch: PathBuf,
    ) -> Self {
        Self {
            db,
            master_key,
            config,
            scratch,
        }
    }

    /// One round: the queued backups are made, the ones being removed are removed.
    pub async fn run(&self) {
        match store::claim_system_backups(&self.db).await {
            Ok(claimed) => {
                for backup in claimed {
                    let result = self
                        .make(backup.id, backup.destination_id, backup.created_at)
                        .await;
                    let saved = match result {
                        Ok((size, parts, sha)) => {
                            store::finish_system_backup(&self.db, backup.id, size, &parts, &sha)
                                .await
                        }
                        Err(error) => {
                            tracing::warn!(%error, backup = %backup.id, "the system backup failed");
                            store::fail_system_backup(&self.db, backup.id, &error).await
                        }
                    };
                    if let Err(error) = saved {
                        tracing::warn!(%error, "could not record a system backup");
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "could not look for system backups"),
        }
        match store::deleting_system(&self.db).await {
            Ok(list) => {
                for backup in list {
                    let result = self.remove(backup.id, backup.destination_id).await;
                    if let Err(error) = &result {
                        tracing::warn!(%error, backup = %backup.id, "could not remove a system backup");
                    }
                    let _ =
                        store::finish_system_removal(&self.db, backup.id, result.err().as_deref())
                            .await;
                }
            }
            Err(error) => tracing::warn!(%error, "could not look for system backups to remove"),
        }
    }

    async fn destination(
        &self,
        id: Uuid,
    ) -> Result<(String, serde_json::Value, serde_json::Value), String> {
        let d = store::destination(&self.db, id)
            .await
            .map_err(|_| "could not read the destination".to_owned())?
            .ok_or_else(|| "the destination is gone".to_owned())?;
        let secrets = match store::sealed_credentials(&self.db, id)
            .await
            .map_err(|_| "could not read the credentials".to_owned())?
        {
            Some(sealed) => {
                let plain = self
                    .master_key
                    .open(&store::credentials_context(id), &sealed)
                    .map_err(|_| "the destination's credentials do not open".to_owned())?;
                serde_json::from_slice(&plain)
                    .map_err(|_| "the destination's credentials are not readable".to_owned())?
            }
            None => serde_json::Value::Null,
        };
        Ok((d.kind, d.settings, secrets))
    }

    async fn make(
        &self,
        id: Uuid,
        destination: Uuid,
        created_at: i64,
    ) -> Result<(i64, serde_json::Value, String), String> {
        let (kind, settings, secrets) = self.destination(destination).await?;
        let key = derive_key(
            self.config
                .master_key()
                .map_err(|e| e.to_string())?
                .expose_secret(),
        );
        let connection = Connection::parse(
            self.config
                .database_url()
                .map_err(|e| e.to_string())?
                .expose_secret(),
        )
        .map_err(|e| e.to_string())?;
        let config = &self.config;
        let config_file = std::env::var_os(CONFIG_PATH_ENV)
            .map_or_else(|| PathBuf::from(DEFAULT_CONTROL_CONFIG), PathBuf::from);
        let mut secret_files = Vec::new();
        for (name, path) in [
            ("database_url", config.database.url_file.clone()),
            ("redis_url", config.redis.url_file.clone()),
            ("powerdns_api_key", config.dns.powerdns_api_key_file.clone()),
        ] {
            if let Some(path) = path.filter(|p| p.is_file()) {
                secret_files.push((name.to_owned(), path));
            }
        }
        let job = Job {
            id: id.simple().to_string(),
            created_at,
            kind,
            settings,
            secrets,
            connection,
            config_file: config_file.is_file().then_some(config_file),
            secret_files,
            ca_dir: Some(config.pki.ca_dir.clone()),
            license_dir: Some(config.license.identity_dir.clone()),
            scratch: self.scratch.clone(),
            key,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        tokio::task::spawn_blocking(move || {
            run_job(&job).map(|d| (d.total_bytes, d.parts, d.manifest_sha256))
        })
        .await
        .map_err(|_| "the work was cancelled".to_owned())?
    }

    async fn remove(&self, id: Uuid, destination: Uuid) -> Result<(), String> {
        let (kind, settings, secrets) = self.destination(destination).await?;
        let prefix = format!("{}/", id.simple());
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let dest = builtpanel_backup_dest::open::open(&kind, &settings, &secrets)
                .map_err(|e| e.to_string())?;
            for name in dest.list(&prefix).map_err(|e| e.to_string())? {
                if name.starts_with(&prefix) {
                    dest.delete(&name).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| "the work was cancelled".to_owned())?
    }
}
