//! The system module: the panel's own machinery.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/admin/bootstrap` | a signed-in administrator |
//! | GET | `/api/v1/client/bootstrap` | a signed-in customer |
//! | GET | `/api/v1/public/branding` | anyone: the look of the sign-in page |
//! | GET | `/api/v1/search` | a signed-in account: domains and accounts by a few letters |
//! | GET | `/api/v1/admin/modules` | `admin.modules.read` |
//! | PUT | `/api/v1/admin/modules/{name}` | `admin.modules.manage`, sudo mode |
//! | GET | `/api/v1/admin/settings` | `admin.settings.read` |
//! | GET | `/api/v1/admin/settings/{module}` | `admin.settings.read` |
//! | PUT | `/api/v1/admin/settings/{module}` | `admin.settings.manage`, sudo mode |
//!
//! Switching a module changes the database and, right after the commit, the
//! routes the server answers ([`builtpanel_kernel::ModuleMount`]): no restart.
//!
//! Rules, enforced here and again by the table and the mount:
//! * a core module cannot be switched off;
//! * a module cannot be switched off while modules that need it are on, and
//!   cannot be switched on while modules it needs are off — unless the request
//!   says `cascade`, which switches those too. Without `cascade` the answer is
//!   409 and names them, so the interface can ask first;
//! * every module that changes is written to the audit log in the same
//!   transaction.

pub mod bootstrap;
pub mod search;

use std::{collections::BTreeSet, sync::Arc};

use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    routing::{get, put},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{modules as db, sqlx::PgPool};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_kernel::{Event, ModuleMount};
use builtpanel_rbac::{
    Require, RequireRecent,
    permissions::{LogsRead, ModulesManage, ModulesRead, SettingsManage, SettingsRead},
};
use builtpanel_session::SessionStore;
use builtpanel_settings::{ModuleSettings, Scope, SettingsError, SettingsRegistry};
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};

/// The settings of the system module, as a JSON Schema (see `Module::settings_schema`).
pub const SETTINGS_SCHEMA: &str = include_str!("../../../../modules/system/settings.schema.json");

/// The system module's manifest text.
pub const MANIFEST: &str = include_str!("../../../../modules/system/module.toml");

/// Dependencies of the system handlers.
#[derive(Clone)]
pub struct SystemState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The live module routes; also knows the modules of this build.
    pub mount: ModuleMount,
    /// The checked settings schemas of all modules.
    pub settings: Arc<SettingsRegistry>,
    /// The panel's look, kept in memory for a few seconds.
    pub branding: Arc<bootstrap::BrandingCache>,
}

impl FromRef<SystemState> for PgPool {
    fn from_ref(s: &SystemState) -> Self {
        s.db.clone()
    }
}

impl FromRef<SystemState> for SessionStore {
    fn from_ref(s: &SystemState) -> Self {
        s.sessions.clone()
    }
}

/// Routes with full paths; merge into the application router.
pub fn router() -> Router<SystemState> {
    Router::new()
        .route("/api/v1/admin/bootstrap", get(bootstrap::admin_bootstrap))
        .route("/api/v1/admin/modules", get(list_modules))
        .route("/api/v1/admin/modules/{name}", put(set_module))
        .route("/api/v1/admin/system-logs", get(system_logs))
        .route("/api/v1/admin/settings", get(list_settings))
        .route(
            "/api/v1/admin/settings/{module}",
            get(get_settings).put(put_settings),
        )
}

/// The customers' routes: their bootstrap.
pub fn client_router() -> Router<SystemState> {
    Router::new()
        .route("/api/v1/client/bootstrap", get(bootstrap::client_bootstrap))
        .route("/api/v1/search", get(search::search))
}

/// Routes that need no session: the look of the sign-in page.
pub fn public_router() -> Router<SystemState> {
    Router::new().route("/api/v1/public/branding", get(bootstrap::public_branding))
}

/// Which events of the control plane's log.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SystemLogQuery {
    /// The worst level wanted: `error`, `warn`, `info` (default) or `debug`.
    pub level: Option<String>,
    /// Only events after this time (Unix milliseconds).
    #[serde(default)]
    pub since_ms: i64,
    /// Only events whose text or module contains this (case-insensitive).
    pub search: Option<String>,
    /// The newest this many (1-1000, default 200).
    pub lines: Option<u32>,
}

