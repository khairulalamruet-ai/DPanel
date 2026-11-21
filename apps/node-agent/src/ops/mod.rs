//! Running operations on the agent (T02.12).
//!
//! An operation frame goes through [`Executor::execute`]:
//!
//! 1. **Parse.** A frame without a usable header is dropped; an operation this
//!    build does not know, or one with bad arguments, gets a typed refusal.
//! 2. **Remembered?** If this `op_id` was answered before, the stored answer is
//!    returned and nothing runs. This is what makes a re-sent operation safe.
//! 3. **Already running?** A second copy of an operation that is still running
//!    is dropped; the first run will answer.
//! 4. **Fresh?** Expired or not-yet-valid envelopes are refused.
//! 5. **Run** the handler registered for the operation type, with a time limit
//!    and a cap on how many run at once. A handler that panics becomes an
//!    `internal` error; it does not take the agent down.
//! 6. **Remember, then answer.** Successes and permanent failures are stored
//!    durably before they are sent. Retryable failures (and expiry, busy) are
//!    not stored, so that the retry really runs.
//!
//! Handlers are plain typed functions registered by operation name. There is no
//! handler that runs a command line, and the registry cannot be extended at
//! run time.

pub mod appinstall;
pub mod backup;
pub mod cache;
pub mod cacheaddon;
pub mod capabilities;
pub mod cron;
pub mod db;
pub mod dbmigrate;
pub mod dbroute;
pub mod dbserver;
pub mod files;
pub mod ftp;
pub mod gitdeploy;
pub mod imagescan;
pub mod logs;
pub mod mail;
pub mod metrics;
pub mod nodeservices;
pub mod pg;
pub mod pgimport;
pub mod pgsplit;
pub mod pma;
pub mod runtimeapp;
pub mod scan;
pub mod server;
pub mod services;
pub mod sqlexport;
pub mod sqlimport;
pub mod sqlsplit;
pub mod tenant;
pub mod traffic;
pub mod transfer;
pub mod vhost;
pub mod webmail;
pub mod wpinstall;

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use builtpanel_node_protocol::{
    AgentError, AgentMessage, ErrorCode, Operation, OperationEnvelope, OperationResult, Output,
    Parsed, encode, parse_operation_frame,
};
use tokio::sync::Semaphore;
use uuid::Uuid;

use self::cache::ResultCache;

/// What a handler returns.
pub type HandlerResult = Result<Output, AgentError>;
type HandlerFuture = Pin<Box<dyn Future<Output = HandlerResult> + Send>>;
type Handler = Box<dyn Fn(Operation) -> HandlerFuture + Send + Sync>;

/// Operations run at once, by default.
pub const DEFAULT_MAX_PARALLEL: usize = 8;
/// Longest a handler may run, whatever the envelope allows.
pub const MAX_RUN_TIME: Duration = Duration::from_secs(300);

/// The handlers this agent has, by operation type name.
#[derive(Default)]
pub struct Registry {
    handlers: HashMap<&'static str, Handler>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The handlers every agent has. `data_dir` is the agent's data directory
    /// (disk figures in the capability report are for its filesystem).
    #[must_use]
    pub fn builtin(data_dir: &std::path::Path) -> Self {
        Self::builtin_with(data_dir, None)
    }

