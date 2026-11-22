//! The dispatcher (T02.11): moves operations from the ledger to nodes and back.
//!
//! Each pass ([`Dispatcher::tick`]):
//! 1. fails operations that expired before they were sent;
//! 2. times out operations that were sent but not answered (the node vanished,
//!    restarted or lost the frame): they are retried after a backoff while
//!    attempts and time remain;
//! 3. claims due operations of the nodes that are connected *right now* and
//!    writes them to the node's session. A frame that cannot be queued puts the
//!    operation back without using an attempt.
//!
//! Results arrive from the gateway ([`Dispatcher::handle_result`]). A result is
//! only accepted from the node the operation was sent to: a node cannot finish
//! another node's operation. Every status change is published on Redis
//! (`bp:ops` and `bp:ops:<op_id>`, a small JSON event without output) so that
//! the panel can update live; a failing publish never affects the ledger.

pub mod dbisolation;
pub mod routes;

use std::time::Duration;

use builtpanel_cache::RedisHandle;
use builtpanel_node_gateway::{Gateway, NodeResult, SendError};
use builtpanel_node_protocol::{
    AgentError, ErrorCode, OperationResult, ResultStatus, encode_operation,
};
use builtpanel_operations::{Operations, Row, Status, envelope};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

/// Redis channel that carries every operation status change.
pub const EVENT_CHANNEL: &str = "bp:ops";

/// Timing and limits.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Time between passes.
    pub poll: Duration,
    /// Most operations claimed per pass.
    pub batch: i64,
    /// How long a sent operation may stay unanswered.
    pub operation_timeout: Duration,
    /// Wait before the first retry; doubles per attempt.
    pub retry_base: Duration,
    /// Longest wait between retries.
    pub retry_cap: Duration,
    /// How often connected nodes are checked for a missing or old capability report.
    pub capabilities_check: Duration,
    /// How old a capability report may get before a new one is requested.
    pub capabilities_max_age: Duration,
    /// How often deleted tenants are checked for data that is due to be purged.
    pub purge_check: Duration,
    /// How often tenants whose web routes changed are looked for.
    pub route_check: Duration,
    /// How often connected nodes are asked which certificates their web server holds.
    pub certificates_every: Duration,
    /// How often certificates that end soon are looked for.
    pub expiry_check: Duration,
    /// How many days before its end a certificate is announced.
    pub expiry_warning_days: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(250),
            batch: 50,
            operation_timeout: Duration::from_secs(60),
            retry_base: Duration::from_secs(2),
            retry_cap: Duration::from_secs(60),
            capabilities_check: Duration::from_secs(30),
            capabilities_max_age: Duration::from_secs(3600),
            purge_check: Duration::from_secs(600),
            route_check: Duration::from_secs(5),
            certificates_every: Duration::from_secs(6 * 3600),
            expiry_check: Duration::from_secs(3600),
            expiry_warning_days: 14,
        }
    }
}

impl Settings {
    /// Wait before delivery number `attempts + 1`, after `attempts` deliveries.
    #[must_use]
    pub fn backoff(&self, attempts: i32) -> Duration {
        let doublings = u32::try_from(attempts.saturating_sub(1).clamp(0, 16)).unwrap_or(0);
        self.retry_base
            .saturating_mul(1_u32 << doublings)
            .min(self.retry_cap)
    }
}

/// What one pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TickStats {
    /// Operations written to a node.
    pub sent: usize,
    /// Operations put back because the node's queue was full or gone.
    pub released: usize,
    /// Operations that timed out waiting for an answer.
    pub timed_out: usize,
    /// Operations that expired unsent.
    pub expired: u64,
}

/// A short number made from a name, for ledger keys that must stay short.
/// A tenant is scanned for malware again after this long (a week).
const SCAN_EVERY_SECS: i64 = 7 * 24 * 3600;