/// One event of the control plane's log.
#[derive(Debug, Serialize, ToSchema)]
pub struct SystemLogLine {
    /// When (Unix milliseconds).
    pub at_ms: i64,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub level: String,
    /// The module that wrote it.
    pub target: String,
    /// The message and its fields.
    pub text: String,
}

/// The events.
#[derive(Debug, Serialize, ToSchema)]
pub struct SystemLogList {
    /// Oldest first.
    pub items: Vec<SystemLogLine>,
}

/// The control plane's own log: the newest events this process wrote (it keeps 2000; the
/// full log is in the service's journal).
#[utoipa::path(
    get,
    path = "/api/v1/admin/system-logs",
    tag = "system",
    operation_id = "adminSystemLogs",
    security(("session" = [])),
    params(SystemLogQuery),
    responses(
        (status = 200, description = "The events", body = SystemLogList),
        (status = 403, description = "`permission_denied` (needs admin.logs.read) or the caller is not an administrator", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A level or count that is not usable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn system_logs(
    _who: Require<LogsRead>,
    axum::extract::Query(q): axum::extract::Query<SystemLogQuery>,
) -> Result<Json<SystemLogList>, AppError> {
    let level = match q.level.as_deref().unwrap_or("info") {
        "error" => "ERROR",
        "warn" => "WARN",
        "info" => "INFO",
        "debug" => "DEBUG",
        _ => {
            return Err(AppError::validation(vec![FieldError::new(
                "level",
                "invalid",
                "error, warn, info or debug.",
            )]));
        }
    };
    let lines = q.lines.unwrap_or(200);
    if !(1..=1000).contains(&lines) || q.since_ms < 0 {
        return Err(AppError::validation(vec![FieldError::new(
            "lines",
            "out_of_range",
            "Between 1 and 1000 lines.",
        )]));
    }
    let items = builtpanel_observability::ring::recent(
        q.since_ms,
        level,
        q.search.as_deref(),
        usize::try_from(lines).unwrap_or(200),
    )
    .into_iter()
    .map(|e| SystemLogLine {
        at_ms: e.at_ms,
        level: e.level.to_owned(),
        target: e.target,
        text: e.text,
    })
    .collect();
    Ok(Json(SystemLogList { items }))
}

/// A module as shown to administrators.
#[derive(Debug, Serialize, ToSchema)]
pub struct ModuleView {
    /// Module name.
    #[schema(example = "domains")]
    pub name: String,
    /// Display name by language code.
    #[schema(value_type = Object)]
    pub title: serde_json::Value,
    /// Version of the build.
    pub version: String,
    /// `core`, `web`, `db`, `mail`, `files`, `security`, `system` or `apps`.
    pub category: String,
    /// Cannot be switched off.
    pub core: bool,
    /// Switched on by an administrator.
    pub enabled: bool,
    /// Part of the running build. A module that is not is listed for history
    /// and cannot be switched.
    pub present: bool,
    /// Its routes are being served right now (switched on, with everything it needs).
    pub active: bool,
    /// Modules it needs, directly.
    pub depends_on: Vec<String>,
    /// Modules that need it, directly or indirectly: switching it off stops these too.
    pub dependents: Vec<String>,
}

/// All modules.
#[derive(Debug, Serialize, ToSchema)]
pub struct ModuleList {
    /// By name.
    pub items: Vec<ModuleView>,
}

/// A change of a module's switch.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetModule {
    /// On or off.
    pub enabled: bool,
    /// Also switch what has to change with it: when switching off, the modules
    /// that need this one; when switching on, the modules this one needs.
    #[serde(default)]
    pub cascade: bool,
}

/// What a switch changed.
#[derive(Debug, Serialize, ToSchema)]
pub struct ModuleChange {
    /// Modules whose switch changed, including the one asked for (empty when
    /// it already was in the requested state).
    pub changed: Vec<String>,
    /// Modules whose routes are being served after the change.
    pub active: Vec<String>,
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

/// List the modules of this installation.
#[utoipa::path(
    get,
    path = "/api/v1/admin/modules",
    tag = "system",
    operation_id = "listModules",
    security(("session" = [])),
    responses(
        (status = 200, description = "All modules, by name", body = ModuleList),
        (status = 403, description = "`permission_denied` (needs admin.modules.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_modules(
    State(state): State<SystemState>,
    _who: Require<ModulesRead>,
) -> Result<Json<ModuleList>, AppError> {
    let rows = db::list(&state.db).await.map_err(internal)?;
    let active = state.mount.mounted();
    let registry = state.mount.registry();
    let items = rows
        .into_iter()
        .map(|row| {
            let manifest = registry.get(&row.name).map(|m| m.manifest());
            ModuleView {
                title: row
                    .manifest
                    .pointer("/module/title")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
                depends_on: manifest
                    .map(|m| m.module.depends_on.clone())
                    .unwrap_or_default(),
                dependents: registry
                    .dependents_of(&row.name)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                active: active.contains(&row.name),
                name: row.name,
                version: row.version,
                category: row.category,
                core: row.core,
                enabled: row.enabled,
                present: row.present,
            }
        })
        .collect();
    Ok(Json(ModuleList { items }))
}

/// Switch a module on or off. Takes effect at once, without a restart.
#[utoipa::path(
    put,
    path = "/api/v1/admin/modules/{name}",
    tag = "system",
    operation_id = "setModule",
    security(("session" = [])),
    params(
        ("name" = String, Path, description = "Module name"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = SetModule,
    responses(
        (status = 200, description = "Done (also when nothing had to change)", body = ModuleChange),
        (status = 403, description = "`permission_denied` (needs admin.modules.manage), `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such module in this build", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`module_is_core`; `module_has_dependents` (switching off: the detail names the modules that need it); `module_needs_dependencies` (switching on: the detail names the modules it needs). Repeat with `cascade` to switch those too.", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn set_module(
    State(state): State<SystemState>,
    RequireRecent(who, _): RequireRecent<ModulesManage>,
    Path(name): Path<String>,
    Json(change): Json<SetModule>,
) -> Result<Json<ModuleChange>, AppError> {
    let registry = state.mount.registry();
    let Some(module) = registry.get(&name) else {
        return Err(AppError::not_found("module"));
    };
    if module.manifest().module.core && !change.enabled {
        return Err(AppError::conflict(
            "module_is_core",
            "This module is part of the core and cannot be switched off.",
        ));
    }

    let mut tx = state.db.begin().await.map_err(internal)?;
    db::lock(&mut tx).await.map_err(internal)?;
    let enabled: BTreeSet<String> = db::enabled_names_in(&mut tx)
        .await
        .map_err(internal)?
        .into_iter()
        .collect();

    // What else has to change for the result to make sense.
    let others: Vec<String> = if change.enabled {
        registry
            .dependencies_of(&name)
            .into_iter()
            .filter(|dep| !enabled.contains(*dep))
            .map(str::to_owned)
            .collect()
    } else {
        registry
            .dependents_of(&name)
            .into_iter()
            .filter(|dep| enabled.contains(*dep))
            .map(str::to_owned)
            .collect()
    };
    if !others.is_empty() && !change.cascade {
        let list = others.join(", ");
        return Err(if change.enabled {
            AppError::conflict(
                "module_needs_dependencies",
                format!("This module needs modules that are switched off: {list}."),
            )
        } else {
            AppError::conflict(
                "module_has_dependents",
                format!("Modules that are switched on need this module: {list}."),
            )
        });
    }
    if !change.enabled
        && let Some(core) = others
            .iter()
            .find(|n| registry.get(n).is_some_and(|m| m.manifest().module.core))
    {
        // Cannot happen with a valid registry (core never depends on optional), but
        // the consequence would be switching off a core module, so check anyway.
        return Err(AppError::conflict(
            "module_is_core",
            format!("The core module {core} needs this module."),
        ));
    }

    let mut names = others;
    names.push(name.clone());
    let changed = db::set_enabled(&mut tx, &names, change.enabled)
        .await
        .map_err(internal)?;
    for module in &changed {
        builtpanel_audit::insert(
            &mut *tx,
            &AuditEntry::new(
                if change.enabled {
                    "admin.module_enabled"
                } else {
                    "admin.module_disabled"
                },
                Outcome::Success,
            )
            .actor(who.account_id())
            .target("module", module)
            .metadata(serde_json::json!({ "requested": name, "cascade": change.cascade })),
        )
        .await
        .map_err(internal)?;
        // Announced with the change: if the transaction fails, nobody hears of it.
        let event = if change.enabled {
            Event::ModuleEnabled {
                name: module.clone(),
            }
        } else {
            Event::ModuleDisabled {
                name: module.clone(),
            }
        };
        builtpanel_events::publish(&mut tx, &event)
            .await
            .map_err(internal)?;
    }
    let now_enabled: BTreeSet<String> = db::enabled_names_in(&mut tx)
        .await
        .map_err(internal)?
        .into_iter()
        .collect();
    tx.commit().await.map_err(internal)?;

    // After the commit: the routes follow the database, never lead it.
    if state.mount.apply(&now_enabled) {
        tracing::info!(module = %name, enabled = change.enabled, changed = ?changed, "module routes swapped");
    }
    Ok(Json(ModuleChange {
        changed,
        active: state.mount.mounted().into_iter().collect(),
    }))
}

// ----------------------------------------------------------------- settings

/// A module's global settings: what they are and what they are now.
#[derive(Debug, Serialize, ToSchema)]
pub struct SettingsView {
    /// Module name.
    #[schema(example = "system")]
    pub module: String,
    /// Display name by language code (falls back to the English `title`).
    #[schema(value_type = Object)]
    pub title: serde_json::Value,
    /// Short description by language code.
    #[schema(value_type = Object)]
    pub description: serde_json::Value,
    /// The settings as a JSON Schema: each property has `type`, `default`,
    /// optional `enum`, limits, `title` / `x-title` and `description` /
    /// `x-description`. Only the settings that apply to the whole panel.
    #[schema(value_type = Object)]
    pub schema: serde_json::Value,
    /// Current values (the default where nothing was changed), by setting key.
    #[schema(value_type = Object)]
    pub values: serde_json::Value,
}

/// All modules that have settings.
#[derive(Debug, Serialize, ToSchema)]
pub struct SettingsList {
    /// By module name.
    pub items: Vec<SettingsView>,
}

/// New values for some settings of a module.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PutSettings {
    /// Setting key to new value. Settings not mentioned keep their value;
    /// writing a setting's default puts it back to the default.
    #[schema(value_type = Object, example = json!({"panel_name": "My Hosting"}))]
    pub values: serde_json::Map<String, serde_json::Value>,
}

/// The outcome of a change.
#[derive(Debug, Serialize, ToSchema)]
pub struct SettingsChange {
    /// Current values of the module, by key.
    #[schema(value_type = Object)]
    pub values: serde_json::Value,
    /// Keys whose value actually changed (empty if nothing did).
    pub changed: Vec<String>,
}

/// Text in the schema's `x-<name>` (a language map), else its plain `<name>`
/// as an English-only map.
fn localized(schema: &serde_json::Value, name: &str) -> serde_json::Value {
    schema
        .get(format!("x-{name}"))
        .filter(|v| v.is_object())
        .cloned()
        .or_else(|| {
            schema
                .get(name)
                .and_then(|v| v.as_str())
                .map(|text| serde_json::json!({ "en": text }))
        })
        .unwrap_or_else(|| serde_json::json!({}))
}

/// The schema as shown: only properties of the global scope.
fn global_schema(settings: &ModuleSettings) -> serde_json::Value {
    let mut schema = settings.schema.clone();
    if let Some(properties) = schema
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
    {
        properties.retain(|key, _| {
            settings
                .properties
                .get(key)
                .is_some_and(|p| p.scope == builtpanel_settings::ScopeKind::Global)
        });
    }
    schema
}

fn has_global_settings(settings: &ModuleSettings) -> bool {
    settings
        .properties
        .values()
        .any(|p| p.scope == builtpanel_settings::ScopeKind::Global)
}

async fn view(state: &SystemState, settings: &ModuleSettings) -> Result<SettingsView, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values =
        builtpanel_settings::get(&state.settings, &mut conn, &settings.module, Scope::Global)
            .await
            .map_err(|e| internal(e.to_string()))?;
    Ok(SettingsView {
        module: settings.module.clone(),
        title: localized(&settings.schema, "title"),
        description: localized(&settings.schema, "description"),
        schema: global_schema(settings),
        values: serde_json::to_value(values).unwrap_or_default(),
    })
}

fn settings_error(err: SettingsError) -> AppError {
    match err {
        SettingsError::UnknownModule(_) => AppError::not_found("settings"),
        SettingsError::Invalid(problems) => AppError::validation(
            problems
                .into_iter()
                .map(|p| FieldError::new(format!("values.{}", p.key), p.code, p.message))
                .collect(),
        ),
        SettingsError::Database(e) => internal(e),
    }
}

/// List the settings of every module that has some.
#[utoipa::path(
    get,
    path = "/api/v1/admin/settings",
    tag = "system",
    operation_id = "listSettings",
    security(("session" = [])),
    responses(
        (status = 200, description = "Modules with settings, their schemas and current values", body = SettingsList),
        (status = 403, description = "`permission_denied` (needs admin.settings.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_settings(
    State(state): State<SystemState>,
    _who: Require<SettingsRead>,
) -> Result<Json<SettingsList>, AppError> {
    let mut items = Vec::new();
    for settings in state.settings.modules().filter(|m| has_global_settings(m)) {
        items.push(view(&state, settings).await?);
    }
    Ok(Json(SettingsList { items }))
}

/// One module's settings.
#[utoipa::path(
    get,
    path = "/api/v1/admin/settings/{module}",
    tag = "system",
    operation_id = "getSettings",
    security(("session" = [])),
    params(("module" = String, Path, description = "Module name")),
    responses(
        (status = 200, description = "Schema and current values", body = SettingsView),
        (status = 403, description = "`permission_denied` (needs admin.settings.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "The module has no settings", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_settings(
    State(state): State<SystemState>,
    _who: Require<SettingsRead>,
    Path(module): Path<String>,
) -> Result<Json<SettingsView>, AppError> {
    let settings = state
        .settings
        .module(&module)
        .filter(|m| has_global_settings(m))
        .ok_or_else(|| AppError::not_found("settings"))?;
    Ok(Json(view(&state, settings).await?))
}

/// Change some settings of a module. All values are checked first; if one is
/// not acceptable nothing is changed. The change, with the old and new value
/// of every setting that differs, is written to the audit log.
#[utoipa::path(
    put,
    path = "/api/v1/admin/settings/{module}",
    tag = "system",
    operation_id = "putSettings",
    security(("session" = [])),
    params(
        ("module" = String, Path, description = "Module name"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = PutSettings,
    responses(
        (status = 200, description = "Saved (also when nothing differed)", body = SettingsChange),
        (status = 403, description = "`permission_denied` (needs admin.settings.manage), `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "The module has no settings", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Unknown setting, wrong scope, or a value that breaks the schema (one entry per problem, `values.<key>`)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn put_settings(
    State(state): State<SystemState>,
    RequireRecent(who, _): RequireRecent<SettingsManage>,
    Path(module): Path<String>,
    Json(body): Json<PutSettings>,
) -> Result<Json<SettingsChange>, AppError> {
    if !state
        .settings
        .module(&module)
        .is_some_and(has_global_settings)
    {
        return Err(AppError::not_found("settings"));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    let change = builtpanel_settings::set(
        &state.settings,
        &mut tx,
        &module,
        Scope::Global,
        &body.values,
        Some(who.account_id()),
    )
    .await
    .map_err(settings_error)?;
    if !change.changed.is_empty() {
        let diff: serde_json::Map<String, serde_json::Value> = change
            .changed
            .iter()
            .map(|key| {
                (
                    key.clone(),
                    serde_json::json!({
                        "before": change.before.get(key),
                        "after": change.after.get(key),
                    }),
                )
            })
            .collect();
        builtpanel_audit::insert(
            &mut *tx,
            &AuditEntry::new("admin.settings_changed", Outcome::Success)
                .actor(who.account_id())
                .target("settings", &module)
                .metadata(serde_json::json!({ "changed": diff })),
        )
        .await
        .map_err(internal)?;
        builtpanel_events::publish(
            &mut tx,
            &Event::SettingsChanged {
                module: module.clone(),
            },
        )
        .await
        .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    if module == "system" {
        state.branding.invalidate();
    }
    Ok(Json(SettingsChange {
        values: serde_json::to_value(&change.after).unwrap_or_default(),
        changed: change.changed,
    }))
}

/// OpenAPI fragment (full paths; merge into the root document).
#[derive(OpenApi)]
#[openapi(
    paths(
        bootstrap::admin_bootstrap, bootstrap::client_bootstrap, bootstrap::public_branding, search::search, system_logs, list_modules, set_module, list_settings, get_settings,
        put_settings
    ),
    components(schemas(
        ModuleView, ModuleList, SetModule, ModuleChange, SettingsView, SettingsList, PutSettings,
        SettingsChange, SystemLogLine, SystemLogList, bootstrap::Bootstrap, bootstrap::BootstrapUser, bootstrap::BootstrapMfa,
        bootstrap::BootstrapNav, bootstrap::BootstrapModule, bootstrap::Branding, search::SearchHit
    )),
    tags((name = "system", description = "Modules and settings"))
)]
pub struct SystemApi;