    /// [`Registry::builtin`] plus the tenant operations, when the agent has a
    /// Docker and a privileged helper to run them with.
    #[must_use]
    pub fn builtin_with(
        data_dir: &std::path::Path,
        tenants: Option<tenant::TenantContext>,
    ) -> Self {
        let mut registry = Self::new();
        if let Some(context) = tenants {
            let context = Arc::new(context);
            registry.register("create_tenant", {
                let context = context.clone();
                move |op| tenant::create_tenant(context.clone(), op)
            });
            registry.register("set_up_tenant", {
                let context = context.clone();
                move |op| tenant::set_up_tenant(context.clone(), op)
            });
            registry.register("suspend_tenant", {
                let context = context.clone();
                move |op| tenant::suspend_tenant(context.clone(), op)
            });
            registry.register("resume_tenant", {
                let context = context.clone();
                move |op| tenant::resume_tenant(context.clone(), op)
            });
            registry.register("delete_tenant", {
                let context = context.clone();
                move |op| tenant::delete_tenant(context.clone(), op)
            });
            registry.register("set_resource_limits", {
                let context = context.clone();
                move |op| tenant::set_resource_limits(context.clone(), op)
            });
            registry.register("provision_tenant_stack", {
                let context = context.clone();
                move |op| tenant::provision_tenant_stack(context.clone(), op)
            });
            registry.register("ensure_db_server", {
                let context = context.clone();
                move |op| db::ensure_db_server(context.clone(), op)
            });
            registry.register("create_database", {
                let context = context.clone();
                move |op| dbroute::create_database(context.clone(), op)
            });
            registry.register("drop_database", {
                let context = context.clone();
                move |op| dbroute::drop_database(context.clone(), op)
            });
            registry.register("upsert_db_user", {
                let context = context.clone();
                move |op| dbroute::upsert_db_user(context.clone(), op)
            });
            registry.register("grant_db", {
                let context = context.clone();
                move |op| dbroute::grant_db(context.clone(), op)
            });
            registry.register("revoke_db", {
                let context = context.clone();
                move |op| dbroute::revoke_db(context.clone(), op)
            });
            registry.register("import_sql", {
                let context = context.clone();
                move |op| dbroute::import_sql(context.clone(), op)
            });
            registry.register("export_sql", {
                let context = context.clone();
                move |op| dbroute::export_sql(context.clone(), op)
            });
            registry.register("upsert_ftp_account", {
                let context = context.clone();
                move |op| ftp::upsert_ftp_account(context.clone(), op)
            });
            registry.register("drop_ftp_account", {
                let context = context.clone();
                move |op| ftp::drop_ftp_account(context.clone(), op)
            });
            registry.register("fetch_file", {
                let context = context.clone();
                move |op| transfer::fetch_file(context.clone(), op)
            });
            registry.register("send_file", {
                let context = context.clone();
                move |op| transfer::send_file(context.clone(), op)
            });
            registry.register("create_backup", {
                let context = context.clone();
                move |op| backup::create_backup(context.clone(), op)
            });
            registry.register("restore_backup", {
                let context = context.clone();
                move |op| backup::restore_backup(context.clone(), op)
            });
            registry.register("delete_backup", backup::delete_backup);
            registry.register("export_backup", {
                let context = context.clone();
                move |op| backup::export_backup(context.clone(), op)
            });
            registry.register("import_export", backup::import_export);
            registry.register("report_services", {
                let context = context.clone();
                move |op| nodeservices::report_services(context.clone(), op)
            });
            registry.register("restart_node_service", {
                let context = context.clone();
                move |op| nodeservices::restart_node_service(context.clone(), op)
            });
            registry.register("read_service_log", {
                let context = context.clone();
                move |op| nodeservices::read_service_log(context.clone(), op)
            });
            registry.register("report_server_info", server::report_server_info);
            registry.register("set_cache_instance", {
                let context = context.clone();
                move |op| cacheaddon::set_cache_instance(context.clone(), op)
            });
            registry.register("remove_cache_instance", {
                let context = context.clone();
                move |op| cacheaddon::remove_cache_instance(context.clone(), op)
            });
            registry.register("run_git_deploy", {
                let context = context.clone();
                move |op| gitdeploy::run_git_deploy(context.clone(), op)
            });
            registry.register("remove_git_deploy", {
                let context = context.clone();
                move |op| gitdeploy::remove_git_deploy(context.clone(), op)
            });
            registry.register("set_runtime_app", {
                let context = context.clone();
                move |op| runtimeapp::set_runtime_app(context.clone(), op)
            });
            registry.register("remove_runtime_app", {
                let context = context.clone();
                move |op| runtimeapp::remove_runtime_app(context.clone(), op)
            });
            registry.register("install_app", {
                let context = context.clone();
                move |op| appinstall::install_app(context.clone(), op)
            });
            registry.register("run_wp_cli", {
                let context = context.clone();
                move |op| wpinstall::run_wp_cli(context.clone(), op)
            });
            registry.register("install_wordpress", {
                let context = context.clone();
                move |op| wpinstall::install_wordpress(context.clone(), op)
            });
            registry.register("scan_image", {
                let context = context.clone();
                move |op| imagescan::scan_image(context.clone(), op)
            });
            registry.register("check_image_updates", {
                let context = context.clone();
                move |op| imagescan::check_image_updates(context.clone(), op)
            });
            registry.register("scan_tenant", {
                let context = context.clone();
                move |op| scan::scan_tenant(context.clone(), op)
            });
            registry.register("inventory_wordpress", {
                let context = context.clone();
                move |op| scan::inventory_wordpress(context.clone(), op)
            });
            registry.register("update_malware_signatures", {
                let context = context.clone();
                move |op| scan::update_malware_signatures(context.clone(), op)
            });
            registry.register("set_firewall_rules", {
                let context = context.clone();
                move |op| server::set_firewall_rules(context.clone(), op)
            });
            registry.register("set_timezone", {
                let context = context.clone();
                move |op| server::set_timezone(context.clone(), op)
            });
            registry.register("set_swap", {
                let context = context.clone();
                move |op| server::set_swap(context.clone(), op)
            });
            registry.register("drop_caches", {
                let context = context.clone();
                move |op| server::drop_caches(context.clone(), op)
            });
            registry.register("reboot_node", {
                let context = context.clone();
                move |op| server::reboot_node(context.clone(), op)
            });
            registry.register("list_processes", {
                let context = context.clone();
                move |op| services::list_processes(context.clone(), op)
            });
            registry.register("signal_process", {
                let context = context.clone();
                move |op| services::signal_process(context.clone(), op)
            });
            registry.register("list_containers", {
                let context = context.clone();
                move |op| services::list_containers(context.clone(), op)
            });
            registry.register("restart_container", {
                let context = context.clone();
                move |op| services::restart_container(context.clone(), op)
            });
            registry.register("collect_metrics", {
                let context = context.clone();
                move |op| metrics::collect_metrics(context.clone(), op)
            });
            registry.register("read_log", {
                let context = context.clone();
                move |op| logs::read_log(context.clone(), op)
            });
            registry.register("read_traffic", {
                let context = context.clone();
                move |op| traffic::read_traffic(context.clone(), op)
            });
            registry.register("sync_cron_jobs", {
                let context = context.clone();
                move |op| cron::sync_cron_jobs(context.clone(), op)
            });
            registry.register("run_cron_job", {
                let context = context.clone();
                move |op| cron::run_cron_job(context.clone(), op)
            });
            registry.register("report_cron_runs", {
                let context = context.clone();
                move |op| cron::report_cron_runs(context.clone(), op)
            });
            registry.register("file_op", {
                let context = context.clone();
                move |op| files::file_op(context.clone(), op)
            });
            registry.register("open_phpmyadmin", {
                let context = context.clone();
                move |op| pma::open_phpmyadmin(context.clone(), op)
            });
            registry.register("open_webmail", {
                let context = context.clone();
                move |op| webmail::open_webmail(context.clone(), op)
            });
            registry.register("report_import_progress", {
                let context = context.clone();
                move |op| sqlimport::report_import_progress(context.clone(), op)
            });
            registry.register("migrate_tenant_databases", {
                let context = context.clone();
                move |op| dbmigrate::migrate(context.clone(), op)
            });
            registry.register("remove_dedicated_db", {
                let context = context.clone();
                move |op| dbmigrate::remove(context.clone(), op)
            });
            registry.register("ensure_mail_server", {
                let context = context.clone();
                move |op| mail::ensure_mail_server(context.clone(), op)
            });
            registry.register("set_mail_domain", {
                let context = context.clone();
                move |op| mail::set_mail_domain(context.clone(), op)
            });
            registry.register("remove_mail_domain", {
                let context = context.clone();
                move |op| mail::remove_mail_domain(context.clone(), op)
            });
            registry.register("upsert_mailbox", {
                let context = context.clone();
                move |op| mail::upsert_mailbox(context.clone(), op)
            });
            registry.register("drop_mailbox", {
                let context = context.clone();
                move |op| mail::drop_mailbox(context.clone(), op)
            });
            registry.register("set_mail_forwarders", {
                let context = context.clone();
                move |op| mail::set_mail_forwarders(context.clone(), op)
            });
            registry.register("set_mail_autoresponder", {
                let context = context.clone();
                move |op| mail::set_mail_autoresponder(context.clone(), op)
            });
            registry.register("report_mail", {
                let context = context.clone();
                move |op| mail::report_mail(context.clone(), op)
            });
            registry.register("mail_queue_action", {
                let context = context.clone();
                move |op| mail::mail_queue_action(context.clone(), op)
            });
            registry.register("ensure_pg_server", {
                let context = context.clone();
                move |op| pg::ensure_pg_server(context.clone(), op)
            });
            registry.register("report_pg_sizes", {
                let context = context.clone();
                move |op| pg::report_sizes(context.clone(), op)
            });
            registry.register("report_db_sizes", {
                let context = context.clone();
                move |op| db::report_db_sizes(context.clone(), op)
            });
            registry.register("set_db_remote_hosts", {
                let context = context.clone();
                move |op| dbroute::set_db_remote_hosts(context.clone(), op)
            });
            registry.register("set_db_remote_access", {
                let context = context.clone();
                move |op| dbroute::set_db_remote_access(context.clone(), op)
            });
            registry.register("drop_db_user", {
                let context = context.clone();
                move |op| dbroute::drop_db_user(context.clone(), op)
            });
            registry.register("allow_tenant_databases", {
                let context = context.clone();
                move |op| dbroute::allow_tenant_databases(context.clone(), op)
            });
            registry.register("restart_service", {
                let context = context.clone();
                move |op| tenant::restart_service(context.clone(), op)
            });
            registry.register("set_php_versions", {
                let context = context.clone();
                move |op| tenant::set_php_versions(context.clone(), op)
            });
            registry.register("set_storage_quota", {
                let context = context.clone();
                move |op| tenant::set_storage_quota(context.clone(), op)
            });
            registry.register("attach_domain", {
                let context = context.clone();
                move |op| vhost::attach_domain(context.clone(), op)
            });
            registry.register("set_php_ini", {
                let context = context.clone();
                move |op| vhost::set_php_ini(context.clone(), op)
            });
            registry.register("detach_domain", {
                let context = context.clone();
                move |op| vhost::detach_domain(context.clone(), op)
            });
            registry.register("keep_domains", move |op| {
                vhost::keep_domains(context.clone(), op)
            });
        }
        let sources = capabilities::Sources::system(data_dir);
        registry.register("report_capabilities", move |op| {
            let sources = sources.clone();
            async move {
                match op {
                    Operation::ReportCapabilities => capabilities::collect(&sources)
                        .await
                        .map(Output::Capabilities),
                    _ => Err(wrong_operation()),
                }
            }
        });
        registry.register("ping", |op| async move {
            match op {
                Operation::Ping { nonce } => Ok(Output::Pong { nonce }),
                _ => Err(wrong_operation()),
            }
        });
        registry
    }

