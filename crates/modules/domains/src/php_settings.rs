//! The php.ini options of an account (T06.10).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, PUT | `/api/v1/client/php/settings` | a client; the `php_ini` feature to change |
//! | GET, PUT | `/api/v1/admin/domains/{id}/php/settings` | `admin.domains.read` / `.manage` (sudo mode) |
//!
//! Only a short list of options can be set (`memory_limit`, `upload_max_filesize`,
//! `post_max_size`, `max_execution_time`, `max_input_time`, `max_input_vars`,
//! `session.gc_maxlifetime`, `display_errors`, `short_open_tag`,
//! `error_reporting`, `date.timezone`), each of its own kind. The plan sets
//! ceilings for memory, upload size and run time (the `php_memory_limit_mb`,
//! `php_upload_max_mb` and `php_max_execution_time` limits); a value above one is
//! refused with 422 `above_plan_limit`. `PUT` replaces all the options the
//! account chose; an empty object puts everything back to the defaults. The
//! options apply to every PHP version of the account and reach the running
//! containers a moment after they are saved.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use builtpanel_database::{php as store, plans};
use builtpanel_domain::php_ini::{self, Caps, IniSettings};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{DomainsManage, DomainsRead},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{DomainsState, Hosting, client_hosting, internal, tenant_of_domain};

/// Routes for administrators.
pub fn admin_router() -> Router<DomainsState> {
    Router::new().route(
        "/api/v1/admin/domains/{id}/php/settings",
        get(admin_get).put(admin_set),
    )
}

/// Routes for clients.
pub fn client_router() -> Router<DomainsState> {
    Router::new().route(
        "/api/v1/client/php/settings",
        get(client_get).put(client_set),
    )
}

/// An option that may be set.
#[derive(Debug, Serialize, ToSchema)]
pub struct OptionView {
    /// The option, as in php.ini.
    #[schema(example = "memory_limit")]
    pub key: String,
    /// `size` (like 64M), `int`, `flag` (true or false), `choice` or `timezone`.
    pub kind: String,
    /// What PHP uses when it is not set.
    pub default: String,
    /// The texts a `choice` may take.
    pub choices: Vec<String>,
}

/// The ceilings of the plan.
#[derive(Debug, Serialize, ToSchema)]
pub struct CapsView {
    /// Most `memory_limit`, in megabytes.
    pub memory_limit_mb: u64,
    /// Most `upload_max_filesize` and `post_max_size`, in megabytes.
    pub upload_max_mb: u64,
    /// Most `max_execution_time` and `max_input_time`, in seconds.
    pub max_execution_time: i64,
}

/// The options of an account.
#[derive(Debug, Serialize, ToSchema)]
pub struct PhpSettingsView {
    /// The options that can be set.
    pub options: Vec<OptionView>,
    /// What the account chose (option to text; flags are `On` or `Off`).
    pub values: serde_json::Value,
    /// What the plan allows at most.
    pub plan_limits: CapsView,
}

/// The options to keep.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPhpSettings {
    /// Option to value: sizes as text (`"64M"`), numbers as numbers, flags as
    /// true or false. Everything not given goes back to its default.
    pub values: serde_json::Map<String, serde_json::Value>,
}

fn caps_of(hosting: &Hosting) -> Caps {
    Caps::from_limits(&hosting.plan.limits)
}

async fn show(state: &DomainsState, hosting: &Hosting) -> Result<PhpSettingsView, AppError> {
    let caps = caps_of(hosting);
    let chosen = store::settings(&state.db, hosting.tenant.id)
        .await
        .map_err(internal)?;
    Ok(PhpSettingsView {
        options: php_ini::options()
            .into_iter()
            .map(|(key, kind, default)| OptionView {
                key: key.to_owned(),
                kind: kind.to_owned(),
                default: default.to_owned(),
                choices: php_ini::choices(key)
                    .iter()
                    .map(|c| (*c).to_owned())
                    .collect(),
            })
            .collect(),
        values: IniSettings::from_stored(&chosen).to_json(),
        plan_limits: CapsView {
            memory_limit_mb: caps.memory_bytes >> 20,
            upload_max_mb: caps.upload_bytes >> 20,
            max_execution_time: caps.seconds,
        },
    })
}

async fn set(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    req: SetPhpSettings,
) -> Result<Json<PhpSettingsView>, AppError> {
    if !plans::includes_feature(&state.db, hosting.plan.id, "php_ini")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Changing PHP options is not included in this plan.",
        ));
    }
    let checked = IniSettings::parse(&req.values, &caps_of(hosting)).map_err(|e| {
        AppError::validation(vec![FieldError::new(
            "values",
            e.code,
            format!("{}: {}", e.key, e.message),
        )])
    })?;
    let before = store::settings(&state.db, hosting.tenant.id)
        .await
        .map_err(internal)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let json = checked.to_json();
    // What is already so is not written again (a write makes the node be told).
    if serde_json::Value::Object(before) != json {
        store::set_settings(&mut tx, hosting.tenant.id, &json)
            .await
            .map_err(internal)?;
        builtpanel_audit::insert(
            &mut *tx,
            &builtpanel_audit::AuditEntry::new(
                "php.settings_changed",
                builtpanel_audit::Outcome::Success,
            )
            .actor(who.account_id())
            .tenant(hosting.tenant.id)
            .metadata(
                serde_json::json!({ "options": checked.values().keys().collect::<Vec<_>>() }),
            ),
        )
        .await
        .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(show(state, hosting).await?))
}

/// The php.ini options you can set, what you chose and what your plan allows.
#[utoipa::path(
    get,
    path = "/api/v1/client/php/settings",
    tag = "domains",
    operation_id = "clientGetPhpSettings",
    security(("session" = [])),
    responses(
        (status = 200, description = "The options", body = PhpSettingsView),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_get(
    State(state): State<DomainsState>,
    who: Principal,
) -> Result<Json<PhpSettingsView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok(Json(show(&state, &h).await?))
}

/// Replace your php.ini options (an empty object puts every option back to its default).
#[utoipa::path(
    put,
    path = "/api/v1/client/php/settings",
    tag = "domains",
    operation_id = "clientSetPhpSettings",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SetPhpSettings,
    responses(
        (status = 200, description = "Saved", body = PhpSettingsView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`unknown_option`, `invalid_value`, `out_of_range`, `above_plan_limit` or `post_smaller_than_upload` (the message names the option)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_set(
    State(state): State<DomainsState>,
    who: Principal,
    Json(req): Json<SetPhpSettings>,
) -> Result<Json<PhpSettingsView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    set(&state, &who, &h, req).await
}

/// The php.ini options of the account a domain belongs to.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains/{id}/php/settings",
    tag = "domains",
    operation_id = "adminGetPhpSettings",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The options", body = PhpSettingsView),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_get(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<PhpSettingsView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok(Json(show(&state, &h).await?))
}

/// Replace the php.ini options of the account a domain belongs to.
#[utoipa::path(
    put,
    path = "/api/v1/admin/domains/{id}/php/settings",
    tag = "domains",
    operation_id = "adminSetPhpSettings",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = SetPhpSettings,
    responses(
        (status = 200, description = "Saved", body = PhpSettingsView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "As for the account's own change", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_set(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SetPhpSettings>,
) -> Result<Json<PhpSettingsView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    set(&state, &who, &h, req).await
}
