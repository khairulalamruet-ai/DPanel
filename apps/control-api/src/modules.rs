//! The modules compiled into this binary, by their manifests (T03.01).
//!
//! Each manifest is embedded at build time and validated at startup. A broken
//! manifest stops the service with a message that lists every problem.
//! (The `Module` trait and the registry that will own this list are T03.02.)

use std::sync::Arc;

use builtpanel_settings::SettingsRegistry;

use builtpanel_kernel::{
    Manifest, ManifestError, ManifestOnly, Module, ModuleMount, ModuleRoutes, Registry,
};

/// Directory name under `modules/` and the manifest text found there.
const EMBEDDED: &[(&str, &str)] = &[
    ("auth", include_str!("../../../modules/auth/module.toml")),
    ("rbac", include_str!("../../../modules/rbac/module.toml")),
    ("audit", include_str!("../../../modules/audit/module.toml")),
    ("nodes", include_str!("../../../modules/nodes/module.toml")),
    ("system", builtpanel_mod_system::MANIFEST),
    ("plans", builtpanel_mod_plans::MANIFEST),
    ("users", builtpanel_mod_users::MANIFEST),
    ("resellers", builtpanel_mod_users::reseller::MANIFEST),
    ("domains", builtpanel_mod_domains::MANIFEST),
    ("databases", builtpanel_mod_databases::MANIFEST),
    ("files", builtpanel_mod_files::MANIFEST),
    ("backups", builtpanel_mod_backups::MANIFEST),
    ("cron", builtpanel_mod_cron::MANIFEST),
    ("mail", builtpanel_mod_mail::MANIFEST),
    ("migration", builtpanel_mod_migration::MANIFEST),
    ("logs", builtpanel_mod_logs::MANIFEST),
    ("usage", builtpanel_mod_usage::MANIFEST),
    ("services", builtpanel_mod_services::MANIFEST),
    ("security", builtpanel_mod_security::MANIFEST),
    ("apps", builtpanel_mod_apps::MANIFEST),
    ("license", builtpanel_mod_license::MANIFEST),
    ("notifications", builtpanel_mod_notifications::MANIFEST),
];

/// Days the data of a deleted user is kept before it is purged.
// TODO(T04.12): make this a panel setting once the system module's schema has a
// place for hosting policy.
const PURGE_AFTER_DAYS: i32 = 7;

/// A module of this binary: its manifest and how to make its routes. The
/// routes are made each time the set of mounted modules changes.
struct Wired {
    manifest: Manifest,
    routes: Box<dyn Fn() -> ModuleRoutes + Send + Sync>,
    /// The notifications module turns events into notifications.
    notifications: Option<builtpanel_mod_notifications::NotificationsState>,
}

impl Module for Wired {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn routes(&self) -> ModuleRoutes {
        (self.routes)()
    }

    fn on_event<'a>(
        &'a self,
        event: &'a builtpanel_kernel::Event,
    ) -> builtpanel_kernel::module::BoxFuture<'a, Result<(), builtpanel_kernel::module::EventError>>
    {
        match &self.notifications {
            Some(state) => Box::pin(builtpanel_mod_notifications::handle_event(state, event)),
            None => Box::pin(async { Ok(()) }),
        }
    }

    fn settings_schema(&self) -> Option<serde_json::Value> {
        match self.manifest.module.name.as_str() {
            "system" => serde_json::from_str(builtpanel_mod_system::SETTINGS_SCHEMA).ok(),
            "domains" => serde_json::from_str(builtpanel_mod_domains::SETTINGS_SCHEMA).ok(),
            "databases" => serde_json::from_str(builtpanel_mod_databases::SETTINGS_SCHEMA).ok(),
            "files" => serde_json::from_str(builtpanel_mod_files::SETTINGS_SCHEMA).ok(),
            "cron" => serde_json::from_str(builtpanel_mod_cron::SETTINGS_SCHEMA).ok(),
            "mail" => serde_json::from_str(builtpanel_mod_mail::SETTINGS_SCHEMA).ok(),
            _ => None,
        }
    }
}

