//! This installation's license, kept up to date (T17.09, T17.10).
//!
//! One [`LicenseService`] holds the client of the authority, the trusted keys and the policy.
//! It writes everything down in the `license_state` row: what the client keeps (the signed
//! entitlement, when it was last confirmed) and the verdict ([`builtpanel_license_client::evaluate`]),
//! which the restricted-mode middleware and the limits read. [`LicenseService::run`] is the
//! loop that asks the authority: a heartbeat every few hours, an entitlement refresh twice a
//! day, and a new verdict every minute (so the end of the offline grace is noticed on time).

use std::{sync::Arc, time::Duration};

use builtpanel_config::LicenseConfig;
use builtpanel_database::{
    license::{self as store, Kept, Verdict},
    sqlx::PgPool,
};
use builtpanel_license::Expect;
use builtpanel_license_client::{
    ClientError, Evaluation, Identity, LicenseClient, Mode, Policy, Received, Stored, evaluate,
};
use uuid::Uuid;

/// Why the service could not do what was asked.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// Licensing is not switched on (no authority is configured).
    #[error("licensing is not configured")]
    Disabled,
    /// The authority could not be used.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The database failed.
    #[error("database: {0}")]
    Database(String),
    /// The work was cancelled.
    #[error("cancelled")]
    Cancelled,
}

fn db(e: &builtpanel_database::sqlx::Error) -> ServiceError {
    ServiceError::Database(e.to_string())
}

/// The machine's host name, as the kernel knows it.
fn machine_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The panel's license.
pub struct LicenseService {
    db: PgPool,
    client: Option<Arc<LicenseClient>>,
    keys: builtpanel_license::PublicKeys,
    issuer: String,
    audience: String,
    installation_id: Option<Uuid>,
    /// The machine's host name and the domain the panel is reached at (the license is bound to
    /// both).
    hostname: String,
    domain: String,
    policy: Policy,
    heartbeat_every: Duration,
    refresh_every: Duration,
    /// Where the release's programs are installed, and how often they are checked.
    binaries_dir: std::path::PathBuf,
    integrity_every: Duration,
}

impl LicenseService {
    /// Sets the service up from the configuration: with an authority it makes (or reads) the
    /// installation's identity and builds the client; without one licensing is off.
    ///
    /// # Errors
    /// A message when the identity, the keys or the address cannot be used.
    pub fn new(db: PgPool, config: &LicenseConfig) -> Result<Self, String> {
        let hours = |h: u32| Duration::from_secs(u64::from(h) * 3600);
        let policy = Policy {
            refresh_every: i64::from(config.refresh_hours) * 3600,
            grace: i64::from(config.grace_hours) * 3600,
        };
        let hostname = machine_hostname();
        let domain = config.domain.clone().unwrap_or_default();
        let keys = builtpanel_license_client::trusted_keys(config.public_keys_file.as_deref())
            .map_err(|e| format!("the authority's public keys: {e}"))?;
        let (client, installation_id) = match &config.authority_url {
            None => (None, None),
            Some(url) => {
                let identity = Identity::load_or_create(&config.identity_dir)
                    .map_err(|e| format!("the installation's identity: {e}"))?;
                let id = identity.installation_id;
                let client = LicenseClient::new(
                    url,
                    keys.clone(),
                    identity,
                    &config.issuer,
                    &config.audience,
                )
                .map_err(|e| e.to_string())?
                .with_host(&hostname, &domain);
                (Some(Arc::new(client)), Some(id))
            }
        };
        Ok(Self {
            db,
            client,
            keys,
            issuer: config.issuer.clone(),
            audience: config.audience.clone(),
            installation_id,
            hostname,
            domain,
            policy,
            heartbeat_every: hours(config.heartbeat_hours),
            refresh_every: hours(config.refresh_hours),
            binaries_dir: config.binaries_dir.clone(),
            integrity_every: hours(config.integrity_hours),
        })
    }

    /// This installation's answer to the authority's domain check, or `None` when licensing is
    /// off or the panel has no domain configured.
    #[must_use]
    pub fn domain_proof(&self, nonce: &str) -> Option<builtpanel_license_client::DomainProof> {
        self.client.as_ref()?.domain_proof(nonce).ok()
    }