    /// Registers the handler for an operation type (see
    /// [`Operation::type_name`]). A later registration for the same type
    /// replaces the earlier one.
    pub fn register<F, Fut>(&mut self, op_type: &'static str, handler: F)
    where
        F: Fn(Operation) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.handlers
            .insert(op_type, Box::new(move |op| Box::pin(handler(op))));
    }

    /// Adds the web server operations, when the agent has a Caddy to drive.
    #[must_use]
    pub fn with_web(mut self, web: Option<Arc<crate::web::WebContext>>) -> Self {
        if let Some(context) = web {
            self.register("set_site_routes", {
                let context = context.clone();
                move |op| crate::web::set_site_routes(context.clone(), op)
            });
            self.register("set_phpmyadmin_host", {
                let context = context.clone();
                move |op| crate::web::set_phpmyadmin_host(context.clone(), op)
            });
            self.register("set_webmail_host", {
                let context = context.clone();
                move |op| crate::web::set_webmail_host(context.clone(), op)
            });
            self.register("set_error_pages", {
                let context = context.clone();
                move |op| crate::web::set_error_pages(context.clone(), op)
            });
            self.register("set_custom_certificates", {
                let context = context.clone();
                move |op| crate::web::set_custom_certificates(context.clone(), op)
            });
            self.register("set_user_agent_blocklist", {
                let context = context.clone();
                move |op| crate::web::set_user_agent_blocklist(context.clone(), op)
            });
            self.register("report_web_service", {
                let context = context.clone();
                move |op| crate::web::report_web_service(context.clone(), op)
            });
            self.register("report_certificates", move |op| {
                crate::web::report_certificates(context.clone(), op)
            });
        }
        self
    }