/// Checks the settings schemas of all modules (T03.08).
///
/// # Errors
/// The messages to show, one per unusable schema.
pub fn settings(registry: &Registry) -> Result<SettingsRegistry, Vec<String>> {
    let mut schemas = Vec::new();
    let mut problems = Vec::new();
    for module in registry.modules() {
        let name = module.manifest().module.name.clone();
        if matches!(name.as_str(), "system" | "domains" | "databases" | "files")
            && module.settings_schema().is_none()
        {
            problems.push(format!(
                "settings schema of module `{name}`: not valid JSON"
            ));
        }
        if let Some(schema) = module.settings_schema() {
            schemas.push((name, schema));
        }
    }
    match SettingsRegistry::build(schemas) {
        Ok(settings) if problems.is_empty() => Ok(settings),
        Ok(_) => Err(problems),
        Err(errors) => {
            problems.extend(errors.iter().map(ToString::to_string));
            Err(problems)
        }
    }
}

/// The state each module's routes need.
pub struct States {
    /// Sign-in and sessions (also carries what the audit viewer needs).
    pub auth: builtpanel_mod_auth::AuthState,
    /// Administrators and roles.
    pub admin: builtpanel_mod_admin::AdminState,
    /// Nodes and operations.
    pub nodes: builtpanel_mod_nodes::NodesState,
    /// Where uploaded SQL dumps wait for the node.
    pub spool_dir: std::path::PathBuf,
    /// This installation's license.
    pub license: Arc<builtpanel_mod_license::service::LicenseService>,
}