fn short_hash(text: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// What a [`SecretFiller`] answers.
pub type Filled = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<builtpanel_node_protocol::Operation, String>>
            + Send,
    >,
>;

/// Puts back the secrets that are kept out of the ledger: the ledger holds an
/// operation without them, and this adds them to the frame at the moment it is
/// sent. The error text must not contain any secret.
pub type SecretFiller =
    std::sync::Arc<dyn Fn(builtpanel_node_protocol::Operation) -> Filled + Send + Sync>;

/// Sends operations and records results.
#[derive(Clone)]
pub struct Dispatcher {
    ops: Operations,
    gateway: Gateway,
    redis: Option<RedisHandle>,
    settings: Settings,
    secrets: Option<SecretFiller>,
}

impl Dispatcher {
    /// A dispatcher. `redis` is optional: without it no events are published.
    #[must_use]
    pub fn new(
        ops: Operations,
        gateway: Gateway,
        redis: Option<RedisHandle>,
        settings: Settings,
    ) -> Self {
        Self {
            ops,
            gateway,
            redis,
            settings,
            secrets: None,
        }
    }

    /// Adds the hook that fills in secrets when an operation is sent.
    #[must_use]
    pub fn with_secrets(mut self, secrets: SecretFiller) -> Self {
        self.secrets = Some(secrets);
        self
    }

    /// Runs until `shutdown` turns `true`.
    pub async fn run(
        self,
        mut results: mpsc::Receiver<NodeResult>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut tick = tokio::time::interval(self.settings.poll);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut capabilities = tokio::time::interval(self.settings.capabilities_check);
        capabilities.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut purges = tokio::time::interval(self.settings.purge_check);
        purges.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut routes = tokio::time::interval(self.settings.route_check);
        routes.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut certificates = tokio::time::interval(self.settings.capabilities_check);
        certificates.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut mail_reports = tokio::time::interval(Duration::from_secs(60));
        mail_reports.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut expiry = tokio::time::interval(self.settings.expiry_check);
        expiry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stopped(&mut shutdown) => return,
                _ = tick.tick() => { self.tick().await; }
                _ = capabilities.tick() => { self.request_capabilities().await; }
                _ = purges.tick() => { self.request_purges().await; }
                _ = routes.tick() => { self.expire_previews().await; self.request_route_syncs().await; self.request_error_pages().await; self.request_user_agents().await; self.request_firewalls().await; self.request_scans().await; self.request_signature_updates().await; self.request_image_checks().await; self.request_app_installs().await; self.settle_cache_instances().await; self.settle_runtime_apps().await; self.settle_git_deploys().await; self.request_import_progress().await; self.request_cron_runs().await; self.request_metrics().await; self.request_services().await; self.reconcile_db_isolation().await; }
                _ = certificates.tick() => { self.request_certificates().await; self.request_db_sizes().await; }
                _ = mail_reports.tick() => { self.request_mail_reports().await; }
                _ = expiry.tick() => { self.announce_expiring().await; }
                Some(report) = results.recv() => self.handle_result(report).await,
            }
        }
    }

    /// Asks connected nodes whose capability report is missing or old for a new
    /// one. Returns how many were asked. The idempotency key carries the time
    /// slot, so however often this runs a node is asked once per slot.
    pub async fn request_capabilities(&self) -> usize {
        let nodes = self.gateway.connected_nodes();
        if nodes.is_empty() {
            return 0;
        }
        let max_age = self.settings.capabilities_max_age.as_secs().max(1);
        let needing = match builtpanel_database::nodes::needing_capabilities(
            self.ops.db(),
            &nodes,
            i32::try_from(max_age).unwrap_or(i32::MAX),
        )
        .await
        {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!(error = %err, "could not check capability reports");
                return 0;
            }
        };
        let slot = u64::try_from(unix_now()).unwrap_or(0) / max_age;
        let mut asked = 0;
        for node in needing {
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::ReportCapabilities,
                format!("capabilities:{slot}"),
            );
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => asked += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(err) => tracing::warn!(%node, error = %err, "could not request capabilities"),
            }
        }
        asked
    }

    /// Asks the nodes to delete the data of tenants that were deleted at least
    /// `purge_after` ago (their stack is already gone). Returns how many
    /// requests were made. The idempotency key carries the day, so a purge that
    /// failed for good is tried again the next day and never more often; the
    /// listing leaves out tenants that already have a purge queued or running.
    /// When the node reports success, the operations ledger removes the tenant
    /// (migration 17).
    pub async fn request_purges(&self) -> usize {
        let due = match builtpanel_database::tenants::due_for_purge(self.ops.db(), 50).await {
            Ok(due) => due,
            Err(err) => {
                tracing::warn!(error = %err, "could not look for data to purge");
                return 0;
            }
        };
        let day = u64::try_from(unix_now()).unwrap_or(0) / 86_400;
        let mut asked = 0;
        for tenant in due {
            let Ok(uid) = u32::try_from(tenant.uid) else {
                continue;
            };
            let mut request = builtpanel_operations::Request::new(
                tenant.node_id,
                builtpanel_node_protocol::Operation::DeleteTenant {
                    tenant_id: tenant.id,
                    username: tenant.username.clone(),
                    uid,
                    purge_data: true,
                },
                format!("purge:{}:{day}", tenant.id.simple()),
            );
            request.tenant_id = Some(tenant.id);
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => {
                    tracing::info!(tenant = %tenant.id, "purge of deleted tenant's data requested");
                    asked += 1;
                }
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(err) => {
                    tracing::warn!(tenant = %tenant.id, error = %err, "could not request a purge")
                }
            }
        }
        asked
    }

    /// Sends the web routes of every tenant that changed since they were last
    /// sent (its domains, aliases, redirects, suspension, or its own status).
    /// Returns how many tenants were queued.
    ///
    /// The operation's key carries the version and a five-minute slot, so a pass
    /// that repeats makes no second operation, and one that failed for good is
    /// tried again at most every five minutes (the ledger puts the tenant back on
    /// the list when it fails).
    pub async fn request_route_syncs(&self) -> usize {
        let pending = match builtpanel_database::routes::pending(self.ops.db(), 50).await {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(error = %err, "could not look for routes to send");
                return 0;
            }
        };
        let slot = u64::try_from(unix_now()).unwrap_or(0) / 300;
        let mut queued = 0;
        for tenant in pending {
            match self.queue_routes(&tenant, slot).await {
                Ok(true) => queued += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(tenant = %tenant.id, %error, "could not queue the web routes");
                }
            }
        }
        queued
    }

    /// Puts a request in the ledger; `true` when the same request is already there
    /// and has failed (then it waits for the next time slot to be asked afresh).
    async fn enqueue_failed(
        &self,
        request: &builtpanel_operations::Request,
    ) -> Result<bool, String> {
        match self.ops.enqueue(request).await.map_err(|e| e.to_string())? {
            builtpanel_operations::Enqueued::Existing(op) => Ok(op.status == Status::Failed),
            builtpanel_operations::Enqueued::Created(_) => Ok(false),
        }
    }

    /// Whether the account's PHP containers are as they should be; if not, asks
    /// the node and says "not yet" (the account stays on the list).
    async fn php_ready(
        &self,
        tenant: &builtpanel_database::routes::Pending,
        username: &builtpanel_domain::TenantUsername,
        slot: u64,
    ) -> Result<bool, String> {
        let db = self.ops.db();
        let Some(state) = builtpanel_database::php::state(db, tenant.id)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(true);
        };
        if state.state_version == state.applied_version {
            return Ok(true);
        }
        let account = builtpanel_database::tenants::get(db, tenant.id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("the account is gone")?;
        let plan = builtpanel_database::plans::get(db, account.plan_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("the plan is gone")?;
        let extra = builtpanel_database::php::extra_versions(db, tenant.id)
            .await
            .map_err(|e| e.to_string())?;
        let request = builtpanel_operations::Request::new(
            tenant.node_id,
            builtpanel_node_protocol::Operation::SetPhpVersions {
                tenant_id: tenant.id,
                username: username.as_str().to_owned(),
                uid: u32::try_from(account.uid).map_err(|_| "negative uid")?,
                cpu_millis: plan.cpu_millis.and_then(|v| u32::try_from(v).ok()),
                memory_bytes: plan.memory_bytes.and_then(|v| u64::try_from(v).ok()),
                pids: plan.pids.and_then(|v| u32::try_from(v).ok()),
                php_version: state.version,
                extra_php: extra,
            },
            // The migration reads the state back out of this key.
            format!("php:{}:{}:{slot}", tenant.id.simple(), state.state_version),
        );
        let mut request = request;
        request.tenant_id = Some(tenant.id);
        self.enqueue_failed(&request).await?;
        Ok(false)
    }

    async fn queue_routes(
        &self,
        tenant: &builtpanel_database::routes::Pending,
        slot: u64,
    ) -> Result<bool, String> {
        let db = self.ops.db();
        let username = builtpanel_domain::TenantUsername::parse(&tenant.username)
            .map_err(|e| e.to_string())?;
        let upstream =
            builtpanel_tenant_stack::web_upstream(&username).map_err(|e| e.to_string())?;
        // PHP first: the web server files point at containers that must exist.
        // (A suspended account has nothing running: it is brought up to date when
        // it is resumed.)
        if tenant.status == "active" && !self.php_ready(tenant, &username, slot).await? {
            return Ok(false);
        }
        let domains = builtpanel_database::routes::domains(db, tenant.id)
            .await
            .map_err(|e| e.to_string())?;
        let redirects = builtpanel_database::routes::redirects(db, tenant.id)
            .await
            .map_err(|e| e.to_string())?;
        let mut routes = routes::build(&tenant.status, &upstream, &domains, &redirects);
        // The firewall's policy of each site: what the most specific policy says.
        let ids: Vec<Uuid> = domains.iter().map(|d| d.id).collect();
        let policies = builtpanel_database::waf::for_domains(db, tenant.id, &ids)
            .await
            .map_err(|e| e.to_string())?;
        let by_host: std::collections::BTreeMap<String, builtpanel_node_protocol::WafSetting> =
            domains
                .iter()
                .filter_map(|d| {
                    let p = policies.get(&d.id)?;
                    (p.mode != builtpanel_database::waf::Mode::Off).then(|| {
                        (
                            d.fqdn.clone(),
                            builtpanel_node_protocol::WafSetting {
                                mode: p.mode.as_str().to_owned(),
                                disabled_rules: p
                                    .disabled_rules
                                    .iter()
                                    .filter_map(|r| u32::try_from(*r).ok())
                                    .collect(),
                            },
                        )
                    })
                })
                .collect();
        routes::apply_waf(&mut routes, &by_host);
        // The addresses each site turns away.
        let blocks = builtpanel_database::ip_blocks::for_domains(db, tenant.id, &ids)
            .await
            .map_err(|e| e.to_string())?;
        let blocked: std::collections::BTreeMap<String, Vec<String>> = domains
            .iter()
            .filter_map(|d| Some((d.fqdn.clone(), blocks.get(&d.id)?.clone())))
            .collect();
        routes::apply_ip_blocks(&mut routes, &blocked);
        // The uploaded certificates go first, so a site never answers without the
        // certificate it should have. Their keys are not in the ledger.
        let certificates = builtpanel_database::certificates::custom_of_tenant(db, tenant.id)
            .await
            .map_err(|e| e.to_string())?;
        let mut certs = builtpanel_operations::Request::new(
            tenant.node_id,
            builtpanel_node_protocol::Operation::SetCustomCertificates {
                username: tenant.username.clone(),
                items: certificates
                    .into_iter()
                    .map(|c| builtpanel_node_protocol::CustomCertificate {
                        host: c.fqdn,
                        certificate: c.chain_pem,
                        private_key: None,
                    })
                    .collect(),
            },
            format!("certs:{}:{}:{slot}", tenant.id.simple(), tenant.version),
        );
        certs.tenant_id = Some(tenant.id);
        if self.enqueue_failed(&certs).await? {
            return Ok(false);
        }
        // The web server configuration of each site, then the removal of what
        // is no longer a site.
        let files =
            builtpanel_web_config::site_files(db, tenant.id, &tenant.username, &tenant.status)
                .await?;
        for (domain, config) in &files {
            let mut attach = builtpanel_operations::Request::new(
                tenant.node_id,
                builtpanel_node_protocol::Operation::AttachDomain {
                    username: tenant.username.clone(),
                    domain: domain.clone(),
                    config: config.clone(),
                },
                format!(
                    "vhost:{}:{}:{:016x}:{slot}",
                    tenant.id.simple(),
                    tenant.version,
                    short_hash(domain)
                ),
            );
            attach.tenant_id = Some(tenant.id);
            // A configuration the web server refuses must not hold back the
            // routes: the failure puts the tenant on the list again (migration 30).
            self.enqueue_failed(&attach).await?;
        }
        if matches!(tenant.status.as_str(), "active" | "suspended") {
            let mut keep = builtpanel_operations::Request::new(
                tenant.node_id,
                builtpanel_node_protocol::Operation::KeepDomains {
                    username: tenant.username.clone(),
                    domains: files.into_iter().map(|(d, _)| d).collect(),
                },
                format!("keep:{}:{}:{slot}", tenant.id.simple(), tenant.version),
            );
            keep.tenant_id = Some(tenant.id);
            self.enqueue_failed(&keep).await?;
        }
        // The account's php.ini options, within what its plan allows now.
        if matches!(tenant.status.as_str(), "active" | "suspended") {
            let account = builtpanel_database::tenants::get(db, tenant.id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("the account is gone")?;
            let plan = builtpanel_database::plans::get(db, account.plan_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("the plan is gone")?;
            let chosen = builtpanel_database::php::settings(db, tenant.id)
                .await
                .map_err(|e| e.to_string())?;
            let ini = builtpanel_domain::php_ini::IniSettings::from_stored(&chosen)
                .render(&builtpanel_domain::php_ini::Caps::from_limits(&plan.limits));
            // Nothing chosen: the node is told to have no options file.
            let ini = if chosen.is_empty() {
                String::new()
            } else {
                ini
            };
            let mut php_ini = builtpanel_operations::Request::new(
                tenant.node_id,
                builtpanel_node_protocol::Operation::SetPhpIni {
                    username: tenant.username.clone(),
                    ini,
                },
                format!("phpini:{}:{}:{slot}", tenant.id.simple(), tenant.version),
            );
            php_ini.tenant_id = Some(tenant.id);
            self.enqueue_failed(&php_ini).await?;
        }
        let mut request = builtpanel_operations::Request::new(
            tenant.node_id,
            builtpanel_node_protocol::Operation::SetSiteRoutes {
                username: tenant.username.clone(),
                routes,
            },
            format!("routes:{}:{}:{slot}", tenant.id.simple(), tenant.version),
        );
        request.tenant_id = Some(tenant.id);
        match self
            .ops
            .enqueue(&request)
            .await
            .map_err(|e| e.to_string())?
        {
            builtpanel_operations::Enqueued::Created(_) => {}
            // The same request again: fine, unless it already failed (then it waits
            // for the next slot to be asked afresh).
            builtpanel_operations::Enqueued::Existing(op) if op.status == Status::Failed => {
                return Ok(false);
            }
            builtpanel_operations::Enqueued::Existing(_) => {}
        }
        builtpanel_database::routes::mark_synced(db, tenant.id, tenant.version)
            .await
            .map_err(|e| e.to_string())
    }

    /// Removes the preview links that have expired (which puts their accounts on
    /// the list of accounts whose routes are out of date). Returns how many.
    pub async fn expire_previews(&self) -> u64 {
        match builtpanel_database::previews::remove_expired(self.ops.db()).await {
            Ok(n) => n,
            Err(err) => {
                tracing::warn!(error = %err, "could not remove expired preview links");
                0
            }
        }
    }

    /// Sends the error pages to every connected node whenever they changed (the key
    /// is made from their content, so each node is told once per version, and a
    /// node that connects later is told too). Returns how many nodes were told.
    pub async fn request_error_pages(&self) -> usize {
        let custom = match builtpanel_database::error_pages::custom(self.ops.db()).await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "could not read the error pages");
                return 0;
            }
        };
        let version = short_hash(&format!("{custom:?}"));
        let pages: Vec<builtpanel_node_protocol::ErrorPage> = custom
            .into_iter()
            .map(|(kind, html)| builtpanel_node_protocol::ErrorPage { kind, html })
            .collect();
        let mut told = 0;
        for node in self.gateway.connected_nodes() {
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::SetErrorPages {
                    pages: pages.clone(),
                },
                format!("pages:{version:016x}"),
            );
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => told += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(error) => tracing::warn!(%node, %error, "could not queue the error pages"),
            }
        }
        told
    }

    /// Sends the blocked User-Agent texts to every connected node whenever they changed (the key
    /// is made from the list, so each node is told once per version, and a node that connects
    /// later is told too). Returns how many nodes were told.
    pub async fn request_user_agents(&self) -> usize {
        let list = match builtpanel_database::user_agents::list(self.ops.db()).await {
            Ok(l) => l,
            Err(err) => {
                tracing::warn!(error = %err, "could not read the blocked User-Agents");
                return 0;
            }
        };
        let patterns: Vec<String> = list.into_iter().map(|b| b.pattern).collect();
        let version = short_hash(&format!("{patterns:?}"));
        let mut told = 0;
        for node in self.gateway.connected_nodes() {
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::SetUserAgentBlocklist {
                    patterns: patterns.clone(),
                },
                format!("useragents:{version:016x}"),
            );
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => told += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(error) => tracing::warn!(%node, %error, "could not queue the User-Agent list"),
            }
        }
        told
    }

    /// Sends each connected node its firewall policy whenever it changed (the key is made from
    /// the policy: a node is told once per version, and one that connects later is told too).
    /// A node without a policy is left alone. Returns how many nodes were told.
    pub async fn request_firewalls(&self) -> usize {
        let all = match builtpanel_database::node_firewalls::all(self.ops.db()).await {
            Ok(a) => a,
            Err(err) => {
                tracing::warn!(error = %err, "could not read the node firewalls");
                return 0;
            }
        };
        let connected = self.gateway.connected_nodes();
        let mut told = 0;
        for (node, value) in all {
            if !connected.contains(&node) {
                continue;
            }
            let policy: builtpanel_node_protocol::FirewallPolicy =
                match serde_json::from_value(value.clone()) {
                    Ok(p) => p,
                    Err(error) => {
                        tracing::warn!(%node, %error, "a stored firewall policy is unreadable");
                        continue;
                    }
                };
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::SetFirewallRules { policy },
                format!("firewall:{:016x}", short_hash(&value.to_string())),
            );
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => told += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(error) => tracing::warn!(%node, %error, "could not queue the firewall policy"),
            }
        }
        told
    }

    /// Malware scans (T11.09): sends the queued ones to their nodes, writes down the results of
    /// the ones that ended, and starts the scheduled scan of tenants not scanned for a week (at
    /// most two scans are active panel-wide at a time, since each needs about 2 GiB on its
    /// node). Returns how many scans were sent.
    pub async fn request_scans(&self) -> usize {
        let db = self.ops.db();
        // Results first: a finished scan frees a place for a scheduled one.
        match builtpanel_database::malware::running(db).await {
            Ok(running) => {
                for (scan_id, op_id) in running {
                    self.settle_scan(scan_id, op_id).await;
                }
            }
            Err(err) => tracing::warn!(error = %err, "could not look for running scans"),
        }
        if builtpanel_database::malware::active_count(db)
            .await
            .is_ok_and(|n| n < 2)
        {
            for kind in ["malware", "wordpress"] {
                match builtpanel_database::malware::due(db, kind, SCAN_EVERY_SECS, 1).await {
                    Ok(due) => {
                        for tenant_id in due {
                            if let Err(err) = builtpanel_database::malware::request(
                                db, tenant_id, kind, None, false, "schedule", None,
                            )
                            .await
                            {
                                tracing::warn!(%tenant_id, error = %err, "could not queue a scheduled scan");
                            }
                        }
                    }
                    Err(err) => tracing::warn!(error = %err, "could not look for tenants to scan"),
                }
            }
        }
        let queued = match builtpanel_database::malware::queued(db, 20).await {
            Ok(q) => q,
            Err(err) => {
                tracing::warn!(error = %err, "could not read the queued scans");
                return 0;
            }
        };
        let connected = self.gateway.connected_nodes();
        let mut sent = 0;
        for scan in queued {
            if !connected.contains(&scan.node_id) {
                continue;
            }
            let uid = u32::try_from(scan.uid).unwrap_or_default();
            let op = if scan.kind == "wordpress" {
                builtpanel_node_protocol::Operation::InventoryWordPress {
                    tenant_id: scan.tenant_id,
                    uid,
                }
            } else {
                builtpanel_node_protocol::Operation::ScanTenant {
                    tenant_id: scan.tenant_id,
                    uid,
                    folder: scan.folder.clone(),
                    quarantine: scan.quarantine,
                }
            };
            let mut request = builtpanel_operations::Request::new(
                scan.node_id,
                op,
                format!("scan:{}", scan.id.simple()),
            );
            request.tenant_id = Some(scan.tenant_id);
            request.ttl = std::time::Duration::from_secs(3600);
            request.max_attempts = 2;
            match self.ops.enqueue(&request).await {
                Ok(
                    builtpanel_operations::Enqueued::Created(op)
                    | builtpanel_operations::Enqueued::Existing(op),
                ) => {
                    if let Err(err) = builtpanel_database::malware::sent(db, scan.id, op.id).await {
                        tracing::warn!(scan = %scan.id, error = %err, "could not note a sent scan");
                    }
                    sent += 1;
                }
                Err(error) => {
                    tracing::warn!(scan = %scan.id, %error, "could not queue a scan");
                    let _ = builtpanel_database::malware::fail(
                        db,
                        scan.id,
                        "the scan could not be queued",
                    )
                    .await;
                }
            }
        }
        sent
    }

    async fn settle_scan(&self, scan_id: Uuid, op_id: Uuid) {
        let db = self.ops.db();
        let Ok(Some(row)) = self.ops.get(op_id).await else {
            return;
        };
        match row.status {
            builtpanel_database::operations::Status::Succeeded => {
                let output = row.output.and_then(|o| {
                    serde_json::from_value::<builtpanel_node_protocol::Output>(o).ok()
                });
                let result = match output {
                    Some(builtpanel_node_protocol::Output::Scanned {
                        scanned,
                        infected,
                        more,
                    }) => {
                        let findings = serde_json::to_value(&infected).unwrap_or_default();
                        builtpanel_database::malware::finish(
                            db,
                            scan_id,
                            i64::try_from(scanned).unwrap_or(i64::MAX),
                            &findings,
                            more,
                        )
                        .await
                    }
                    _ => {
                        builtpanel_database::malware::fail(
                            db,
                            scan_id,
                            "the node's answer is unreadable",
                        )
                        .await
                    }
                };
                if let Err(err) = result {
                    tracing::warn!(%scan_id, error = %err, "could not keep a scan result");
                }
            }
            builtpanel_database::operations::Status::Failed => {
                let message = row
                    .error
                    .as_ref()
                    .and_then(|e| e.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("the scan failed")
                    .to_owned();
                if let Err(err) = builtpanel_database::malware::fail(db, scan_id, &message).await {
                    tracing::warn!(%scan_id, error = %err, "could not keep a scan failure");
                }
            }
            _ => {}
        }
    }

    /// Image checks (T11.12), on one connected node: what the registries say the allow-list's
    /// tags are now (every six hours) and the vulnerability scan of the image that is due
    /// (one at a time; each image once a day, or when an administrator asked). Returns how many
    /// operations were queued.
    pub async fn request_image_checks(&self) -> usize {
        static SYNCED_SLOT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let db = self.ops.db();
        let now = u64::try_from(unix_now()).unwrap_or(0);
        let Some(node) = self.gateway.connected_nodes().into_iter().min() else {
            return 0;
        };
        let allowlist = match builtpanel_tenant_stack::Allowlist::embedded() {
            Ok(a) => a,
            Err(err) => {
                tracing::warn!(error = %err, "the image list is broken");
                return 0;
            }
        };
        // The rows follow the list shipped with this build (checked every ten minutes).
        let slot = now / 600 + 1;
        if SYNCED_SLOT.swap(slot, std::sync::atomic::Ordering::AcqRel) != slot {
            let entries: Vec<(String, String, Option<String>, String)> = allowlist
                .entries()
                .iter()
                .map(|e| {
                    (
                        e.reference.as_str().to_owned(),
                        e.role.as_str().to_owned(),
                        e.tag.clone(),
                        e.note.clone(),
                    )
                })
                .collect();
            if let Err(err) = builtpanel_database::images::sync(db, &entries).await {
                tracing::warn!(error = %err, "could not keep the image list");
                return 0;
            }
        }
        let mut queued = 0;
        let names: Vec<String> = allowlist
            .entries()
            .iter()
            .filter_map(|e| {
                let (name, _) = e.reference.as_str().split_once('@')?;
                Some(format!("{name}:{}", e.tag.as_deref()?))
            })
            .collect();
        if !names.is_empty() {
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::CheckImageUpdates { images: names },
                format!("imgcheck:{}", now / (6 * 3600)),
            );
            if let Ok(builtpanel_operations::Enqueued::Created(_)) =
                self.ops.enqueue(&request).await
            {
                queued += 1;
            }
        }
        match builtpanel_database::images::next_to_scan(db, 24 * 3600).await {
            Ok(Some(reference)) => {
                let mut request = builtpanel_operations::Request::new(
                    node,
                    builtpanel_node_protocol::Operation::ScanImage {
                        reference: reference.clone(),
                    },
                    format!("imgscan:{:016x}:{}", short_hash(&reference), now / 3600),
                );
                request.ttl = std::time::Duration::from_secs(3600);
                request.max_attempts = 2;
                match self.ops.enqueue(&request).await {
                    Ok(builtpanel_operations::Enqueued::Created(_)) => {
                        queued += 1;
                        let _ = builtpanel_database::images::scan_started(db, &reference).await;
                    }
                    Ok(builtpanel_operations::Enqueued::Existing(_)) => {
                        let _ = builtpanel_database::images::scan_started(db, &reference).await;
                    }
                    Err(error) => tracing::warn!(%error, "could not queue an image scan"),
                }
            }
            Ok(None) => {}
            Err(err) => tracing::warn!(error = %err, "could not look for an image to scan"),
        }
        queued
    }

    async fn store_image_results(
        &self,
        row: &builtpanel_database::operations::Operation,
        output: &builtpanel_node_protocol::Output,
    ) {
        let db = self.ops.db();
        match output {
            builtpanel_node_protocol::Output::ImageScanned {
                critical,
                high,
                medium,
                low,
                top,
            } => {
                let Some(reference) = row
                    .payload
                    .get("reference")
                    .and_then(serde_json::Value::as_str)
                else {
                    return;
                };
                let count = |n: &u32| i32::try_from(*n).unwrap_or(i32::MAX);
                let top = serde_json::to_value(top).unwrap_or_default();
                if let Err(err) = builtpanel_database::images::store_scan(
                    db,
                    reference,
                    [count(critical), count(high), count(medium), count(low)],
                    &top,
                )
                .await
                {
                    tracing::warn!(error = %err, "could not keep an image scan");
                }
            }
            builtpanel_node_protocol::Output::ImageDigests { items } => {
                for item in items {
                    if let Some((name, tag)) = item.image.rsplit_once(':')
                        && let Err(err) =
                            builtpanel_database::images::store_digest(db, name, tag, &item.digest)
                                .await
                    {
                        tracing::warn!(error = %err, "could not keep an image digest");
                    }
                }
            }
            _ => {}
        }
    }

    /// Writes down what the nodes answered about cache servers (T12.10): a server that is up
    /// is `ready` with the name to reach it by, one that could not be made is `failed`, one
    /// that was removed leaves the table. Returns how many rows moved.
    pub async fn settle_cache_instances(&self) -> usize {
        let db = self.ops.db();
        let rows = match builtpanel_database::cache_instances::awaiting(db).await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(error = %err, "could not look for cache servers under way");
                return 0;
            }
        };
        let mut moved = 0;
        for row in rows {
            let Some(op_id) = row.op_id else { continue };
            let Ok(Some(op)) = self.ops.get(op_id).await else {
                continue;
            };
            let result = match op.status {
                builtpanel_database::operations::Status::Succeeded if row.status == "removing" => {
                    builtpanel_database::cache_instances::forget(db, row.id).await
                }
                builtpanel_database::operations::Status::Succeeded => {
                    let answer = op.output.and_then(|o| {
                        serde_json::from_value::<builtpanel_node_protocol::Output>(o).ok()
                    });
                    match answer {
                        Some(builtpanel_node_protocol::Output::CacheInstanceSet { host, port }) => {
                            builtpanel_database::cache_instances::ready(
                                db,
                                row.id,
                                &host,
                                i32::from(port),
                            )
                            .await
                        }
                        _ => {
                            builtpanel_database::cache_instances::failed(
                                db,
                                row.id,
                                "the node's answer is unreadable",
                            )
                            .await
                        }
                    }
                }
                builtpanel_database::operations::Status::Failed => {
                    let message = op
                        .error
                        .as_ref()
                        .and_then(|e| e.get("message"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("the node could not do it")
                        .to_owned();
                    // (A removal that failed leaves the row, with the reason, to try again.)
                    builtpanel_database::cache_instances::failed(db, row.id, &message).await
                }
                _ => continue,
            };
            match result {
                Ok(()) => moved += 1,
                Err(err) => tracing::warn!(error = %err, "could not keep a cache server's state"),
            }
        }
        moved
    }

    /// Writes down what the nodes answered about the applications clients run themselves
    /// (T12.07, T12.08): one that is up is `ready` with the address to reach it by, one that
    /// could not be made is `failed` with the reason, one that was removed leaves the table.
    /// Returns how many rows moved.
    pub async fn settle_runtime_apps(&self) -> usize {
        let db = self.ops.db();
        let rows = match builtpanel_database::runtime_apps::awaiting(db).await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(error = %err, "could not look for applications under way");
                return 0;
            }
        };
        let mut moved = 0;
        for row in rows {
            let Some(op_id) = row.op_id else { continue };
            let Ok(Some(op)) = self.ops.get(op_id).await else {
                continue;
            };
            let result = match op.status {
                builtpanel_database::operations::Status::Succeeded if row.status == "removing" => {
                    builtpanel_database::runtime_apps::forget(db, row.id).await
                }
                builtpanel_database::operations::Status::Succeeded => {
                    let answer = op.output.and_then(|o| {
                        serde_json::from_value::<builtpanel_node_protocol::Output>(o).ok()
                    });
                    match answer {
                        Some(builtpanel_node_protocol::Output::RuntimeAppSet { host }) => {
                            builtpanel_database::runtime_apps::ready(db, row.id, &host).await
                        }
                        _ => {
                            builtpanel_database::runtime_apps::failed(
                                db,
                                row.id,
                                "the node's answer is unreadable",
                            )
                            .await
                        }
                    }
                }
                builtpanel_database::operations::Status::Failed => {
                    let message = op
                        .error
                        .as_ref()
                        .and_then(|e| e.get("message"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("the node could not do it")
                        .to_owned();
                    builtpanel_database::runtime_apps::failed(db, row.id, &message).await
                }
                _ => continue,
            };
            match result {
                Ok(()) => moved += 1,
                Err(err) => tracing::warn!(error = %err, "could not keep an application's state"),
            }
        }
        moved
    }

    /// Writes down what the nodes answered about deploys from git (T12.09): a release that is
    /// live (with its commit and the end of its log), a deploy that failed (with the reason), a
    /// deploy that was removed leaves the table. Returns how many rows moved.
    pub async fn settle_git_deploys(&self) -> usize {
        let db = self.ops.db();
        let rows = match builtpanel_database::git_deploys::awaiting(db).await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(error = %err, "could not look for deploys under way");
                return 0;
            }
        };
        let mut moved = 0;
        for row in rows {
            let Some(op_id) = row.op_id else { continue };
            let Ok(Some(op)) = self.ops.get(op_id).await else {
                continue;
            };
            let result = match op.status {
                builtpanel_database::operations::Status::Succeeded if row.status == "removing" => {
                    builtpanel_database::git_deploys::forget(db, row.id).await
                }
                builtpanel_database::operations::Status::Succeeded => {
                    let answer = op.output.and_then(|o| {
                        serde_json::from_value::<builtpanel_node_protocol::Output>(o).ok()
                    });
                    match answer {
                        Some(builtpanel_node_protocol::Output::GitDeployed { commit, log }) => {
                            builtpanel_database::git_deploys::deployed(db, row.id, &commit, &log)
                                .await
                        }
                        _ => {
                            builtpanel_database::git_deploys::failed(
                                db,
                                row.id,
                                "the node's answer is unreadable",
                            )
                            .await
                        }
                    }
                }
                builtpanel_database::operations::Status::Failed => {
                    let message = op
                        .error
                        .as_ref()
                        .and_then(|e| e.get("message"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("the node could not do it")
                        .to_owned();
                    builtpanel_database::git_deploys::failed(db, row.id, &message).await
                }
                _ => continue,
            };
            match result {
                Ok(()) => moved += 1,
                Err(err) => tracing::warn!(error = %err, "could not keep a deploy's state"),
            }
        }
        moved
    }

    /// Moves the installs of applications along (T12.02): once the database and its user
    /// are ready the user is given the database, once that is done the installer is sent
    /// to the node, and when it has answered the application is `active` or `failed`.
    /// Returns how many operations were queued.
    pub async fn request_app_installs(&self) -> usize {
        let db = self.ops.db();
        let apps = match builtpanel_database::apps::installing(db, 20).await {
            Ok(a) => a,
            Err(err) => {
                tracing::warn!(error = %err, "could not look for installs under way");
                return 0;
            }
        };
        let mut queued = 0;
        for app in apps {
            match self.step_install(&app).await {
                Ok(true) => queued += 1,
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(app = %app.id, error = %err, "an install could not be moved on");
                }
            }
        }
        queued
    }

    async fn fail_app(
        &self,
        app: &builtpanel_database::apps::Installing,
        why: &str,
    ) -> Result<bool, String> {
        let db = self.ops.db();
        let why: String = why.chars().filter(|c| !c.is_control()).take(400).collect();
        builtpanel_database::apps::set_status(db, app.id, "failed", None, Some(&why))
            .await
            .map_err(|e| e.to_string())?;
        builtpanel_database::apps::forget_secrets(db, app.id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(false)
    }

    async fn step_install(
        &self,
        app: &builtpanel_database::apps::Installing,
    ) -> Result<bool, String> {
        let db = self.ops.db();
        let text = |key: &str| {
            app.settings
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let id_of = |key: &str| text(key).and_then(|s| s.parse::<Uuid>().ok());
        let mut settings = app.settings.clone();
        match text("stage").as_deref() {
            Some("setup") => {
                let (Some(db_id), Some(user_id)) = (id_of("db_id"), id_of("user_id")) else {
                    return Ok(false);
                };
                let Some(state) = builtpanel_database::apps::setup_state(db, db_id, user_id)
                    .await
                    .map_err(|e| e.to_string())?
                else {
                    return self.fail_app(app, "the database was removed").await;
                };
                if state.database_status == "failed" || state.user_status == "failed" {
                    return self.fail_app(app, "the database could not be made").await;
                }
                if state.database_status != "ready" || state.user_status != "ready" {
                    return Ok(false);
                }
                let mut conn = db.acquire().await.map_err(|e| e.to_string())?;
                builtpanel_database::databases::set_grant(
                    &mut conn,
                    app.tenant_id,
                    db_id,
                    user_id,
                    "all",
                )
                .await
                .map_err(|e| e.to_string())?;
                drop(conn);
                let mut request = builtpanel_operations::Request::new(
                    app.node_id,
                    builtpanel_node_protocol::Operation::GrantDb {
                        database: state.database,
                        user: state.user,
                        access: "all".to_owned(),
                        engine: builtpanel_node_protocol::DbEngine::Mysql,
                        dedicated: None,
                    },
                    format!("appgrant:{}", app.id.simple()),
                );
                request.tenant_id = Some(app.tenant_id);
                let op = match self
                    .ops
                    .enqueue(&request)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    builtpanel_operations::Enqueued::Created(op)
                    | builtpanel_operations::Enqueued::Existing(op) => op,
                };
                settings["stage"] = "grant".into();
                settings["op"] = op.id.to_string().into();
                builtpanel_database::apps::set_settings(db, app.id, &settings)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }
            Some("grant") => {
                // An application with a database waits for the grant to be done; one
                // without goes straight to the installer.
                let has_database = text("database").is_some();
                if has_database {
                    let Some(op_id) = id_of("op") else {
                        return self
                            .fail_app(app, "the install lost track of its operation")
                            .await;
                    };
                    let Some(row) = self.ops.get(op_id).await.map_err(|e| e.to_string())? else {
                        return self
                            .fail_app(app, "the install lost track of its operation")
                            .await;
                    };
                    match row.status {
                        builtpanel_database::operations::Status::Succeeded => {}
                        builtpanel_database::operations::Status::Failed => {
                            return self
                                .fail_app(app, "the database could not be given to its user")
                                .await;
                        }
                        _ => return Ok(false),
                    }
                }
                let (Some(url), Some(title)) = (text("url"), text("title")) else {
                    return self
                        .fail_app(app, "the install's settings are incomplete")
                        .await;
                };
                let uid = u32::try_from(app.uid).unwrap_or_default();
                let operation = if app.kind == "wordpress" {
                    let (Some(db_name), Some(admin_user), Some(admin_email)) =
                        (text("database"), text("admin_user"), text("admin_email"))
                    else {
                        return self
                            .fail_app(app, "the install's settings are incomplete")
                            .await;
                    };
                    let plugins: Vec<String> = app
                        .settings
                        .get("plugins")
                        .and_then(|p| serde_json::from_value(p.clone()).ok())
                        .unwrap_or_default();
                    builtpanel_node_protocol::Operation::InstallWordPress {
                        app_id: app.id,
                        tenant_id: app.tenant_id,
                        username: app.username.clone(),
                        uid,
                        path: text("folder").unwrap_or_default(),
                        database: builtpanel_node_protocol::WpDatabase {
                            name: db_name.clone(),
                            user: db_name,
                        },
                        // Not in the ledger: opened from the sealed copies at send time.
                        database_password: None,
                        site: builtpanel_node_protocol::WpSite {
                            url,
                            title,
                            admin_user,
                            admin_email,
                        },
                        admin_password: None,
                        version: None,
                        locale: text("locale"),
                        plugins,
                    }
                } else {
                    let Some(definition) = app.settings.get("definition").and_then(|d| {
                        serde_json::from_value::<builtpanel_node_protocol::AppDefinition>(d.clone())
                            .ok()
                    }) else {
                        return self
                            .fail_app(app, "the install's definition is missing")
                            .await;
                    };
                    builtpanel_node_protocol::Operation::InstallApp {
                        app_id: app.id,
                        tenant_id: app.tenant_id,
                        username: app.username.clone(),
                        uid,
                        path: text("folder").unwrap_or_default(),
                        definition: Box::new(definition),
                        database: text("database").map(|name| {
                            builtpanel_node_protocol::WpDatabase {
                                name: name.clone(),
                                user: name,
                            }
                        }),
                        database_password: None,
                        site: builtpanel_node_protocol::AppSite {
                            url,
                            title,
                            admin_user: text("admin_user").unwrap_or_default(),
                            admin_email: text("admin_email").unwrap_or_default(),
                        },
                        admin_password: None,
                    }
                };
                let mut request = builtpanel_operations::Request::new(
                    app.node_id,
                    operation,
                    format!("appinstall:{}", app.id.simple()),
                );
                request.tenant_id = Some(app.tenant_id);
                request.ttl = std::time::Duration::from_secs(3600);
                request.max_attempts = 2;
                let op = match self
                    .ops
                    .enqueue(&request)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    builtpanel_operations::Enqueued::Created(op)
                    | builtpanel_operations::Enqueued::Existing(op) => op,
                };
                settings["stage"] = "install".into();
                settings["op"] = op.id.to_string().into();
                builtpanel_database::apps::set_settings(db, app.id, &settings)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }
            Some("install") => {
                let Some(op_id) = id_of("op") else {
                    return self
                        .fail_app(app, "the install lost track of its operation")
                        .await;
                };
                let Some(row) = self.ops.get(op_id).await.map_err(|e| e.to_string())? else {
                    return self
                        .fail_app(app, "the install lost track of its operation")
                        .await;
                };
                match row.status {
                    builtpanel_database::operations::Status::Succeeded => {
                        let version = row
                            .output
                            .and_then(|o| {
                                serde_json::from_value::<builtpanel_node_protocol::Output>(o).ok()
                            })
                            .and_then(|o| match o {
                                builtpanel_node_protocol::Output::WordPressInstalled {
                                    version,
                                }
                                | builtpanel_node_protocol::Output::AppInstalled { version } => {
                                    Some(version)
                                }
                                _ => None,
                            });
                        builtpanel_database::apps::set_status(
                            db,
                            app.id,
                            "active",
                            version.as_deref(),
                            None,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        builtpanel_database::apps::forget_secrets(db, app.id)
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok(false)
                    }
                    builtpanel_database::operations::Status::Failed => {
                        let message = row
                            .error
                            .as_ref()
                            .and_then(|e| e.get("message"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("the install failed")
                            .to_owned();
                        self.fail_app(app, &message).await
                    }
                    _ => Ok(false),
                }
            }
            _ => Ok(false),
        }
    }

    /// Tells every connected node to bring its malware signatures up to date, once per six
    /// hours (the key carries the time slot). Returns how many nodes were told.
    pub async fn request_signature_updates(&self) -> usize {
        let slot = u64::try_from(unix_now()).unwrap_or(0) / (6 * 3600);
        let mut told = 0;
        for node in self.gateway.connected_nodes() {
            let mut request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::UpdateMalwareSignatures,
                format!("clamsig:{slot}"),
            );
            request.max_attempts = 3;
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => told += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(error) => tracing::warn!(%node, %error, "could not queue the signature update"),
            }
        }
        told
    }

    /// Asks each connected node for the certificates its web server holds, once per
    /// `certificates_every` (the key carries the time slot, so however often this
    /// runs a node is asked once per slot). A node without a web server answers
    /// "unsupported", which is recorded and not asked about again until the next slot.
    /// Returns how many nodes were asked.
    pub async fn request_certificates(&self) -> usize {
        let slot = u64::try_from(unix_now()).unwrap_or(0)
            / self.settings.certificates_every.as_secs().max(1);
        let mut asked = 0;
        for node in self.gateway.connected_nodes() {
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::ReportCertificates,
                format!("certificates:{slot}"),
            );
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => asked += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(err) => tracing::warn!(%node, error = %err, "could not request certificates"),
            }
        }
        asked
    }

    /// Asks each connected node for the sizes of its databases, once per
    /// `certificates_every` (the key carries the time slot). A node without a
    /// database server answers "unsupported" or an error, which is recorded and
    /// not asked about again until the next slot. Returns how many nodes were asked.
    pub async fn request_db_sizes(&self) -> usize {
        let slot = u64::try_from(unix_now()).unwrap_or(0)
            / self.settings.certificates_every.as_secs().max(1);
        let mut asked = 0;
        for node in self.gateway.connected_nodes() {
            // Only nodes that have databases are worth asking, once for each kind of server.
            for (engine, op, key) in [
                (
                    "mysql",
                    builtpanel_node_protocol::Operation::ReportDbSizes,
                    "dbsizes",
                ),
                (
                    "postgres",
                    builtpanel_node_protocol::Operation::ReportPgSizes,
                    "pgsizes",
                ),
            ] {
                match builtpanel_database::databases::node_has_databases(
                    self.ops.db(),
                    node,
                    engine,
                )
                .await
                {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(err) => {
                        tracing::warn!(%node, error = %err, "could not look for databases");
                        continue;
                    }
                }
                let request =
                    builtpanel_operations::Request::new(node, op, format!("{key}:{slot}"));
                match self.ops.enqueue(&request).await {
                    Ok(builtpanel_operations::Enqueued::Created(_)) => asked += 1,
                    Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                    Err(error) => {
                        tracing::warn!(%node, %error, "could not queue the size report");
                    }
                }
            }
        }
        asked
    }

    /// Asks each connected node whose mail server has domains what it holds: the size of every
    /// mailbox and the outgoing queue (the key has a minute slot). Returns how many nodes were
    /// asked.
    pub async fn request_mail_reports(&self) -> usize {
        let slot = u64::try_from(unix_now()).unwrap_or(0) / 60;
        let mut asked = 0;
        for node in self.gateway.connected_nodes() {
            match builtpanel_database::mail::node_has_mail(self.ops.db(), node).await {
                Ok(true) => {}
                Ok(false) => continue,
                Err(err) => {
                    tracing::warn!(%node, error = %err, "could not look for mail domains");
                    continue;
                }
            }
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::ReportMail,
                format!("mailreport:{slot}"),
            );
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => asked += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(error) => tracing::warn!(%node, %error, "could not queue the mail report"),
            }
        }
        asked
    }

    async fn store_mail_report(
        &self,
        node_id: Uuid,
        usage: &[builtpanel_node_protocol::MailUsage],
        queue: &[builtpanel_node_protocol::MailQueueItem],
        total: u64,
    ) {
        let sizes: Vec<(String, i64)> = usage
            .iter()
            .map(|u| {
                (
                    u.address.clone(),
                    i64::try_from(u.used_bytes).unwrap_or(i64::MAX),
                )
            })
            .collect();
        let items = serde_json::to_value(queue).unwrap_or_default();
        let result = async {
            builtpanel_database::mail::set_usage(self.ops.db(), node_id, &sizes).await?;
            builtpanel_database::mail::save_queue(
                self.ops.db(),
                node_id,
                &items,
                i64::try_from(total).unwrap_or(i64::MAX),
            )
            .await
        }
        .await;
        if let Err(err) = result {
            tracing::warn!(%node_id, error = %err, "could not keep what a node's mail server reported");
        }
    }

    /// Asks every connected node how its core services are (the key has a thirty-second slot).
    /// A service that stopped is announced once, within about a minute. Returns how many
    /// operations were queued.
    pub async fn request_services(&self) -> usize {
        let slot = u64::try_from(unix_now()).unwrap_or(0) / 30;
        let mut asked = 0;
        for node in self.gateway.connected_nodes() {
            for (op, key) in [
                (
                    builtpanel_node_protocol::Operation::ReportServices,
                    "services",
                ),
                (
                    builtpanel_node_protocol::Operation::ReportWebService,
                    "webservice",
                ),
            ] {
                let request =
                    builtpanel_operations::Request::new(node, op, format!("{key}:{slot}"));
                match self.ops.enqueue(&request).await {
                    Ok(builtpanel_operations::Enqueued::Created(_)) => asked += 1,
                    Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                    Err(error) => {
                        tracing::warn!(%node, %error, "could not queue the services report")
                    }
                }
            }
        }
        asked
    }

    /// Asks every connected node for its usage readings (the key has a one-minute slot, so a
    /// node is asked once a minute however often this runs). Returns how many were asked.
    pub async fn request_metrics(&self) -> usize {
        let slot = u64::try_from(unix_now()).unwrap_or(0) / 60;
        let mut asked = 0;
        for node in self.gateway.connected_nodes() {
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::CollectMetrics,
                format!("metrics:{slot}"),
            );
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => asked += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(error) => tracing::warn!(%node, %error, "could not queue the metrics reading"),
            }
        }
        asked
    }

    /// Asks the nodes that have accounts with cron jobs for the runs of the last half hour
    /// (the key has a thirty-second slot). The answers become the jobs' last results (a
    /// trigger of the ledger). Returns how many were asked.
    pub async fn request_cron_runs(&self) -> usize {
        let nodes = match builtpanel_database::cron_jobs::nodes_with_jobs(self.ops.db()).await {
            Ok(n) => n,
            Err(err) => {
                tracing::warn!(error = %err, "could not look for cron jobs");
                return 0;
            }
        };
        let now = unix_now();
        let slot = u64::try_from(now).unwrap_or(0) / 30;
        let connected = self.gateway.connected_nodes();
        let mut asked = 0;
        for node in nodes.into_iter().filter(|n| connected.contains(n)) {
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::ReportCronRuns { since: now - 1800 },
                format!("cronruns:{slot}"),
            );
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => asked += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(error) => tracing::warn!(%node, %error, "could not queue the cron report"),
            }
        }
        asked
    }

    /// Asks the nodes that run an SQL import how far they have come (the key has a
    /// ten-second slot, so each import is asked about once per slot). Returns how
    /// many were asked.
    pub async fn request_import_progress(&self) -> usize {
        let running = match builtpanel_database::imports::running(self.ops.db()).await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(error = %err, "could not look for running imports");
                return 0;
            }
        };
        let slot = u64::try_from(unix_now()).unwrap_or(0) / 10;
        let connected = self.gateway.connected_nodes();
        let mut asked = 0;
        for (import_id, node) in running {
            if !connected.contains(&node) {
                continue;
            }
            let request = builtpanel_operations::Request::new(
                node,
                builtpanel_node_protocol::Operation::ReportImportProgress { import_id },
                format!("importprog:{}:{slot}", import_id.simple()),
            );
            match self.ops.enqueue(&request).await {
                Ok(builtpanel_operations::Enqueued::Created(_)) => asked += 1,
                Ok(builtpanel_operations::Enqueued::Existing(_)) => {}
                Err(error) => {
                    tracing::warn!(%node, %error, "could not queue an import progress request")
                }
            }
        }
        asked
    }

    async fn store_import_progress(
        &self,
        import_id: Uuid,
        report: &builtpanel_node_protocol::Output,
    ) {
        let builtpanel_node_protocol::Output::ImportProgress {
            stage,
            bytes_done,
            statements,
            ..
        } = report
        else {
            return;
        };
        let result = if stage == "unknown" {
            // The node has no such import: it restarted, so the run is lost. (A
            // young import may only not have been noticed by the node yet.)
            builtpanel_database::imports::lost(self.ops.db(), import_id).await
        } else {
            builtpanel_database::imports::record_progress(
                self.ops.db(),
                import_id,
                stage,
                i64::try_from(*bytes_done).unwrap_or(i64::MAX),
                i64::try_from(*statements).unwrap_or(i64::MAX),
            )
            .await
        };
        if let Err(err) = result {
            tracing::warn!(%import_id, error = %err, "could not keep the progress of an import");
        }
    }

    async fn store_services(
        &self,
        node_id: Uuid,
        items: &[builtpanel_node_protocol::ServiceStatus],
    ) {
        let result = async {
            let mut tx = self.ops.db().begin().await?;
            for item in items {
                let change = builtpanel_database::node_services::record(
                    &mut tx,
                    node_id,
                    &item.name,
                    &item.state,
                    &item.detail,
                )
                .await?;
                let event = match change {
                    builtpanel_database::node_services::Change::Down => {
                        Some(builtpanel_kernel::Event::NodeServiceDown {
                            node_id,
                            service: item.name.clone(),
                            state: item.state.clone(),
                        })
                    }
                    builtpanel_database::node_services::Change::Recovered => {
                        Some(builtpanel_kernel::Event::NodeServiceRecovered {
                            node_id,
                            service: item.name.clone(),
                        })
                    }
                    builtpanel_database::node_services::Change::None => None,
                };
                if let Some(event) = &event {
                    builtpanel_events::publish(&mut tx, event)
                        .await
                        .map_err(|e| builtpanel_database::sqlx::Error::Protocol(e.to_string()))?;
                }
            }
            tx.commit().await?;
            Ok::<_, builtpanel_database::sqlx::Error>(())
        }
        .await;
        if let Err(err) = result {
            tracing::warn!(%node_id, error = %err, "could not keep the services a node reported");
        }
    }

    async fn store_metrics(
        &self,
        node_id: Uuid,
        at: i64,
        node: &builtpanel_node_protocol::NodeMetrics,
        containers: &[builtpanel_node_protocol::ContainerMetrics],
    ) {
        let containers: Vec<builtpanel_database::usage::Container> = containers
            .iter()
            .map(|c| builtpanel_database::usage::Container {
                name: c.name.clone(),
                memory_bytes: c.memory_bytes,
                cpu_nanos: c.cpu_nanos,
                pids: c.pids,
                net_rx: c.net_rx_bytes,
                net_tx: c.net_tx_bytes,
                io_read: c.io_read_bytes,
                io_write: c.io_write_bytes,
            })
            .collect();
        let node = builtpanel_database::usage::Node {
            cpus: node.cpus,
            busy_jiffies: node.cpu_busy_jiffies,
            total_jiffies: node.cpu_total_jiffies,
            memory_used: node
                .memory_total_bytes
                .saturating_sub(node.memory_available_bytes),
            memory_total: node.memory_total_bytes,
            swap_used: node.swap_used_bytes,
            load1_x100: node.load_x100[0],
            disk_used: node.disk_used_bytes,
            disk_total: node.disk_total_bytes,
        };
        if let Err(err) =
            builtpanel_database::usage::record(self.ops.db(), node_id, at, &node, &containers).await
        {
            tracing::warn!(%node_id, error = %err, "could not keep a usage reading");
        }
    }

    async fn store_db_sizes(&self, node_id: Uuid, items: &[builtpanel_node_protocol::DbSize]) {
        let sizes: Vec<(String, i64)> = items
            .iter()
            .map(|d| (d.name.clone(), i64::try_from(d.bytes).unwrap_or(i64::MAX)))
            .collect();
        let result = async {
            let mut tx = self.ops.db().begin().await?;
            let n = builtpanel_database::databases::record_sizes(&mut tx, node_id, &sizes).await?;
            tx.commit().await?;
            Ok::<_, builtpanel_database::sqlx::Error>(n)
        }
        .await;
        if let Err(err) = result {
            tracing::warn!(%node_id, error = %err, "could not keep the database sizes a node reported");
        }
    }

    async fn store_certificates(
        &self,
        node_id: Uuid,
        items: &[builtpanel_node_protocol::CertificateInfo],
    ) {
        let reported: Vec<builtpanel_database::certificates::Reported<'_>> = items
            .iter()
            .map(|c| builtpanel_database::certificates::Reported {
                host: &c.host,
                issuer: &c.issuer,
                not_before: c.not_before,
                not_after: c.not_after,
            })
            .collect();
        let result = async {
            let mut tx = self.ops.db().begin().await?;
            let n = builtpanel_database::certificates::record_reported(&mut tx, node_id, &reported)
                .await?;
            tx.commit().await?;
            Ok::<_, builtpanel_database::sqlx::Error>(n)
        }
        .await;
        if let Err(err) = result {
            tracing::warn!(%node_id, error = %err, "could not keep the certificates a node reported");
        }
    }

    /// Announces certificates that end within `expiry_warning_days` days (or have
    /// ended) and have not been announced: one `certificate_expiring` event each,
    /// written in the same transaction that records it as announced, so none is
    /// lost or sent twice. Returns how many were announced.
    pub async fn announce_expiring(&self) -> usize {
        let result = async {
            let mut tx = self.ops.db().begin().await.map_err(|e| e.to_string())?;
            let due = builtpanel_database::certificates::expiring(
                &mut tx,
                self.settings.expiry_warning_days,
                200,
            )
            .await
            .map_err(|e| e.to_string())?;
            for c in &due {
                builtpanel_events::publish(
                    &mut tx,
                    &builtpanel_kernel::Event::CertificateExpiring {
                        tenant_id: c.tenant_id,
                        domain_id: c.domain_id,
                        name: c.fqdn.clone(),
                        not_after: c.not_after,
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
                builtpanel_database::certificates::mark_notified(&mut tx, c.domain_id)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().await.map_err(|e| e.to_string())?;
            Ok::<_, String>(due.len())
        }
        .await;
        match result {
            Ok(n) => n,
            Err(error) => {
                tracing::warn!(%error, "could not announce expiring certificates");
                0
            }
        }
    }

    /// One pass; see the module documentation.
    pub async fn tick(&self) -> TickStats {
        let mut stats = TickStats::default();
        match self.ops.expire_overdue().await {
            Ok(n) => stats.expired = n,
            Err(err) => tracing::warn!(error = %err, "could not expire operations"),
        }
        let retry_in = self.settings.backoff(1);
        match self
            .ops
            .time_out_dispatched(self.settings.operation_timeout, retry_in)
            .await
        {
            Ok(timed_out) => {
                stats.timed_out = timed_out.len();
                for (id, status) in timed_out {
                    tracing::warn!(op_id = %id, ?status, "operation timed out waiting for the node");
                    self.publish(id, None, status).await;
                }
            }
            Err(err) => tracing::warn!(error = %err, "could not time out operations"),
        }

        let nodes = self.gateway.connected_nodes();
        if nodes.is_empty() {
            return stats;
        }
        let due = match self.ops.claim_due(&nodes, self.settings.batch).await {
            Ok(due) => due,
            Err(err) => {
                tracing::warn!(error = %err, "could not claim operations");
                return stats;
            }
        };
        let now = unix_now();
        for op in due {
            match self.deliver(&op, now).await {
                Delivery::Sent => stats.sent += 1,
                Delivery::Released => stats.released += 1,
                Delivery::Rejected => {}
            }
        }
        stats
    }

    /// The frame for `op`: the stored operation with its secrets put back.
    async fn frame(&self, op: &Row, now: i64) -> Result<String, String> {
        let mut env = envelope(op, now).map_err(|e| e.to_string())?;
        if let Some(fill) = &self.secrets {
            env.op = fill(env.op).await?;
        }
        encode_operation(&env).map_err(|e| e.to_string())
    }

    async fn deliver(&self, op: &Row, now: i64) -> Delivery {
        let frame = match self.frame(op, now).await {
            Ok(frame) => frame,
            Err(reason) => {
                // A ledger row this build cannot express (for example written by a newer
                // control plane): trying again cannot help.
                tracing::error!(op_id = %op.id, %reason, "operation cannot be encoded");
                let error = serde_json::json!({
                    "code": "internal", "message": "the operation could not be encoded", "retryable": false
                });
                self.finish_failed(op.id, op.node_id, &error, None).await;
                return Delivery::Rejected;
            }
        };
        match self.gateway.send(op.node_id, frame) {
            Ok(()) => {
                self.publish(op.id, Some(op.node_id), Status::Dispatched)
                    .await;
                Delivery::Sent
            }
            Err(err @ (SendError::NotConnected | SendError::Busy)) => {
                tracing::debug!(op_id = %op.id, %err, "operation put back");
                if let Err(db) = self.ops.release_undelivered(op.id).await {
                    tracing::warn!(op_id = %op.id, error = %db, "could not put the operation back");
                }
                Delivery::Released
            }
        }
    }

    /// Records a result reported by `report.node_id`.
    pub async fn handle_result(&self, report: NodeResult) {
        let NodeResult { node_id, result } = report;
        let OperationResult {
            op_id,
            status,
            output,
            error,
        } = result;
        let row = match self.ops.get(op_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                tracing::warn!(%node_id, %op_id, "result for an unknown operation ignored");
                return;
            }
            Err(err) => {
                tracing::warn!(%node_id, %op_id, error = %err, "could not look up an operation");
                return;
            }
        };
        if row.node_id != node_id {
            tracing::warn!(%node_id, %op_id, owner = %row.node_id, "result from the wrong node ignored");
            return;
        }
        let error_json = || {
            let error = error.clone().unwrap_or_else(|| {
                AgentError::new(
                    ErrorCode::Internal,
                    "the node reported a failure without detail",
                )
            });
            serde_json::to_value(error).unwrap_or(serde_json::Value::Null)
        };
        match status {
            ResultStatus::Succeeded => {
                if let Some(builtpanel_node_protocol::Output::Capabilities(report)) = &output {
                    self.store_capabilities(node_id, report).await;
                }
                if let Some(builtpanel_node_protocol::Output::Certificates { items }) = &output {
                    self.store_certificates(node_id, items).await;
                }
                if let Some(builtpanel_node_protocol::Output::DbSizes { items }) = &output {
                    self.store_db_sizes(node_id, items).await;
                }
                if let Some(builtpanel_node_protocol::Output::MailReport {
                    usage,
                    queue,
                    queue_total,
                }) = &output
                {
                    self.store_mail_report(node_id, usage, queue, *queue_total)
                        .await;
                }
                if let Some(builtpanel_node_protocol::Output::Services { items }) = &output {
                    self.store_services(node_id, items).await;
                }
                if let Some(builtpanel_node_protocol::Output::Metrics {
                    at,
                    node,
                    containers,
                }) = &output
                {
                    self.store_metrics(node_id, *at, node, containers).await;
                }
                if let Some(builtpanel_node_protocol::Output::File {
                    result:
                        builtpanel_node_protocol::files::FileResult::Usage {
                            total_bytes,
                            inodes,
                            folders,
                            partial,
                        },
                }) = &output
                    && let Some(tenant_id) = row.tenant_id
                {
                    let folders = serde_json::to_value(folders).unwrap_or_default();
                    if let Err(err) = builtpanel_database::file_transfers::record_usage(
                        self.ops.db(),
                        tenant_id,
                        i64::try_from(*total_bytes).unwrap_or(i64::MAX),
                        i64::try_from(*inodes).unwrap_or(i64::MAX),
                        *partial,
                        &folders,
                    )
                    .await
                    {
                        tracing::warn!(%tenant_id, error = %err, "could not keep a disk usage measurement");
                    }
                }
                if let Some(
                    result @ (builtpanel_node_protocol::Output::ImageScanned { .. }
                    | builtpanel_node_protocol::Output::ImageDigests { .. }),
                ) = &output
                {
                    self.store_image_results(&row, result).await;
                }
                if let Some(report @ builtpanel_node_protocol::Output::ImportProgress { .. }) =
                    &output
                    && let Some(import_id) = row
                        .payload
                        .get("import_id")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|s| s.parse::<Uuid>().ok())
                {
                    self.store_import_progress(import_id, report).await;
                }
                let output = output
                    .and_then(|o| serde_json::to_value(o).ok())
                    .unwrap_or_else(|| serde_json::json!({}));
                match self.ops.succeed(op_id, &output).await {
                    Ok(true) => self.publish(op_id, Some(node_id), Status::Succeeded).await,
                    Ok(false) => tracing::debug!(%op_id, "late or repeated result ignored"),
                    Err(err) => tracing::warn!(%op_id, error = %err, "could not record a result"),
                }
            }
            ResultStatus::Failed => {
                self.finish_failed(op_id, node_id, &error_json(), None)
                    .await
            }
            ResultStatus::Retryable => {
                let wait = self.settings.backoff(row.attempts);
                self.finish_failed(op_id, node_id, &error_json(), Some(wait))
                    .await;
            }
        }
    }

    /// Keeps a node's capability report. The report was parsed into the typed
    /// form on arrival, so only known fields are stored; free text is clipped.
    async fn store_capabilities(
        &self,
        node_id: Uuid,
        report: &builtpanel_node_protocol::Capabilities,
    ) {
        let clip =
            |s: &str| -> String { s.chars().filter(|c| !c.is_control()).take(128).collect() };
        let mut report = report.clone();
        report.agent_version = clip(&report.agent_version);
        report.kernel = clip(&report.kernel);
        report.docker_version = report.docker_version.as_deref().map(clip);
        let Ok(json) = serde_json::to_value(&report) else {
            return;
        };
        if let Err(err) =
            builtpanel_database::nodes::set_capabilities(self.ops.db(), node_id, &json).await
        {
            tracing::warn!(%node_id, error = %err, "could not store the capability report");
        }
    }

    async fn finish_failed(
        &self,
        id: Uuid,
        node_id: Uuid,
        error: &serde_json::Value,
        retry_in: Option<Duration>,
    ) {
        match self.ops.fail(id, error, retry_in).await {
            Ok(Some(status)) => self.publish(id, Some(node_id), status).await,
            Ok(None) => tracing::debug!(op_id = %id, "late or repeated failure ignored"),
            Err(err) => tracing::warn!(op_id = %id, error = %err, "could not record a failure"),
        }
    }

    async fn publish(&self, id: Uuid, node_id: Option<Uuid>, status: Status) {
        let Some(redis) = &self.redis else { return };
        let event = serde_json::json!({
            "op_id": id,
            "node_id": node_id,
            "status": status.as_str(),
        })
        .to_string();
        for channel in [EVENT_CHANNEL.to_owned(), format!("{EVENT_CHANNEL}:{id}")] {
            if let Err(err) = redis.publish(&channel, &event).await {
                tracing::debug!(error = %err, "could not publish an operation event");
                return;
            }
        }
    }
}

async fn stopped(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

enum Delivery {
    Sent,
    Released,
    Rejected,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn retry_waits_double_up_to_the_cap() {
        let s = Settings::default();
        let waits: Vec<u64> = (0..8).map(|a| s.backoff(a).as_secs()).collect();
        assert_eq!(waits, [2, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(s.backoff(i32::MAX), s.retry_cap);
        assert_eq!(s.backoff(-5), s.retry_base);
    }
}