    /// Operation types that have a handler, sorted.
    #[must_use]
    pub fn types(&self) -> Vec<&'static str> {
        let mut types: Vec<_> = self.handlers.keys().copied().collect();
        types.sort_unstable();
        types
    }
}

/// The error for a handler that was handed an operation of another type
/// (a registration mistake, not something a peer can cause).
#[must_use]
pub fn wrong_operation() -> AgentError {
    AgentError::new(
        ErrorCode::Internal,
        "handler registered for the wrong operation",
    )
}

/// Runs operations exactly once.
pub struct Executor {
    registry: Registry,
    cache: ResultCache,
    running: Mutex<HashSet<Uuid>>,
    slots: Arc<Semaphore>,
}

impl Executor {
    /// An executor over these handlers and this cache.
    #[must_use]
    pub fn new(registry: Registry, cache: ResultCache, max_parallel: usize) -> Arc<Self> {
        Arc::new(Self {
            registry,
            cache,
            running: Mutex::new(HashSet::new()),
            slots: Arc::new(Semaphore::new(max_parallel.max(1))),
        })
    }

    /// The cache (for pruning and status output).
    #[must_use]
    pub fn cache(&self) -> &ResultCache {
        &self.cache
    }

    /// Handles one frame from the control plane and returns the frame to send
    /// back, if any. `now` is Unix seconds on this machine's clock.
    pub async fn execute(self: &Arc<Self>, frame: &str, now: i64) -> Option<String> {
        let result = match parse_operation_frame(frame) {
            // Not an operation (for example a late Welcome): nothing to answer.
            Err(_) => return None,
            Ok(Parsed::Unsupported { header, op_type }) => OperationResult::failure(
                header.op_id,
                AgentError::new(
                    ErrorCode::UnsupportedOperation,
                    format!(
                        "unknown operation `{}`",
                        op_type.chars().take(64).collect::<String>()
                    ),
                ),
            ),
            Ok(Parsed::InvalidArguments { header, message }) => OperationResult::failure(
                header.op_id,
                AgentError::new(ErrorCode::InvalidArguments, message),
            ),
            Ok(Parsed::Known(envelope)) => self.run_once(envelope, now).await?,
        };
        encode(&AgentMessage::Result(result)).ok()
    }