/// Builds the registry of this binary with every module's routes attached,
/// and the live mount that serves them. `base` holds the routes outside any
/// module and the fallback for unknown paths.
///
/// # Errors
/// The messages to show when a manifest or the module set is broken.
pub fn mount(
    states: States,
    base: axum::Router,
) -> Result<(ModuleMount, Arc<SettingsRegistry>), Vec<String>> {
    let manifests = load_from(EMBEDDED)
        .map_err(|errors| errors.iter().map(ToString::to_string).collect::<Vec<_>>())?;
    // The system module's routes need the mount itself (to switch modules), which
    // exists only after the registry does.
    let late_mount: Arc<std::sync::OnceLock<ModuleMount>> = Arc::default();
    // Asks the public DNS about domains' name servers (set up on first use).
    let resolver: Arc<dyn builtpanel_dns::resolver::NsResolver> =
        Arc::new(builtpanel_dns::resolver::SystemResolver::new());
    // Likewise the settings schemas come from the modules once they exist.
    let late_settings: Arc<std::sync::OnceLock<Arc<SettingsRegistry>>> = Arc::default();
    // One cache for every rebuild of the system routes, so it survives a module switch.
    let branding: Arc<builtpanel_mod_system::bootstrap::BrandingCache> = Arc::default();
    let (db, sessions) = (states.auth.db.clone(), states.auth.sessions.clone());
    let master_key = states.auth.master_key.clone();
    // One counter for the whole panel, so the limit holds across instances.
    let ask_limiter = builtpanel_cache::ratelimit::RateLimiter::new(
        states.auth.redis.clone(),
        "tls-ask",
        builtpanel_mod_domains::tls::ASK_LIMIT_PER_MINUTE,
        std::time::Duration::from_secs(60),
    );
    let restart_limiter = builtpanel_cache::ratelimit::RateLimiter::new(
        states.auth.redis.clone(),
        "service-restart",
        builtpanel_mod_domains::services::RESTARTS_PER_MINUTE,
        std::time::Duration::from_secs(60),
    );
    let States {
        auth,
        admin,
        nodes,
        spool_dir,
        license,
    } = states;
    let agent_url: Option<String> = nodes.public_url.as_deref().map(str::to_owned);
    let notifications_state = builtpanel_mod_notifications::NotificationsState {
        db: db.clone(),
        sessions: sessions.clone(),
        settings: late_settings.clone(),
        master_key: master_key.clone(),
    };
    let modules = manifests
        .into_iter()
        .map(|manifest| {
            let routes: Box<dyn Fn() -> ModuleRoutes + Send + Sync> = match manifest
                .module
                .name
                .as_str()
            {
                "auth" => {
                    let auth = auth.clone();
                    Box::new(move || ModuleRoutes {
                        public: Some(builtpanel_mod_auth::router().with_state(auth.clone())),
                        ..ModuleRoutes::none()
                    })
                }
                "rbac" => {
                    let admin = admin.clone();
                    Box::new(move || ModuleRoutes {
                        admin: Some(builtpanel_mod_admin::router().with_state(admin.clone())),
                        ..ModuleRoutes::none()
                    })
                }
                "audit" => {
                    let auth = auth.clone();
                    Box::new(move || ModuleRoutes {
                        admin: Some(
                            builtpanel_audit::router::<builtpanel_mod_auth::AuthState>()
                                .with_state(auth.clone()),
                        ),
                        ..ModuleRoutes::none()
                    })
                }
                "nodes" => {
                    let nodes = nodes.clone();
                    Box::new(move || ModuleRoutes {
                        admin: Some(builtpanel_mod_nodes::router().with_state(nodes.clone())),
                        ..ModuleRoutes::none()
                    })
                }
                "system" => {
                    let (db, sessions, late_mount, late_settings) = (
                        db.clone(),
                        sessions.clone(),
                        late_mount.clone(),
                        late_settings.clone(),
                    );
                    let branding = branding.clone();
                    Box::new(move || match (late_mount.get(), late_settings.get()) {
                        (Some(mount), Some(settings)) => {
                            let state = builtpanel_mod_system::SystemState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                mount: mount.clone(),
                                settings: settings.clone(),
                                branding: branding.clone(),
                            };
                            ModuleRoutes {
                                admin: Some(
                                    builtpanel_mod_system::router().with_state(state.clone()),
                                ),
                                client: Some(
                                    builtpanel_mod_system::client_router()
                                        .with_state(state.clone()),
                                ),
                                public: Some(
                                    builtpanel_mod_system::public_router().with_state(state),
                                ),
                            }
                        }
                        _ => ModuleRoutes::none(),
                    })
                }
                "plans" => {
                    let (db, sessions, late_mount) =
                        (db.clone(), sessions.clone(), late_mount.clone());
                    Box::new(move || match late_mount.get() {
                        Some(mount) => ModuleRoutes {
                            admin: Some(builtpanel_mod_plans::router().with_state(
                                builtpanel_mod_plans::PlansState {
                                    db: db.clone(),
                                    sessions: sessions.clone(),
                                    mount: mount.clone(),
                                },
                            )),
                            ..ModuleRoutes::none()
                        },
                        None => ModuleRoutes::none(),
                    })
                }
                "resellers" => {
                    let (db, sessions, hasher) =
                        (db.clone(), sessions.clone(), admin.hasher.clone());
                    Box::new(move || ModuleRoutes {
                        admin: Some(builtpanel_mod_users::reseller::admin_router().with_state(
                            builtpanel_mod_users::UsersState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                hasher: hasher.clone(),
                                purge_after_days: PURGE_AFTER_DAYS,
                            },
                        )),
                        client: Some(builtpanel_mod_users::reseller::portal_router().with_state(
                            builtpanel_mod_users::UsersState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                hasher: hasher.clone(),
                                purge_after_days: PURGE_AFTER_DAYS,
                            },
                        )),
                        ..ModuleRoutes::none()
                    })
                }
                "users" => {
                    let (db, sessions, hasher) =
                        (db.clone(), sessions.clone(), admin.hasher.clone());
                    Box::new(move || ModuleRoutes {
                        admin: Some(builtpanel_mod_users::router().with_state(
                            builtpanel_mod_users::UsersState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                hasher: hasher.clone(),
                                purge_after_days: PURGE_AFTER_DAYS,
                            },
                        )),
                        ..ModuleRoutes::none()
                    })
                }
                "domains" => {
                    let (
                        db,
                        sessions,
                        late_settings,
                        resolver,
                        ask_limiter,
                        restart_limiter,
                        master_key,
                    ) = (
                        db.clone(),
                        sessions.clone(),
                        late_settings.clone(),
                        resolver.clone(),
                        ask_limiter.clone(),
                        restart_limiter.clone(),
                        master_key.clone(),
                    );
                    Box::new(move || match late_settings.get() {
                        Some(settings) => {
                            let state = builtpanel_mod_domains::DomainsState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                settings: settings.clone(),
                                resolver: resolver.clone(),
                                ask_limiter: ask_limiter.clone(),
                                restart_limiter: restart_limiter.clone(),
                                master_key: master_key.clone(),
                            };
                            ModuleRoutes {
                                admin: Some(
                                    builtpanel_mod_domains::admin_router().with_state(state.clone()),
                                ),
                                client: Some(
                                    builtpanel_mod_domains::client_router().with_state(state.clone()),
                                ),
                                public: Some(
                                    builtpanel_mod_domains::tls::public_router().with_state(state),
                                ),
                            }
                        }
                        None => ModuleRoutes::none(),
                    })
                }
                "databases" => {
                    let (db, sessions, late_settings, master_key) = (
                        db.clone(),
                        sessions.clone(),
                        late_settings.clone(),
                        master_key.clone(),
                    );
                    let (spool_dir, agent_url) = (spool_dir.clone(), agent_url.clone());
                    Box::new(move || match late_settings.get() {
                        Some(settings) => {
                            let state = builtpanel_mod_databases::DatabasesState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                settings: settings.clone(),
                                master_key: master_key.clone(),
                                spool_dir: spool_dir.clone(),
                                agent_url: agent_url.clone(),
                            };
                            ModuleRoutes {
                                admin: Some(
                                    builtpanel_mod_databases::admin_router().with_state(state.clone()),
                                ),
                                client: Some(
                                    builtpanel_mod_databases::client_router()
                                        .with_state(state.clone()),
                                ),
                                public: Some(
                                    builtpanel_mod_databases::public_router().with_state(state),
                                ),
                            }
                        }
                        None => ModuleRoutes::none(),
                    })
                }
                "files" => {
                    let (db, sessions, late_settings, master_key) = (
                        db.clone(),
                        sessions.clone(),
                        late_settings.clone(),
                        master_key.clone(),
                    );
                    let (spool_dir, agent_url) = (spool_dir.clone(), agent_url.clone());
                    Box::new(move || match late_settings.get() {
                        Some(settings) => {
                            let state = builtpanel_mod_files::FilesState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                settings: settings.clone(),
                                wait: builtpanel_mod_files::DEFAULT_WAIT,
                                master_key: master_key.clone(),
                                spool_dir: spool_dir.clone(),
                                agent_url: agent_url.clone(),
                            };
                            ModuleRoutes {
                                admin: Some(
                                    builtpanel_mod_files::admin_router().with_state(state.clone()),
                                ),
                                client: Some(
                                    builtpanel_mod_files::client_router().with_state(state.clone()),
                                ),
                                public: Some(builtpanel_mod_files::public_router().with_state(state)),
                            }
                        }
                        None => ModuleRoutes::none(),
                    })
                }
                "backups" => {
                    let (db, sessions, late_settings, master_key) = (
                        db.clone(),
                        sessions.clone(),
                        late_settings.clone(),
                        master_key.clone(),
                    );
                    Box::new(move || match late_settings.get() {
                        Some(settings) => {
                            let state = builtpanel_mod_backups::BackupsState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                settings: settings.clone(),
                                master_key: master_key.clone(),
                            };
                            ModuleRoutes {
                                admin: Some(
                                    builtpanel_mod_backups::admin_router().with_state(state.clone()),
                                ),
                                client: Some(
                                    builtpanel_mod_backups::client_router().with_state(state),
                                ),
                                ..ModuleRoutes::none()
                            }
                        }
                        None => ModuleRoutes::none(),
                    })
                }
                "migration" => {
                    let (db, sessions, spool_dir, loopback) = (
                        db.clone(),
                        sessions.clone(),
                        spool_dir.clone(),
                        builtpanel_mod_migration::exec::Loopback(late_mount.clone()),
                    );
                    Box::new(move || ModuleRoutes {
                        admin: Some(builtpanel_mod_migration::admin_router().with_state(
                            builtpanel_mod_migration::MigrationState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                spool_dir: spool_dir.clone(),
                                loopback: loopback.clone(),
                                max_backup_bytes: 100 * 1024 * 1024 * 1024,
                                allow_local_sources: false,
                            },
                        )),
                        ..ModuleRoutes::none()
                    })
                }
                "cron" => {
                    let (db, sessions, late_settings) =
                        (db.clone(), sessions.clone(), late_settings.clone());
                    Box::new(move || match late_settings.get() {
                        Some(settings) => ModuleRoutes {
                            client: Some(builtpanel_mod_cron::client_router().with_state(
                                builtpanel_mod_cron::CronState {
                                    db: db.clone(),
                                    sessions: sessions.clone(),
                                    settings: settings.clone(),
                                },
                            )),
                            ..ModuleRoutes::none()
                        },
                        None => ModuleRoutes::none(),
                    })
                }
                "mail" => {
                    let (db, sessions, late_settings, master_key) = (
                        db.clone(),
                        sessions.clone(),
                        late_settings.clone(),
                        master_key.clone(),
                    );
                    Box::new(move || match late_settings.get() {
                        Some(settings) => {
                            let state = builtpanel_mod_mail::MailState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                settings: settings.clone(),
                                master_key: master_key.clone(),
                            };
                            ModuleRoutes {
                                admin: Some(
                                    builtpanel_mod_mail::admin_router().with_state(state.clone()),
                                ),
                                client: Some(builtpanel_mod_mail::client_router().with_state(state)),
                                ..ModuleRoutes::none()
                            }
                        }
                        None => ModuleRoutes::none(),
                    })
                }
                "logs" => {
                    let (db, sessions) = (db.clone(), sessions.clone());
                    Box::new(move || ModuleRoutes {
                        client: Some(builtpanel_mod_logs::client_router().with_state(
                            builtpanel_mod_logs::LogsState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                wait: builtpanel_mod_logs::DEFAULT_WAIT,
                            },
                        )),
                        ..ModuleRoutes::none()
                    })
                }
                "notifications" => {
                    let state = notifications_state.clone();
                    Box::new(move || ModuleRoutes {
                        admin: Some(
                            builtpanel_mod_notifications::admin_router().with_state(state.clone()),
                        ),
                        client: Some(
                            builtpanel_mod_notifications::client_router().with_state(state.clone()),
                        ),
                        ..ModuleRoutes::none()
                    })
                }
                "usage" => {
                    let (db, sessions) = (db.clone(), sessions.clone());
                    Box::new(move || {
                        let state = builtpanel_mod_usage::UsageState {
                            db: db.clone(),
                            sessions: sessions.clone(),
                        };
                        ModuleRoutes {
                            admin: Some(
                                builtpanel_mod_usage::admin_router().with_state(state.clone()),
                            ),
                            client: Some(builtpanel_mod_usage::client_router().with_state(state)),
                            ..ModuleRoutes::none()
                        }
                    })
                }
                "services" => {
                    let (db, sessions) = (db.clone(), sessions.clone());
                    Box::new(move || {
                        let state = builtpanel_mod_services::ServicesState {
                            db: db.clone(),
                            sessions: sessions.clone(),
                            wait: builtpanel_mod_services::DEFAULT_WAIT,
                        };
                        ModuleRoutes {
                            admin: Some(
                                builtpanel_mod_services::admin_router().with_state(state.clone()),
                            ),
                            client: Some(builtpanel_mod_services::client_router().with_state(state)),
                            ..ModuleRoutes::none()
                        }
                    })
                }
                "security" => {
                    let (db, sessions) = (db.clone(), sessions.clone());
                    Box::new(move || {
                        let state = builtpanel_mod_security::SecurityState {
                            db: db.clone(),
                            sessions: sessions.clone(),
                        };
                        ModuleRoutes {
                            admin: Some(
                                builtpanel_mod_security::admin_router().with_state(state.clone()),
                            ),
                            client: Some(builtpanel_mod_security::client_router().with_state(state)),
                            ..ModuleRoutes::none()
                        }
                    })
                }
                "license" => {
                    let (db, sessions, service) = (db.clone(), sessions.clone(), license.clone());
                    Box::new(move || ModuleRoutes {
                        admin: Some(builtpanel_mod_license::admin_router().with_state(
                            builtpanel_mod_license::LicenseState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                service: service.clone(),
                            },
                        )),
                        public: Some(builtpanel_mod_license::public_router().with_state(
                            builtpanel_mod_license::LicenseState {
                                db: db.clone(),
                                sessions: sessions.clone(),
                                service: service.clone(),
                            },
                        )),
                        ..ModuleRoutes::none()
                    })
                }
                "apps" => {
                    let (db, sessions, master_key) =
                        (db.clone(), sessions.clone(), master_key.clone());
                    // Built-in applications, and any the operator put in a directory.
                    let catalog = Arc::new(
                        match std::env::var_os("BUILTPANEL_CATALOG_DIR") {
                            Some(dir) => builtpanel_catalog::Catalog::with_dir(std::path::Path::new(&dir)),
                            None => builtpanel_catalog::Catalog::embedded(),
                        }
                        .unwrap_or_else(|e| {
                            tracing::error!(error = %e, "the application catalog is not valid; using none");
                            builtpanel_catalog::Catalog::default()
                        }),
                    );
                    Box::new(move || {
                        let state = builtpanel_mod_apps::AppsState {
                            db: db.clone(),
                            sessions: sessions.clone(),
                            master_key: master_key.clone(),
                            wait: builtpanel_mod_apps::DEFAULT_WAIT,
                            catalog: catalog.clone(),
                        };
                        ModuleRoutes {
                            admin: Some(builtpanel_mod_apps::admin_router().with_state(state.clone())),
                            client: Some(builtpanel_mod_apps::client_router().with_state(state.clone())),
                            public: Some(builtpanel_mod_apps::public_router().with_state(state)),
                        }
                    })
                }
                // A manifest without code behind it has no routes.
                _ => Box::new(ModuleRoutes::none),
            };
            let notifications =
                (manifest.module.name == "notifications").then(|| notifications_state.clone());
            Arc::new(Wired {
                manifest,
                routes,
                notifications,
            }) as Arc<dyn Module>
        })
        .collect();
    let registry = Registry::build(modules).map_err(|err| vec![err.to_string()])?;
    // The settings schemas of the modules must be usable before anything is served.
    let settings = Arc::new(settings(&registry)?);
    let mount = ModuleMount::new(registry, base);
    let _ = late_mount.set(mount.clone());
    let _ = late_settings.set(settings.clone());
    mount.rebuild();
    Ok((mount, settings))
}