    /// Whether licensing is on.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.client.is_some()
    }

    /// This installation's id (when licensing is on).
    #[must_use]
    pub fn installation_id(&self) -> Option<Uuid> {
        self.installation_id
    }

    /// What is stored.
    ///
    /// # Errors
    /// [`ServiceError::Database`].
    pub async fn stored(&self) -> Result<Stored, ServiceError> {
        let row = store::get(&self.db).await.map_err(|e| db(&e))?;
        Ok(serde_json::from_value(row.stored).unwrap_or_default())
    }

    /// Decides what the stored license allows now and writes the verdict down.
    ///
    /// # Errors
    /// [`ServiceError::Database`].
    pub async fn evaluate_and_save(&self) -> Result<Option<Evaluation>, ServiceError> {
        let Some(id) = self.installation_id else {
            // Licensing is off: nothing is checked and nothing is restricted.
            store::save_verdict(
                &self.db,
                &Verdict {
                    enforced: false,
                    mode: "unlicensed",
                    reason: None,
                    features: &[],
                    limits: &serde_json::json!({}),
                    entitlement_expires_at: None,
                    grace_ends_at: None,
                },
            )
            .await
            .map_err(|e| db(&e))?;
            return Ok(None);
        };
        let stored = self.stored().await?;
        let expect = Expect {
            issuer: &self.issuer,
            audience: &self.audience,
            installation_id: id,
            clock_skew: 300,
            host: builtpanel_license::HostFacts {
                hostname: &self.hostname,
                domain: &self.domain,
            },
        };
        let verdict = evaluate(&stored, &self.keys, &expect, &self.policy, unix_now());
        let mode = match verdict.mode {
            Mode::Unlicensed => "unlicensed",
            Mode::Active => "active",
            Mode::Grace => "grace",
            Mode::Restricted => "restricted",
        };
        let (features, limits, ends) = verdict.claims.as_ref().map_or_else(
            || (Vec::new(), serde_json::json!({}), None),
            |c| (c.features.clone(), serde_json::json!(c.limits), Some(c.exp)),
        );
        store::save_verdict(
            &self.db,
            &Verdict {
                enforced: true,
                mode,
                reason: verdict.reason.as_deref(),
                features: &features,
                limits: &limits,
                entitlement_expires_at: ends,
                grace_ends_at: verdict.grace_ends_at,
            },
        )
        .await
        .map_err(|e| db(&e))?;
        Ok(Some(verdict))
    }

    async fn keep(
        &self,
        stored: &Stored,
        summary: Option<&builtpanel_license_client::LicenseSummary>,
    ) -> Result<(), ServiceError> {
        let value = serde_json::to_value(stored).unwrap_or_default();
        store::save_stored(
            &self.db,
            &Kept {
                stored: &value,
                license_id: summary.map(|s| s.id),
                plan: summary.map(|s| s.plan.as_str()),
                license_status: summary.map(|s| s.status.as_str()),
                license_expires_at: summary.and_then(|s| s.expires_at),
            },
        )
        .await
        .map_err(|e| db(&e))
    }

    fn client(&self) -> Result<Arc<LicenseClient>, ServiceError> {
        self.client.clone().ok_or(ServiceError::Disabled)
    }

    async fn confirmed(&self, got: Received) -> Result<(), ServiceError> {
        let mut stored = self.stored().await?;
        stored.entitlement = Some(got.document);
        stored.last_refresh_ok_at = Some(unix_now());
        stored.refused = None;
        self.keep(&stored, Some(&got.license)).await?;
        self.evaluate_and_save().await?;
        Ok(())
    }

    /// What a failed call means for the stored license: a refusal of the license is kept (and
    /// restricts at once); trouble reaching the authority is only noted (the grace covers it).
    async fn failed(&self, err: &ClientError) -> Result<(), ServiceError> {
        if let ClientError::Refused { code, .. } = err
            && err.is_license_refusal()
        {
            let mut stored = self.stored().await?;
            stored.refused = Some(code.clone());
            let row = store::get(&self.db).await.map_err(|e| db(&e))?;
            store::save_stored(
                &self.db,
                &Kept {
                    stored: &serde_json::to_value(&stored).unwrap_or_default(),
                    license_id: row.license_id,
                    plan: row.plan.as_deref(),
                    license_status: Some(code.strip_prefix("license_").unwrap_or(code)),
                    license_expires_at: row.license_expires_at,
                },
            )
            .await
            .map_err(|e| db(&e))?;
        }
        store::note_error(&self.db, &err.to_string())
            .await
            .map_err(|e| db(&e))?;
        self.evaluate_and_save().await?;
        Ok(())
    }

    /// Activates this installation with a license key.
    ///
    /// # Errors
    /// [`ServiceError`].
    pub async fn activate(&self, license_key: String) -> Result<(), ServiceError> {
        let client = self.client()?;
        let result = tokio::task::spawn_blocking(move || client.activate(&license_key))
            .await
            .map_err(|_| ServiceError::Cancelled)?;
        match result {
            Ok(got) => self.confirmed(got).await,
            Err(e) => {
                // A wrong key is the person's to fix; it does not change the stored license.
                store::note_error(&self.db, &e.to_string())
                    .await
                    .map_err(|e| db(&e))?;
                Err(e.into())
            }
        }
    }

    /// Asks the authority for a fresh entitlement.
    ///
    /// # Errors
    /// [`ServiceError`].
    pub async fn refresh(&self) -> Result<(), ServiceError> {
        let client = self.client()?;
        match tokio::task::spawn_blocking(move || client.refresh())
            .await
            .map_err(|_| ServiceError::Cancelled)?
        {
            Ok(got) => self.confirmed(got).await,
            Err(e) => {
                self.failed(&e).await?;
                Err(e.into())
            }
        }
    }

    /// Tells the authority this installation is alive.
    ///
    /// # Errors
    /// [`ServiceError`].
    pub async fn heartbeat(&self) -> Result<(), ServiceError> {
        let client = self.client()?;
        let nodes = count(&self.db, "nodes").await;
        let tenants = count(&self.db, "tenants").await;
        let info = serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "nodes": nodes, "tenants": tenants });
        match tokio::task::spawn_blocking(move || client.heartbeat(&info))
            .await
            .map_err(|_| ServiceError::Cancelled)?
        {
            Ok(beat) => {
                let mut stored = self.stored().await?;
                stored.last_heartbeat_ok_at = Some(unix_now());
                // The authority says the license is no longer good: restricted at once.
                if beat.license_status != "active" {
                    stored.refused = Some(format!("license_{}", beat.license_status));
                } else if beat.integrity != "ok" {
                    stored.refused = Some("integrity_failed".to_owned());
                }
                let row = store::get(&self.db).await.map_err(|e| db(&e))?;
                store::save_stored(
                    &self.db,
                    &Kept {
                        stored: &serde_json::to_value(&stored).unwrap_or_default(),
                        license_id: row.license_id,
                        plan: row.plan.as_deref(),
                        license_status: Some(beat.license_status.as_str()),
                        license_expires_at: row.license_expires_at,
                    },
                )
                .await
                .map_err(|e| db(&e))?;
                self.evaluate_and_save().await?;
                Ok(())
            }
            Err(e) => {
                self.failed(&e).await?;
                Err(e.into())
            }
        }
    }

    /// Lets go of the license.
    ///
    /// # Errors
    /// [`ServiceError`].
    pub async fn deactivate(&self) -> Result<(), ServiceError> {
        let client = self.client()?;
        tokio::task::spawn_blocking(move || client.deactivate())
            .await
            .map_err(|_| ServiceError::Cancelled)??;
        self.keep(&Stored::default(), None).await?;
        self.evaluate_and_save().await?;
        Ok(())
    }

    /// Asks the authority for help.
    ///
    /// # Errors
    /// [`ServiceError`].
    pub async fn recovery_request(&self, note: String) -> Result<(), ServiceError> {
        let client = self.client()?;
        tokio::task::spawn_blocking(move || client.recovery_request(&note))
            .await
            .map_err(|_| ServiceError::Cancelled)?
            .map_err(Into::into)
    }

    /// The integrity scan: asks the authority which files make this release, hashes them where
    /// they are installed and sends the hashes back (signed). A file that is not there is left
    /// out of the report, which is how the authority learns it is missing.
    ///
    /// # Errors
    /// [`ServiceError`].
    pub async fn integrity_scan(
        &self,
    ) -> Result<builtpanel_license_client::ScanResult, ServiceError> {
        let client = self.client()?;
        let dir = self.binaries_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let scan = client.integrity_challenge(env!("CARGO_PKG_VERSION"))?;
            let mut files = std::collections::BTreeMap::new();
            for name in &scan.components {
                // Names come from the authority: never a path.
                if name.contains(['/', '\\']) || name.starts_with('.') {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(dir.join(name)) {
                    files.insert(name.clone(), builtpanel_license::sha256_hex(&bytes));
                }
            }
            client.integrity_report(&scan, &files)
        })
        .await
        .map_err(|_| ServiceError::Cancelled)?
        .map_err(ServiceError::from)?;
        self.apply_integrity(&result.integrity).await?;
        Ok(result)
    }

    /// What the authority says about this installation's files decides whether management is
    /// restricted: a failed (or still recovering) installation is, a cleared one is not.
    async fn apply_integrity(&self, state: &str) -> Result<(), ServiceError> {
        let mut stored = self.stored().await?;
        let failed = Some("integrity_failed".to_owned());
        let changed = if state == "ok" {
            if stored.refused == failed {
                stored.refused = None;
                true
            } else {
                false
            }
        } else if stored.refused.is_none() {
            stored.refused = failed;
            true
        } else {
            false
        };
        if changed {
            // The license's own summary stays as it is; only what is stored changes.
            let row = store::get(&self.db).await.map_err(|e| db(&e))?;
            store::save_stored(
                &self.db,
                &Kept {
                    stored: &serde_json::to_value(&stored).unwrap_or_default(),
                    license_id: row.license_id,
                    plan: row.plan.as_deref(),
                    license_status: row.license_status.as_deref(),
                    license_expires_at: row.license_expires_at,
                },
            )
            .await
            .map_err(|e| db(&e))?;
            self.evaluate_and_save().await?;
        }
        Ok(())
    }

    /// The loop: a verdict every minute, a heartbeat and a refresh when each is due (a failed
    /// try is repeated after fifteen minutes at most). Returns when `shutdown` turns true.
    pub async fn run(self: Arc<Self>, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let retry = Duration::from_secs(900);
        let (mut next_refresh, mut next_heartbeat) = (Duration::ZERO, Duration::ZERO);
        // The first scan is a little after the start, so a restart does not scan at once.
        let mut next_scan = Duration::from_secs(600);
        let mut elapsed = Duration::ZERO;
        let tick = Duration::from_secs(60);
        loop {
            if self.enabled() {
                if let Err(e) = self.evaluate_and_save().await {
                    tracing::warn!(error = %e, "could not evaluate the license");
                }
                let has_license = self.stored().await.is_ok_and(|s| s.entitlement.is_some());
                if has_license && elapsed >= next_refresh {
                    next_refresh = elapsed
                        + match self.refresh().await {
                            Ok(()) => self.refresh_every,
                            Err(e) => {
                                tracing::warn!(error = %e, "the license could not be refreshed");
                                retry.min(self.refresh_every)
                            }
                        };
                }
                if has_license && elapsed >= next_heartbeat {
                    next_heartbeat = elapsed
                        + match self.heartbeat().await {
                            Ok(()) => self.heartbeat_every,
                            Err(e) => {
                                tracing::warn!(error = %e, "the heartbeat did not get through");
                                retry.min(self.heartbeat_every)
                            }
                        };
                }
                if has_license && elapsed >= next_scan {
                    next_scan = elapsed
                        + match self.integrity_scan().await {
                            Ok(scan) => {
                                if scan.result != "passed" {
                                    tracing::warn!(result = %scan.result, mismatched = ?scan.mismatched, missing = ?scan.missing, "the integrity scan found a difference");
                                }
                                // Until the authority has cleared this installation the scan is
                                // repeated every hour, so a recovery ends soon after it is approved.
                                if scan.integrity == "ok" {
                                    self.integrity_every
                                } else {
                                    Duration::from_secs(3600).min(self.integrity_every)
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "the integrity scan did not get through");
                                retry.min(self.integrity_every)
                            }
                        };
                }
            } else if let Err(e) = self.evaluate_and_save().await {
                tracing::warn!(error = %e, "could not write the license state");
            }
            tokio::select! {
                () = tokio::time::sleep(tick) => elapsed += tick,
                _ = shutdown.changed() => return,
            }
        }
    }
}

async fn count(db: &PgPool, table: &str) -> i64 {
    let query = match table {
        "nodes" => builtpanel_database::sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM nodes WHERE status <> 'revoked'",
        ),
        _ => builtpanel_database::sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM tenants WHERE status <> 'deleting'",
        ),
    };
    query.fetch_one(db).await.unwrap_or(0)
}