    /// `None` means "say nothing": the operation is running already.
    async fn run_once(
        self: &Arc<Self>,
        envelope: OperationEnvelope,
        now: i64,
    ) -> Option<OperationResult> {
        let op_id = envelope.op_id;
        match self.cache.get(op_id) {
            Ok(Some(remembered)) => return Some(remembered),
            Ok(None) => {}
            // Without the cache we cannot tell whether this ran before. Refusing
            // (retryably) is the only answer that can never run something twice.
            Err(err) => {
                log(&format!("{err}"));
                return Some(OperationResult::failure(
                    op_id,
                    AgentError::new(ErrorCode::Internal, "the operation cache is unavailable")
                        .retryable(),
                ));
            }
        }
        let _running = Running::enter(self, op_id)?;

        if let Err(error) = envelope.check_fresh(now) {
            return Some(OperationResult::failure(op_id, error));
        }
        let Some(handler) = self.registry.handlers.get(envelope.op.type_name()) else {
            return Some(OperationResult::failure(
                op_id,
                AgentError::new(
                    ErrorCode::UnsupportedOperation,
                    "this agent build does not run this operation",
                ),
            ));
        };
        let Ok(_slot) = self.slots.clone().try_acquire_owned() else {
            return Some(OperationResult::failure(
                op_id,
                AgentError::new(ErrorCode::Busy, "too many operations are running").retryable(),
            ));
        };

        let left = u64::try_from(envelope.expires_at.saturating_sub(now)).unwrap_or(0);
        let limit = Duration::from_secs(left).min(MAX_RUN_TIME);
        // In its own task so that a panic is contained.
        let mut task = tokio::spawn(handler(envelope.op));
        let outcome = match tokio::time::timeout(limit, &mut task).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(join)) => Err(AgentError::new(
                ErrorCode::Internal,
                if join.is_panic() {
                    "the operation handler crashed"
                } else {
                    "the operation was cancelled"
                },
            )),
            Err(_) => {
                task.abort();
                Err(
                    AgentError::new(ErrorCode::Timeout, "the operation ran out of time")
                        .retryable(),
                )
            }
        };
        let result = match outcome {
            Ok(output) => OperationResult::success(op_id, output),
            Err(error) => OperationResult::failure(op_id, error),
        };
        let retryable = result.error.as_ref().is_some_and(|e| e.retryable);
        if retryable {
            return Some(result);
        }
        match self.cache.put(&result, now) {
            Ok(stored) => Some(stored),
            Err(err) => {
                // The work is done but cannot be remembered. Say what happened; if
                // it is sent again it may run again, which the log makes visible.
                log(&format!("{err}: result of {op_id} not stored"));
                Some(result)
            }
        }
    }
}