/// Keeps the served routes in step with the modules table: once as soon as the
/// database answers, then every `every` (which also picks up a change made by
/// another control plane instance).
pub async fn follow_enabled(
    db: builtpanel_database::sqlx::PgPool,
    mount: ModuleMount,
    every: std::time::Duration,
) {
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match builtpanel_database::modules::enabled_names(&db).await {
            Ok(names) => {
                if mount.apply(&names.into_iter().collect()) {
                    tracing::info!(active = ?mount.mounted(), "module routes follow the modules table");
                }
            }
            Err(err) => tracing::debug!(error = %err, "could not read the modules table"),
        }
    }
}

/// Parses the embedded manifests.
///
/// # Errors
/// One [`ManifestError`] per broken manifest.
#[cfg(test)]
pub fn load() -> Result<Vec<Manifest>, Vec<ManifestError>> {
    load_from(EMBEDDED)
}

/// Parses `(directory, text)` pairs. Besides each manifest's own rules, the
/// module name must equal its directory. How the modules fit together is
/// checked by [`registry`].
pub fn load_from(sources: &[(&str, &str)]) -> Result<Vec<Manifest>, Vec<ManifestError>> {
    let mut manifests = Vec::new();
    let mut errors = Vec::new();
    for (dir, text) in sources {
        let origin = format!("modules/{dir}/module.toml");
        match Manifest::parse(&origin, text) {
            Ok(manifest) if manifest.module.name != *dir => errors.push(ManifestError {
                origin,
                problems: vec![format!(
                    "module.name: `{}` must match its directory `{dir}`",
                    manifest.module.name
                )],
            }),
            Ok(manifest) => manifests.push(manifest),
            Err(err) => errors.push(err),
        }
    }
    if errors.is_empty() {
        Ok(manifests)
    } else {
        Err(errors)
    }
}

