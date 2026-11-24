//! Built Panel control plane HTTP API (Axum).
//!
//! Startup: load and validate configuration (`builtpanel-config`), initialise
//! telemetry, serve HTTP, shut down gracefully on SIGINT/SIGTERM.

mod app;
#[cfg(test)]
mod import_tests;
mod modules;
mod openapi;
mod secrets;
#[cfg(test)]
mod security_tests;
mod system_backup;

use std::{process::ExitCode, sync::Arc};

use builtpanel_config::ControlConfig;
use builtpanel_http_kit::health::Readiness;
use builtpanel_observability::{LogFormat, TelemetryConfig};

const SERVICE: &str = "control-api";

/// `EX_CONFIG` from sysexits.h: systemd and operators can tell config errors apart.
const EXIT_CONFIG: u8 = 78;

fn telemetry_config(config: &ControlConfig) -> TelemetryConfig {
    let mut t = TelemetryConfig::new(SERVICE);
    t.log_format = match config.log.format {
        builtpanel_config::LogFormat::Json => LogFormat::Json,
        builtpanel_config::LogFormat::Pretty => LogFormat::Pretty,
    };
    t.log_filter.clone_from(&config.log.filter);
    t.otlp_endpoint.clone_from(&config.log.otlp_endpoint);
    t
}