/// Marks an operation as running for as long as it lives.
struct Running<'a> {
    executor: &'a Executor,
    op_id: Uuid,
}

impl<'a> Running<'a> {
    fn enter(executor: &'a Executor, op_id: Uuid) -> Option<Self> {
        // The guard must only exist when the id was newly inserted: creating it
        // and dropping it otherwise would take the lock again (while it is
        // held) and would un-mark the run that is really in progress.
        let newly_inserted = executor
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(op_id);
        if newly_inserted {
            Some(Self { executor, op_id })
        } else {
            None
        }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.executor
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.op_id);
    }
}

/// One line to stderr (the journal under systemd). Never contains secrets.
#[allow(clippy::print_stderr)]
fn log(line: &str) {
    eprintln!("builtpanel-agent: {line}");
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use builtpanel_node_protocol::{ResultStatus, decode};

    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn frame(op_id: Uuid, issued_at: i64, expires_at: i64, op: &str) -> String {
        format!(
            r#"{{"type":"operation","op_id":"{op_id}","idempotency_key":"k","issued_at":{issued_at},"expires_at":{expires_at},"protocol_version":1,"op":{op}}}"#
        )
    }

    fn ping(op_id: Uuid, nonce: u64) -> String {
        frame(
            op_id,
            NOW,
            NOW + 60,
            &format!(r#"{{"type":"ping","nonce":{nonce}}}"#),
        )
    }

    fn result(reply: Option<String>) -> OperationResult {
        match decode::<AgentMessage>(&reply.expect("a reply")).unwrap() {
            AgentMessage::Result(r) => r,
            other => panic!("not a result: {other:?}"),
        }
    }

    /// An executor whose ping handler counts its runs and can be made to fail.
    fn counting(max_parallel: usize) -> (Arc<Executor>, Arc<AtomicUsize>) {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = Registry::new();
        let counter = runs.clone();
        registry.register("ping", move |op| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                match op {
                    // Nonces steer the test handler.
                    Operation::Ping { nonce: 500 } => {
                        Err(AgentError::new(ErrorCode::Internal, "try later").retryable())
                    }
                    Operation::Ping { nonce: 400 } => {
                        Err(AgentError::new(ErrorCode::InvalidArguments, "never"))
                    }
                    Operation::Ping { nonce: 666 } => panic!("handler bug"),
                    Operation::Ping { nonce: 900 } => {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        Ok(Output::Pong { nonce: 900 })
                    }
                    Operation::Ping { nonce } => Ok(Output::Pong { nonce }),
                    _ => Err(wrong_operation()),
                }
            }
        });
        (
            Executor::new(registry, ResultCache::in_memory().unwrap(), max_parallel),
            runs,
        )
    }

    #[tokio::test]
    async fn the_same_operation_twice_runs_once_and_gets_the_same_answer() {
        let (executor, runs) = counting(4);
        let id = Uuid::now_v7();
        let first = result(executor.execute(&ping(id, 7), NOW).await);
        assert_eq!(first.output, Some(Output::Pong { nonce: 7 }));
        // Sent again, even much later and with other contents under the same id.
        let second = result(executor.execute(&ping(id, 7), NOW + 30).await);
        let third = result(executor.execute(&ping(id, 99), NOW + 100_000).await);
        assert_eq!((&second, &third), (&first, &first));
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the handler ran once");
        // Another operation runs normally.
        result(executor.execute(&ping(Uuid::now_v7(), 8), NOW).await);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn permanent_failures_are_remembered_and_retryable_ones_run_again() {
        let (executor, runs) = counting(4);
        let permanent = Uuid::now_v7();
        let a = result(executor.execute(&ping(permanent, 400), NOW).await);
        let b = result(executor.execute(&ping(permanent, 400), NOW).await);
        assert_eq!(a.status, ResultStatus::Failed);
        assert_eq!(a, b);
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        let flaky = Uuid::now_v7();
        let a = result(executor.execute(&ping(flaky, 500), NOW).await);
        assert_eq!(a.status, ResultStatus::Retryable);
        result(executor.execute(&ping(flaky, 500), NOW).await);
        assert_eq!(runs.load(Ordering::SeqCst), 3, "a retry really runs");
    }

    #[tokio::test]
    async fn stale_unknown_and_malformed_operations_are_refused_without_running() {
        let (executor, runs) = counting(4);
        let id = Uuid::now_v7();
        let expired = frame(id, NOW - 120, NOW - 60, r#"{"type":"ping","nonce":1}"#);
        assert_eq!(
            result(executor.execute(&expired, NOW).await)
                .error
                .unwrap()
                .code,
            ErrorCode::Expired
        );
        let early = frame(id, NOW + 3600, NOW + 3660, r#"{"type":"ping","nonce":1}"#);
        assert_eq!(
            result(executor.execute(&early, NOW).await)
                .error
                .unwrap()
                .code,
            ErrorCode::Expired
        );
        // Expiry is not remembered: with a corrected clock the same operation runs.
        let ok = frame(id, NOW, NOW + 60, r#"{"type":"ping","nonce":1}"#);
        assert!(result(executor.execute(&ok, NOW).await).error.is_none());

        let unknown = frame(Uuid::now_v7(), NOW, NOW + 60, r#"{"type":"format_disk"}"#);
        let r = result(executor.execute(&unknown, NOW).await);
        assert_eq!(
            r.error.as_ref().unwrap().code,
            ErrorCode::UnsupportedOperation
        );
        assert!(r.error.unwrap().message.contains("format_disk"));
        // Known to the protocol, but this registry has no handler for it.
        let no_handler = frame(
            Uuid::now_v7(),
            NOW,
            NOW + 60,
            r#"{"type":"report_capabilities"}"#,
        );
        assert_eq!(
            result(executor.execute(&no_handler, NOW).await)
                .error
                .unwrap()
                .code,
            ErrorCode::UnsupportedOperation
        );
        let bad_args = frame(
            Uuid::now_v7(),
            NOW,
            NOW + 60,
            r#"{"type":"ping","nonce":"seven"}"#,
        );
        assert_eq!(
            result(executor.execute(&bad_args, NOW).await)
                .error
                .unwrap()
                .code,
            ErrorCode::InvalidArguments
        );
        assert!(executor.execute("not json", NOW).await.is_none());
        assert!(
            executor
                .execute(
                    r#"{"type":"welcome","protocol_version":1,"server_time":1}"#,
                    NOW
                )
                .await
                .is_none()
        );
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_crashing_handler_is_an_error_and_the_agent_goes_on() {
        let (executor, _) = counting(4);
        let id = Uuid::now_v7();
        let r = result(executor.execute(&ping(id, 666), NOW).await);
        assert_eq!(r.error.as_ref().unwrap().code, ErrorCode::Internal);
        assert!(
            !r.error.unwrap().message.contains("handler bug"),
            "no internals leak"
        );
        assert!(
            result(executor.execute(&ping(Uuid::now_v7(), 1), NOW).await)
                .error
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_copy_of_a_running_operation_is_dropped_and_the_limit_is_enforced() {
        let (executor, runs) = counting(1);
        let slow = Uuid::now_v7();
        let first = tokio::spawn({
            let executor = executor.clone();
            async move { executor.execute(&ping(slow, 900), NOW).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        // The same operation again while it runs: no second run, no second answer.
        assert!(executor.execute(&ping(slow, 900), NOW).await.is_none());
        // Another operation while the only slot is taken: busy, retryable, not remembered.
        let other = Uuid::now_v7();
        let busy = result(executor.execute(&ping(other, 1), NOW).await);
        assert_eq!(busy.error.as_ref().unwrap().code, ErrorCode::Busy);
        assert_eq!(busy.status, ResultStatus::Retryable);

        assert!(result(first.await.unwrap()).error.is_none());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        // The slot is free again and the refused operation now runs.
        assert!(
            result(executor.execute(&ping(other, 1), NOW).await)
                .error
                .is_none()
        );
        // And the slow one is answered from memory.
        assert_eq!(
            result(executor.execute(&ping(slow, 900), NOW).await).output,
            Some(Output::Pong { nonce: 900 })
        );
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn an_operation_that_outlives_its_envelope_times_out() {
        let mut registry = Registry::new();
        registry.register("ping", |_| async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            Ok(Output::Pong { nonce: 0 })
        });
        let executor = Executor::new(registry, ResultCache::in_memory().unwrap(), 2);
        let r = result(
            executor
                .execute(
                    &frame(Uuid::now_v7(), NOW, NOW + 5, r#"{"type":"ping","nonce":1}"#),
                    NOW,
                )
                .await,
        );
        assert_eq!(r.error.as_ref().unwrap().code, ErrorCode::Timeout);
        assert_eq!(r.status, ResultStatus::Retryable);
    }

    #[test]
    fn the_builtin_registry_only_knows_typed_operations() {
        assert_eq!(
            Registry::builtin(std::path::Path::new("/")).types(),
            ["ping", "report_capabilities"]
        );
    }
}