/// Builds the module registry of this binary: every manifest valid, every
/// dependency present, no cycles, no key defined twice.
///
/// # Errors
/// The messages to show, one per broken manifest or one for the module set.
pub fn registry() -> Result<Registry, Vec<String>> {
    registry_from(EMBEDDED)
}

/// [`registry`] for the given manifests.
pub fn registry_from(sources: &[(&str, &str)]) -> Result<Registry, Vec<String>> {
    let manifests = load_from(sources)
        .map_err(|errors| errors.iter().map(ToString::to_string).collect::<Vec<_>>())?;
    // Until the modules' code is behind the `Module` trait (T03.04) they are
    // represented by their manifests.
    let modules = manifests
        .into_iter()
        .map(|m| Arc::new(ManifestOnly(m)) as Arc<dyn Module>)
        .collect();
    Registry::build(modules).map_err(|err| vec![err.to_string()])
}

/// Writes the modules of this build into the `modules` table (T03.03): new
/// ones appear switched on, known ones are refreshed, and modules that are no
/// longer part of the build are marked missing.
///
/// # Errors
/// Database errors (the caller retries; the API works without the sync).
pub async fn sync(
    db: &builtpanel_database::sqlx::PgPool,
    registry: &Registry,
) -> Result<builtpanel_database::modules::SyncReport, builtpanel_database::sqlx::Error> {
    let manifests: Vec<(&Manifest, serde_json::Value)> = registry
        .modules()
        .iter()
        .map(|m| {
            let manifest = m.manifest();
            (manifest, serde_json::to_value(manifest).unwrap_or_default())
        })
        .collect();
    let records: Vec<builtpanel_database::modules::ModuleRecord<'_>> = manifests
        .iter()
        .map(
            |(manifest, json)| builtpanel_database::modules::ModuleRecord {
                name: &manifest.module.name,
                version: &manifest.module.version,
                category: manifest.module.category.as_str(),
                core: manifest.module.core,
                manifest: json,
            },
        )
        .collect();
    builtpanel_database::modules::sync(db, &records).await
}