#[tokio::main]
async fn main() -> ExitCode {
    // Config comes first; telemetry is not up yet, so errors go to stderr.
    let config = match ControlConfig::load() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{SERVICE}: {err}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    let telemetry = match builtpanel_observability::init(&telemetry_config(&config)) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("{SERVICE}: failed to initialise telemetry: {err}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    // Module manifests are part of the build; a broken one is a build mistake
    // that must not reach users half-working.
    let registry = match modules::registry() {
        Ok(registry) => {
            tracing::info!(
                modules = %registry.names().collect::<Vec<_>>().join(", "),
                "module registry built"
            );
            registry
        }
        Err(messages) => {
            for message in messages {
                tracing::error!("{message}");
                eprintln!("{SERVICE}: {message}");
            }
            telemetry.shutdown();
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    // Lazy pool: the API starts even if PostgreSQL is down; /ready reports it.
    let db = match config
        .database_url()
        .map_err(|e| e.to_string())
        .and_then(|url| {
            builtpanel_database::connect_lazy(
                &url,
                &builtpanel_database::PoolSettings {
                    max_connections: config.database.max_connections,
                    application_name: SERVICE.to_owned(),
                },
            )
            .map_err(|e| e.to_string())
        }) {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(error = %err, "database configuration invalid");
            telemetry.shutdown();
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    // Redis: URL parsed now, connection opened on first use; /ready reports outages.
    let redis = match config
        .redis_url()
        .map_err(|e| e.to_string())
        .and_then(|url| builtpanel_cache::RedisHandle::new(&url).map_err(|e| e.to_string()))
    {
        Ok(handle) => handle,
        Err(err) => {
            tracing::error!(error = %err, "redis configuration invalid");
            telemetry.shutdown();
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    // Tell the database which modules this build has (retried until it works).
    tokio::spawn(modules::sync_until_done(db.clone(), registry.clone()));

    let readiness = Readiness::default()
        .with_probe("database", {
            let db = db.clone();
            Arc::new(move || {
                let db = db.clone();
                Box::pin(async move {
                    builtpanel_database::ping(&db)
                        .await
                        .map_err(|e| e.to_string())
                })
            })
        })
        .with_probe("redis", {
            let redis = redis.clone();
            Arc::new(move || {
                let redis = redis.clone();
                Box::pin(async move { redis.ping().await.map_err(|e| e.to_string()) })
            })
        });

    let hasher = match builtpanel_auth_core::password::PasswordHasher::new(
        builtpanel_auth_core::password::Argon2Params {
            memory_kib: config.auth.argon2_memory_kib,
            iterations: config.auth.argon2_iterations,
            parallelism: config.auth.argon2_parallelism,
        },
    ) {
        Ok(h) => h,
        Err(err) => {
            tracing::error!(error = %err, "password hashing configuration invalid");
            telemetry.shutdown();
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    let sessions = builtpanel_session::SessionStore::new(
        redis.clone(),
        db.clone(),
        builtpanel_session::SessionConfig {
            idle_timeout: std::time::Duration::from_secs(
                u64::from(config.auth.session_idle_minutes) * 60,
            ),
            absolute_timeout: std::time::Duration::from_secs(
                u64::from(config.auth.session_absolute_hours) * 3600,
            ),
            sudo_window: std::time::Duration::from_secs(u64::from(config.auth.sudo_minutes) * 60),
            ..builtpanel_session::SessionConfig::default()
        },
    );
    let master_key = match config
        .master_key()
        .map_err(|e| e.to_string())
        .and_then(|key| {
            builtpanel_auth_core::secretbox::MasterKey::from_base64(
                secrecy::ExposeSecret::expose_secret(&key),
            )
            .map_err(|e| format!("secrets.master_key: {e}"))
        }) {
        Ok(key) => key,
        Err(err) => {
            tracing::error!(error = %err, "master key unavailable");
            telemetry.shutdown();
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    let operation_secrets = secrets::operation_secrets(db.clone(), master_key.clone());
    let runner_key = master_key.clone();
    let system_key = master_key.clone();
    let notify_key = master_key.clone();
    let notify_sessions = sessions.clone();
    let minutes = |m: u32| std::time::Duration::from_secs(u64::from(m) * 60);
    let lockout = |threshold, lock_minutes| builtpanel_cache::lockout::LockoutPolicy {
        threshold,
        window: minutes(config.auth.lockout_window_minutes),
        lockout: minutes(lock_minutes),
    };
    // No CA yet is fine (enrollment then answers 503); a CA that exists but is
    // unusable (expired, wrong key, key readable by others) must not be ignored.
    let ca = match builtpanel_pki::load_ca_dir(&config.pki.ca_dir, time::OffsetDateTime::now_utc())
    {
        Ok(ca) => {
            tracing::info!(fingerprint = ca.fingerprint_sha256(), "node CA loaded");
            Some(Arc::new(ca))
        }
        Err(builtpanel_pki::PkiError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            tracing::warn!(
                dir = %config.pki.ca_dir.display(),
                "no node CA found: node enrollment is switched off; create one with `bpctl ca init`"
            );
            None
        }
        Err(err) => {
            tracing::error!(error = %err, "node CA unusable");
            telemetry.shutdown();
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    // The agent port: mutual TLS, only with a CA and a server certificate.
    let gateway = match (config.agent.stream_listen, &ca) {
        (None, _) => None,
        (Some(_), None) => {
            tracing::error!(
                "agent.stream_listen is set but there is no node CA: run `bpctl ca init`"
            );
            telemetry.shutdown();
            return ExitCode::from(EXIT_CONFIG);
        }
        (Some(addr), Some(ca)) => {
            let tls = match builtpanel_node_gateway::tls::load_server_config(
                ca.certificate_pem(),
                &config.pki.ca_dir,
            ) {
                Ok(tls) => tls,
                Err(err) => {
                    tracing::error!(error = %err, "agent port TLS unusable: create the server certificate with `bpctl ca issue-server`");
                    telemetry.shutdown();
                    return ExitCode::from(EXIT_CONFIG);
                }
            };
            let listener = match builtpanel_node_gateway::bind(addr).await {
                Ok(l) => l,
                Err(err) => {
                    tracing::error!(%addr, error = %err, "failed to bind the agent port");
                    return ExitCode::FAILURE;
                }
            };
            tracing::info!(%addr, "agent port listening (mutual TLS)");
            Some((
                builtpanel_node_gateway::Gateway::new(
                    db.clone(),
                    tls,
                    builtpanel_node_gateway::Settings::default(),
                ),
                listener,
            ))
        }
    };
    let operations = builtpanel_operations::Operations::new(db.clone());
    let nodes = builtpanel_mod_nodes::NodesState {
        db: db.clone(),
        sessions: sessions.clone(),
        public_url: config.agent.public_url.as_deref().map(Into::into),
        stream_url: config.agent.stream_url.as_deref().map(Into::into),
        token_ttl: std::time::Duration::from_secs(
            u64::from(config.agent.enrollment_token_minutes) * 60,
        ),
        ca,
        limiter: builtpanel_mod_nodes::enrollment_limiter(redis.clone()),
        gateway: gateway.as_ref().map(|(gateway, _)| gateway.clone()),
        operations: operations.clone(),
        audit: builtpanel_audit::AuditWriter::new(db.clone()),
    };
    let admin = builtpanel_mod_admin::AdminState {
        db: db.clone(),
        sessions: sessions.clone(),
        hasher: hasher.clone(),
    };
    let auth = builtpanel_mod_auth::AuthState {
        db: db.clone(),
        sessions,
        hasher,
        limits: builtpanel_mod_auth::LoginLimits::new(
            redis.clone(),
            lockout(config.auth.lockout_threshold, config.auth.lockout_minutes),
            lockout(
                config.auth.ip_lockout_threshold,
                config.auth.ip_lockout_minutes,
            ),
        ),
        trusted_proxies: config.trusted_proxies.clone().into(),
        redis: redis.clone(),
        master_key,
        totp_issuer: config.auth.totp_issuer.trim().into(),
        root_login: config
            .auth
            .root_login_socket
            .clone()
            .map(builtpanel_mod_auth::root_login::RootLogin::new),
        audit: builtpanel_audit::AuditWriter::new(db.clone()),
    };

    let handle = telemetry.prometheus_handle();
    let trusted_proxies: Arc<[std::net::IpAddr]> = config.trusted_proxies.clone().into();
    let base = app::base_router(app::AppState {
        metrics: Arc::new(move || handle.render()),
        readiness,
        trusted_proxies: trusted_proxies.clone(),
        fleet: match config.metrics_token() {
            Ok(Some(token)) => Some(app::FleetMetrics {
                db: db.clone(),
                token: Arc::new(token),
            }),
            Ok(None) => None,
            Err(error) => {
                tracing::error!(%error, "the metrics token cannot be used");
                return ExitCode::from(EXIT_CONFIG);
            }
        },
    });
    // Module routes: core modules at once, the others as soon as the modules
    // table can be read, and again whenever an administrator switches one.
    // The license: without an authority in the configuration nothing is checked.
    let license =
        match builtpanel_mod_license::service::LicenseService::new(db.clone(), &config.license) {
            Ok(l) => Arc::new(l),
            Err(message) => {
                tracing::error!(%message, "the license settings cannot be used");
                telemetry.shutdown();
                return ExitCode::from(EXIT_CONFIG);
            }
        };
    if license.enabled() {
        tracing::info!(installation = ?license.installation_id(), "licensing is on");
    }
    let (mount, settings) = match modules::mount(
        modules::States {
            auth,
            admin,
            nodes,
            spool_dir: config.imports.spool_dir.clone(),
            license: license.clone(),
        },
        base,
    ) {
        Ok(mounted) => mounted,
        Err(messages) => {
            for message in messages {
                tracing::error!("{message}");
            }
            telemetry.shutdown();
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    tracing::info!(
        modules = %settings.modules().map(|m| m.module.as_str()).collect::<Vec<_>>().join(", "),
        "settings schemas valid"
    );
    // What the notification channels are to get is sent every half minute (and the inboxes of what
    // is old are cleared once a day).
    tokio::spawn({
        let state = builtpanel_mod_notifications::NotificationsState {
            db: db.clone(),
            sessions: notify_sessions,
            settings: Arc::new(std::sync::OnceLock::from(settings.clone())),
            master_key: notify_key,
        };
        async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(30));
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut rounds = 0_u32;
            loop {
                every.tick().await;
                builtpanel_mod_notifications::deliver_due(&state).await;
                rounds = rounds.wrapping_add(1);
                if rounds % 2880 == 1
                    && let Err(error) =
                        builtpanel_database::notifications::prune(&state.db, 90).await
                {
                    tracing::warn!(%error, "could not clear the old notifications");
                }
            }
        }
    });
    // Imports that were running when the panel last stopped cannot go on by themselves.
    if let Err(error) = builtpanel_mod_migration::exec::end_interrupted(&db).await {
        tracing::warn!(%error, "could not end the imports that were interrupted");
    }
    // Uploaded dumps nobody needs any more are removed; forgotten uploads are cancelled.
    tokio::spawn({
        let (db, spool_dir) = (db.clone(), config.imports.spool_dir.clone());
        async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(120));
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                if let Err(error) = builtpanel_mod_databases::imports::sweep(&db, &spool_dir).await
                {
                    tracing::warn!(%error, "could not sweep the import spool");
                }
                if let Err(error) = builtpanel_mod_files::transfers::sweep(&db, &spool_dir).await {
                    tracing::warn!(%error, "could not sweep the file transfers");
                }
            }
        }
    });
    // Scheduled backups are made, and the old ones removed (T09.07).
    tokio::spawn({
        let db = db.clone();
        let system = system_backup::Executor::new(
            db.clone(),
            system_key,
            std::sync::Arc::new(config.clone()),
            config.imports.spool_dir.join("system-backup"),
        );
        async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(30));
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
                if let Err(error) =
                    builtpanel_mod_backups::runner::tick(&db, &runner_key, now).await
                {
                    tracing::warn!(%error, "the backup runner could not finish a round");
                }
                if let Err(error) = builtpanel_mod_backups::transfer::tick(&db, &runner_key).await {
                    tracing::warn!(%error, "the node transfers could not finish a round");
                }
                system.run().await;
            }
        }
    });
    // Usage readings are rolled up into hours, and old ones removed (T10.09).
    tokio::spawn({
        let db = db.clone();
        async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(900));
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                if let Err(error) = builtpanel_database::usage::maintain(&db).await {
                    tracing::warn!(%error, "could not roll up the usage readings");
                }
            }
        }
    });
    // The panel's own update: looks for a new release now and then, tells the administrators about
    // it, and installs it by itself when the settings (system module) say so.
    tokio::spawn(builtpanel_mod_system::update::run_auto(
        db.clone(),
        settings.clone(),
        std::sync::Arc::new(builtpanel_mod_system::update::UpdateConfig::from_env()),
    ));
    // What has been in the accounts' trash longer than the setting says goes for good.
    tokio::spawn({
        let (db, settings) = (db.clone(), settings.clone());
        async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(3600));
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                if let Err(error) = builtpanel_mod_files::purge_trash(&db, &settings).await {
                    tracing::warn!(%error, "could not queue the trash purges");
                }
                if let Err(error) = builtpanel_mod_files::measure_usage(&db).await {
                    tracing::warn!(%error, "could not queue the disk usage measurements");
                }
            }
        }
    });
    // Mail: what the mail servers have to be told, the DNS records mail needs, and the hold of a
    // domain that sends too much (T13).
    tokio::spawn({
        let (db, settings) = (db.clone(), settings.clone());
        async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(15));
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                if let Err(error) = builtpanel_mod_mail::tick::tick(&db, &settings).await {
                    tracing::warn!(%error, "the mail tick could not finish a round");
                }
            }
        }
    });
    tokio::spawn(modules::follow_enabled(
        db.clone(),
        mount.clone(),
        std::time::Duration::from_secs(15),
    ));
    // While the license is not good, changes are closed (reading and the license page stay).
    let app = app::router(
        mount.service().layer(axum::middleware::from_fn_with_state(
            builtpanel_mod_license::Restrictor::new(db.clone()),
            builtpanel_mod_license::restrict,
        )),
        trusted_proxies,
    );
    // The event relay: hands outbox events to the enabled modules.
    let relay = builtpanel_events::Relay::new(
        db.clone(),
        {
            let mount = mount.clone();
            Arc::new(move || mount.active_modules())
        },
        builtpanel_events::Settings::default(),
    );

    // One signal stops the API and the agent port.
    let (stop, stopped) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = stop.send(true);
    });
    // The dispatcher lives next to the agent port: it needs the live sessions.
    let dispatcher_task = gateway.as_ref().and_then(|(gateway, _)| {
        let results = gateway.take_results()?;
        let dispatcher = builtpanel_dispatcher::Dispatcher::new(
            operations.clone(),
            gateway.clone(),
            Some(redis.clone()),
            builtpanel_dispatcher::Settings::default(),
        )
        .with_secrets(operation_secrets);
        Some(tokio::spawn(dispatcher.run(results, stopped.clone())))
    });
    let relay_task = tokio::spawn(relay.run(stopped.clone()));
    let license_task = tokio::spawn(license.clone().run(stopped.clone()));
    // Zones kept in the panel are pushed to the DNS server, when there is one.
    let dns_task = match config.dns.powerdns_url.clone() {
        None => {
            tracing::info!("DNS sync is off: dns.powerdns_url is not set");
            None
        }
        Some(url) => match config.powerdns_api_key() {
            Ok(api_key) => {
                let provider =
                    builtpanel_dns::powerdns::PowerDns::new(&builtpanel_dns::powerdns::Config {
                        url,
                        api_key,
                        server_id: config.dns.powerdns_server_id.clone(),
                    });
                let sync = builtpanel_dns::sync::ZoneSync::new(
                    db.clone(),
                    Arc::new(provider),
                    Arc::new(SettingsNameservers {
                        registry: settings.clone(),
                        db: db.clone(),
                    }),
                );
                Some(tokio::spawn(sync.run(
                    std::time::Duration::from_secs(u64::from(config.dns.sync_seconds)),
                    stopped.clone(),
                )))
            }
            Err(err) => {
                tracing::error!(error = %err, "DNS API key unavailable");
                telemetry.shutdown();
                return ExitCode::from(EXIT_CONFIG);
            }
        },
    };
    let gateway_task =
        gateway.map(|(gateway, listener)| tokio::spawn(gateway.serve(listener, stopped.clone())));

    let addr = config.listen;
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(err) => {
            tracing::error!(%addr, error = %err, "failed to bind");
            return ExitCode::FAILURE;
        }
    };

    tracing::info!(%addr, version = builtpanel_common::VERSION, "{SERVICE} listening");
    // Connect info gives handlers the peer address (login events, sessions).
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown({
        let mut stopped = stopped.clone();
        async move {
            let _ = stopped.wait_for(|stop| *stop).await;
        }
    })
    .await;
    if let Some(task) = gateway_task {
        let _ = task.await;
    }
    if let Some(task) = dispatcher_task {
        let _ = task.await;
    }
    let _ = relay_task.await;
    let _ = license_task.await;
    if let Some(task) = dns_task {
        let _ = task.await;
    }

    db.close().await;
    telemetry.shutdown();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{SERVICE}: server error: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}

/// The name servers the administrator set in the domains module's settings.
struct SettingsNameservers {
    registry: Arc<builtpanel_settings::SettingsRegistry>,
    db: builtpanel_database::sqlx::PgPool,
}

impl builtpanel_dns::sync::NameserverSource for SettingsNameservers {
    fn nameservers(&self) -> builtpanel_dns::BoxFuture<'_, Vec<String>> {
        Box::pin(async move {
            // A settings read that fails leaves the zones as they are: no name
            // servers means "do not change the ones the server has".
            builtpanel_mod_domains::nameservers::configured(&self.registry, &self.db)
                .await
                .unwrap_or_default()
        })
    }
}