/// Writes the permission keys the manifests declare into the catalogue (T03.06).
///
/// # Errors
/// Database errors.
pub async fn sync_permissions(
    db: &builtpanel_database::sqlx::PgPool,
    registry: &Registry,
) -> Result<builtpanel_database::permissions::SyncReport, builtpanel_database::sqlx::Error> {
    let keys: Vec<(&str, &str)> = registry
        .modules()
        .iter()
        .flat_map(|m| {
            let manifest = m.manifest();
            manifest
                .permissions
                .keys
                .iter()
                .map(|key| (key.as_str(), manifest.module.name.as_str()))
        })
        .collect();
    builtpanel_database::permissions::sync(db, &keys).await
}

/// Runs [`sync`] until it succeeds once (the database may come up after the
/// API does), logging what changed.
pub async fn sync_until_done(db: builtpanel_database::sqlx::PgPool, registry: Registry) {
    let mut wait = std::time::Duration::from_secs(1);
    loop {
        match sync_permissions(&db, &registry).await {
            Ok(report) if !report.is_empty() => tracing::info!(
                added = ?report.added,
                restored = ?report.restored,
                moved = ?report.moved,
                missing = ?report.missing,
                "permission catalogue synced with this build"
            ),
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(error = %err, retry_in = ?wait, "could not sync the permission catalogue");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(std::time::Duration::from_secs(30));
                continue;
            }
        }
        match sync(&db, &registry).await {
            Ok(report) => {
                if report.is_empty() {
                    tracing::debug!("modules table already matches this build");
                } else {
                    tracing::info!(
                        added = ?report.added,
                        restored = ?report.restored,
                        updated = ?report.updated,
                        missing = ?report.missing,
                        "modules table synced with this build"
                    );
                }
                return;
            }
            Err(err) => {
                tracing::warn!(error = %err, retry_in = ?wait, "could not sync the modules table");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(std::time::Duration::from_secs(30));
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifests_are_valid() {
        let manifests = load().unwrap_or_else(|errors| {
            panic!(
                "{}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
        let names: Vec<_> = manifests.iter().map(|m| m.module.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "auth",
                "rbac",
                "audit",
                "nodes",
                "system",
                "plans",
                "users",
                "resellers",
                "domains",
                "databases",
                "files",
                "backups",
                "cron",
                "mail",
                "migration",
                "logs",
                "usage",
                "services",
                "security",
                "apps",
                "license",
                "notifications"
            ]
        );
        // And they fit together: dependencies first.
        let registry = registry().unwrap_or_else(|e| panic!("{}", e.join("\n")));
        assert_eq!(
            registry.names().collect::<Vec<_>>(),
            [
                "auth",
                "rbac",
                "audit",
                "nodes",
                "system",
                "license",
                "plans",
                "users",
                "backups",
                "cron",
                "databases",
                "domains",
                "files",
                "logs",
                "migration",
                "notifications",
                "resellers",
                "services",
                "usage",
                "apps",
                "mail",
                "security"
            ]
        );
        assert_eq!(
            registry.dependents_of("rbac"),
            [
                "audit",
                "nodes",
                "system",
                "license",
                "plans",
                "users",
                "backups",
                "cron",
                "databases",
                "domains",
                "files",
                "logs",
                "migration",
                "notifications",
                "resellers",
                "services",
                "usage",
                "apps",
                "mail",
                "security"
            ]
        );
        // Every dependency names a module that exists.
        for m in &manifests {
            for dep in &m.module.depends_on {
                assert!(
                    names.contains(&dep.as_str()),
                    "{} depends on unknown {dep}",
                    m.module.name
                );
            }
        }
    }

    #[test]
    fn every_permission_the_code_checks_is_declared_by_a_manifest() {
        use builtpanel_rbac::{PermissionKey as _, permissions as p};
        let declared: Vec<String> = load()
            .unwrap()
            .into_iter()
            .flat_map(|m| m.permissions.keys)
            .collect();
        for key in [
            p::AdministratorsRead::KEY,
            p::AdministratorsManage::KEY,
            p::RolesRead::KEY,
            p::RolesManage::KEY,
            p::AuditRead::KEY,
            p::NodesRead::KEY,
            p::NodesManage::KEY,
            p::ModulesRead::KEY,
            p::ModulesManage::KEY,
            p::SettingsRead::KEY,
            p::SettingsManage::KEY,
            p::PlansRead::KEY,
            p::PlansManage::KEY,
            p::UsersRead::KEY,
            p::UsersManage::KEY,
            p::UsersImpersonate::KEY,
            p::ResellersRead::KEY,
            p::ResellersManage::KEY,
            p::MigrationRead::KEY,
            p::MigrationManage::KEY,
            p::DomainsRead::KEY,
            p::DomainsManage::KEY,
            p::DatabasesRead::KEY,
            p::DatabasesManage::KEY,
            p::BackupsRead::KEY,
            p::BackupsManage::KEY,
            p::LogsRead::KEY,
            p::ServerRead::KEY,
            p::ServerManage::KEY,
            p::SecurityRead::KEY,
            p::SecurityManage::KEY,
            p::AppsRead::KEY,
            p::AppsManage::KEY,
            p::LicenseRead::KEY,
            p::LicenseManage::KEY,
            p::EmailRead::KEY,
            p::EmailManage::KEY,
        ] {
            assert!(declared.iter().any(|d| d == key), "{key} is in no manifest");
        }
        assert_eq!(
            declared.len(),
            37,
            "a manifest declares a key the code does not know: {declared:?}"
        );
    }

    #[test]
    fn a_broken_manifest_gives_a_clear_error() {
        let good = "[module]\nname = \"alpha\"\nversion = \"1.0.0\"\ntitle = { en = \"Alpha\" }\ncategory = \"web\"\n[permissions]\nkeys = [\"alpha.read\"]\n";
        let errors = load_from(&[
            ("alpha", good),
            ("beta", "[module]\nname = \"beta\"\nversion = \"one\"\ntitle = { en = \"Beta\" }\ncategory = \"web\"\ndepend_on = []\n"),
        ])
        .unwrap_err();
        assert_eq!(errors.len(), 1);
        let text = errors[0].to_string();
        assert!(
            text.starts_with("invalid manifest for modules/beta/module.toml:"),
            "{text}"
        );
        assert!(text.contains("depend_on"), "the typo is named: {text}");

        // The name must match the directory.
        let text = load_from(&[("gamma", good)]).unwrap_err()[0].to_string();
        assert!(
            text.contains("`alpha` must match its directory `gamma`"),
            "{text}"
        );

        // Two modules cannot define the same permission key, and a missing
        // dependency or a cycle stops the build of the registry.
        let thief = good.replace("name = \"alpha\"", "name = \"delta\"");
        let text = registry_from(&[("alpha", good), ("delta", &thief)])
            .unwrap_err()
            .join("\n");
        assert!(
            text.contains("permission `alpha.read` is defined by both `alpha` and `delta`"),
            "{text}"
        );
        let needy = "[module]\nname = \"needy\"\nversion = \"1.0.0\"\ntitle = { en = \"N\" }\ncategory = \"web\"\ndepends_on = [\"ghost\"]\n";
        let text = registry_from(&[("needy", needy)]).unwrap_err().join("\n");
        assert!(
            text.contains("module `needy` depends on `ghost`, which is not part of this build"),
            "{text}"
        );
    }
}
