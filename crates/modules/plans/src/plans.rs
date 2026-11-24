//! Hosting plans: what an account is given.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET    | `/api/v1/admin/plan-limits` | `admin.plans.read` |
//! | GET    | `/api/v1/admin/plans` | `admin.plans.read` |
//! | POST   | `/api/v1/admin/plans` | `admin.plans.manage`, sudo mode |
//! | GET    | `/api/v1/admin/plans/{id}` | `admin.plans.read` |
//! | PUT    | `/api/v1/admin/plans/{id}` | `admin.plans.manage`, sudo mode |
//! | DELETE | `/api/v1/admin/plans/{id}` | `admin.plans.manage`, sudo mode |
//!
//! Resources use `-1` for "unlimited" on the wire (and NULL in the table);
//! memory, disk and bandwidth are in megabytes, CPU in thousandths of a core.
//! Module limits (`max_domains`, ...) come from the manifests' `[limits]`:
//! a plan stores a value for every declared limit (the manifest's default when
//! the request leaves one out), and keys no installed module declares are kept
//! and reported in `unknown_limits`.
//!
//! A plan may not promise more than the biggest node has: CPU, memory and disk
//! above the largest reported machine are refused with 422
//! `exceeds_node_capacity`. When no node has reported its capabilities yet
//! there is nothing to compare with and the check is skipped.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::plans::{self, DeleteOutcome, NodeMaximum, Plan, PlanWrite, WriteError};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Require, RequireRecent,
    permissions::{PlansManage, PlansRead},
};
use builtpanel_session::rfc3339;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::PlansState;

const MB: i64 = 1024 * 1024;

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

// --------------------------------------------------------------- the limits

/// A numeric limit a module declares.
#[derive(Debug, Serialize, ToSchema)]
pub struct LimitView {
    /// The key, for example `max_domains`.
    #[schema(example = "max_domains")]
    pub key: String,
    /// The module that declares it.
    pub module: String,
    /// That module's display name by language code.
    #[schema(value_type = Object)]
    pub module_title: BTreeMap<String, String>,
    /// The module's category.
    pub category: String,
    /// The value a new plan gets.
    pub default: i64,
    /// Whether `-1` (unlimited) is allowed.
    pub unlimited_allowed: bool,
}

/// Every limit of the installed modules.
#[derive(Debug, Serialize, ToSchema)]
pub struct LimitList {
    /// By category, module, then key.
    pub items: Vec<LimitView>,
}

fn declared_limits(state: &PlansState) -> Vec<LimitView> {
    let mut items: Vec<LimitView> = state
        .mount
        .registry()
        .modules()
        .iter()
        .flat_map(|m| {
            let manifest = m.manifest();
            manifest.limits.iter().map(move |(key, def)| LimitView {
                key: key.clone(),
                module: manifest.module.name.clone(),
                module_title: manifest.module.title.clone(),
                category: manifest.module.category.as_str().to_owned(),
                default: def.default,
                unlimited_allowed: def.unlimited == Some(-1),
            })
        })
        .collect();
    items.sort_by(|a, b| (&a.category, &a.module, &a.key).cmp(&(&b.category, &b.module, &b.key)));
    items
}

/// List the numeric limits the installed modules declare (for the plan form).
#[utoipa::path(
    get,
    path = "/api/v1/admin/plan-limits",
    tag = "plans",
    operation_id = "listPlanLimits",
    security(("session" = [])),
    responses(
        (status = 200, description = "The limit catalogue", body = LimitList),
        (status = 403, description = "`permission_denied` (needs admin.plans.read)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_limits(
    State(state): State<PlansState>,
    _who: Require<PlansRead>,
) -> Json<LimitList> {
    Json(LimitList {
        items: declared_limits(&state),
    })
}

// ------------------------------------------------------------------- plans

/// The resources of a plan's containers. `-1` means unlimited.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Resources {
    /// CPU in thousandths of a core (1000 = one core); 10 to 256000.
    #[schema(example = 1000)]
    pub cpu_millis: i64,
    /// Memory in megabytes; 6 to 1048576.
    #[schema(example = 512)]
    pub memory_mb: i64,
    /// Disk space in megabytes; 1 or more.
    #[schema(example = 5120)]
    pub disk_mb: i64,
    /// Most files and folders; 1000 to 1000000000.
    #[serde(default = "unlimited")]
    pub inodes: i64,
    /// Transfer per month in megabytes; 0 or more.
    #[serde(default = "unlimited")]
    pub bandwidth_mb: i64,
    /// Most processes; 1 to 32768.
    #[schema(example = 128)]
    pub pids: i64,
}

fn unlimited() -> i64 {
    -1
}

/// A plan.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlanView {
    /// Id.
    pub id: Uuid,
    /// Unique name.
    #[schema(example = "basic")]
    pub name: String,
    /// Description.
    pub description: String,
    /// The feature set.
    pub feature_set_id: Uuid,
    /// Its name.
    pub feature_set_name: String,
    /// New accounts get this plan unless told otherwise.
    pub is_default: bool,
    /// Accounts on this plan.
    pub users_count: i64,
    /// Container resources.
    pub resources: Resources,
    /// Module limits by key; `-1` is unlimited.
    #[schema(value_type = Object)]
    pub limits: BTreeMap<String, i64>,
    /// Limit keys that no installed module declares: kept, but they limit nothing.
    pub unknown_limits: Vec<String>,
    /// `standard` or `strict`.
    #[schema(example = "standard")]
    pub isolation: String,
    /// Created by the installation: cannot be renamed or deleted.
    pub built_in: bool,
    /// Creation time (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
    /// Last change (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub updated_at: String,
}

fn from_bytes(value: Option<i64>) -> i64 {
    value.map_or(-1, |bytes| bytes / MB)
}

fn view(plan: Plan, declared: &[LimitView]) -> PlanView {
    let limits: BTreeMap<String, i64> = plan
        .limits
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_i64().map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default();
    let unknown_limits = limits
        .keys()
        .filter(|k| !declared.iter().any(|d| &d.key == *k))
        .cloned()
        .collect();
    PlanView {
        id: plan.id,
        name: plan.name,
        description: plan.description,
        feature_set_id: plan.feature_set_id,
        feature_set_name: plan.feature_set_name,
        is_default: plan.is_default,
        users_count: plan.users,
        resources: Resources {
            cpu_millis: plan.cpu_millis.map_or(-1, i64::from),
            memory_mb: from_bytes(plan.memory_bytes),
            disk_mb: from_bytes(plan.disk_bytes),
            inodes: plan.inodes.unwrap_or(-1),
            bandwidth_mb: from_bytes(plan.bandwidth_bytes),
            pids: plan.pids.map_or(-1, i64::from),
        },
        limits,
        unknown_limits,
        isolation: plan.isolation,
        built_in: plan.built_in,
        created_at: rfc3339(plan.created_at),
        updated_at: rfc3339(plan.updated_at),
    }
}

/// All plans.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlanList {
    /// By name.
    pub items: Vec<PlanView>,
}

/// What a plan is made of (create and replace).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    /// Unique name: 2-32 characters of `a-z`, `0-9` and `_`, starting with a
    /// letter. Ignored when updating a built-in plan.
    #[schema(example = "starter")]
    pub name: String,
    /// Description (at most 255 characters).
    #[serde(default)]
    pub description: String,
    /// The feature set.
    pub feature_set_id: Uuid,
    /// Make this the default plan for new accounts.
    #[serde(default)]
    pub is_default: bool,
    /// Container resources.
    pub resources: Resources,
    /// Module limits by key: `-1` (unlimited) or zero and more. Declared limits
    /// left out get their default.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub limits: BTreeMap<String, i64>,
    /// `standard` or `strict`.
    #[serde(default = "standard")]
    pub isolation: String,
}

fn standard() -> String {
    "standard".to_owned()
}

fn valid_name(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn valid_limit_key(key: &str) -> bool {
    (1..=48).contains(&key.len())
        && key.starts_with(|c: char| c.is_ascii_lowercase())
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A value in `low..=high`, or `-1` for unlimited (when `unlimited_ok`).
fn in_range(
    errors: &mut Vec<FieldError>,
    field: &str,
    value: i64,
    low: i64,
    high: i64,
    unlimited_ok: bool,
) {
    let fine = (low..=high).contains(&value) || (unlimited_ok && value == -1);
    if !fine {
        errors.push(FieldError::new(
            format!("resources.{field}"),
            "out_of_range",
            format!(
                "Use {low} to {high}{}.",
                if unlimited_ok {
                    ", or -1 for unlimited"
                } else {
                    ""
                }
            ),
        ));
    }
}

/// A plan, checked and converted to what the table stores.
#[derive(Debug)]
struct Checked {
    cpu_millis: Option<i32>,
    memory_bytes: Option<i64>,
    pids: Option<i32>,
    disk_bytes: Option<i64>,
    inodes: Option<i64>,
    bandwidth_bytes: Option<i64>,
    limits: serde_json::Value,
}

fn check(input: &PlanInput, declared: &[LimitView]) -> Result<Checked, AppError> {
    let mut errors = Vec::new();
    if !valid_name(&input.name) {
        errors.push(FieldError::new(
            "name",
            "invalid_format",
            "Use 2-32 characters: a-z, 0-9 and '_', starting with a letter.",
        ));
    }
    if input.description.chars().count() > 255 || input.description.chars().any(char::is_control) {
        errors.push(FieldError::new(
            "description",
            "invalid_format",
            "At most 255 characters, no control characters.",
        ));
    }
    if !matches!(input.isolation.as_str(), "standard" | "strict") {
        errors.push(FieldError::new(
            "isolation",
            "invalid_value",
            "Use standard or strict.",
        ));
    }
    let r = &input.resources;
    in_range(&mut errors, "cpu_millis", r.cpu_millis, 10, 256_000, true);
    in_range(&mut errors, "memory_mb", r.memory_mb, 6, 1_048_576, true);
    in_range(&mut errors, "disk_mb", r.disk_mb, 1, 1_073_741_824, true);
    in_range(&mut errors, "inodes", r.inodes, 1_000, 1_000_000_000, true);
    in_range(
        &mut errors,
        "bandwidth_mb",
        r.bandwidth_mb,
        0,
        i64::MAX / MB,
        true,
    );
    in_range(&mut errors, "pids", r.pids, 1, 32_768, true);

    // Declared limits get their default; whatever is given must be -1 or 0 and more.
    let mut limits: BTreeMap<String, i64> = declared
        .iter()
        .map(|d| (d.key.clone(), d.default))
        .collect();
    for (key, value) in &input.limits {
        if !valid_limit_key(key) {
            errors.push(FieldError::new(
                "limits",
                "invalid_key",
                "Limit keys use a-z, 0-9 and '_', starting with a letter.",
            ));
            continue;
        }
        let allows_unlimited = declared
            .iter()
            .find(|d| &d.key == key)
            .is_none_or(|d| d.unlimited_allowed);
        let fine = *value >= 0 || (*value == -1 && allows_unlimited);
        if !fine {
            errors.push(FieldError::new(
                format!("limits.{key}"),
                "out_of_range",
                if allows_unlimited {
                    "Use 0 or more, or -1 for unlimited."
                } else {
                    "Use 0 or more."
                },
            ));
        }
        limits.insert(key.clone(), *value);
    }
    if limits.len() > 128 {
        errors.push(FieldError::new("limits", "too_many", "At most 128 limits."));
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let unlimited_or = |value: i64| (value != -1).then_some(value);
    let mb = |value: i64| unlimited_or(value).and_then(|n| n.checked_mul(MB));
    Ok(Checked {
        cpu_millis: unlimited_or(r.cpu_millis).and_then(|n| i32::try_from(n).ok()),
        memory_bytes: mb(r.memory_mb),
        pids: unlimited_or(r.pids).and_then(|n| i32::try_from(n).ok()),
        disk_bytes: mb(r.disk_mb),
        inodes: unlimited_or(r.inodes),
        bandwidth_bytes: mb(r.bandwidth_mb),
        limits: serde_json::to_value(limits).unwrap_or_default(),
    })
}

/// A plan may not promise more than the biggest node has. `-1` (unlimited)
/// means "whatever the node has" and is always allowed.
fn against_nodes(checked: &Checked, max: NodeMaximum) -> Result<(), AppError> {
    let mut errors = Vec::new();
    let mut compare = |field: &str, wanted: Option<i64>, available: Option<i64>, unit: &str| {
        if let (Some(wanted), Some(available)) = (wanted, available)
            && wanted > available
        {
            errors.push(FieldError::new(
                format!("resources.{field}"),
                "exceeds_node_capacity",
                format!("The largest node has only {unit}."),
            ));
        }
    };
    compare(
        "cpu_millis",
        checked.cpu_millis.map(i64::from),
        max.cpu_cores.map(|c| i64::from(c) * 1000),
        &format!("{} CPU core(s)", max.cpu_cores.unwrap_or(0)),
    );
    compare(
        "memory_mb",
        checked.memory_bytes,
        max.memory_bytes,
        &format!("{} MB of memory", max.memory_bytes.unwrap_or(0) / MB),
    );
    compare(
        "disk_mb",
        checked.disk_bytes,
        max.disk_bytes,
        &format!("{} MB of disk", max.disk_bytes.unwrap_or(0) / MB),
    );
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::validation(errors))
    }
}

fn write_error(err: WriteError) -> AppError {
    match err {
        WriteError::NameTaken => {
            AppError::conflict("plan_name_taken", "A plan with this name already exists.")
        }
        WriteError::UnknownFeatureSet => AppError::validation(vec![FieldError::new(
            "feature_set_id",
            "unknown",
            "No such feature set.",
        )]),
        WriteError::Database(e) => internal(e),
    }
}

/// List the plans.
#[utoipa::path(
    get,
    path = "/api/v1/admin/plans",
    tag = "plans",
    operation_id = "listPlans",
    security(("session" = [])),
    responses(
        (status = 200, description = "All plans, by name", body = PlanList),
        (status = 403, description = "`permission_denied` (needs admin.plans.read)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_plans(
    State(state): State<PlansState>,
    _who: Require<PlansRead>,
) -> Result<Json<PlanList>, AppError> {
    let declared = declared_limits(&state);
    let items = plans::list(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|p| view(p, &declared))
        .collect();
    Ok(Json(PlanList { items }))
}

/// One plan.
#[utoipa::path(
    get,
    path = "/api/v1/admin/plans/{id}",
    tag = "plans",
    operation_id = "getPlan",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Plan id")),
    responses(
        (status = 200, description = "The plan", body = PlanView),
        (status = 403, description = "`permission_denied` (needs admin.plans.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such plan", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_plan(
    State(state): State<PlansState>,
    _who: Require<PlansRead>,
    Path(id): Path<String>,
) -> Result<Json<PlanView>, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::not_found("plan"))?;
    let plan = plans::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("plan"))?;
    Ok(Json(view(plan, &declared_limits(&state))))
}

fn audit_snapshot(plan: &PlanView) -> serde_json::Value {
    serde_json::json!({
        "name": plan.name,
        "feature_set": plan.feature_set_name,
        "is_default": plan.is_default,
        "resources": plan.resources,
        "limits": plan.limits,
        "isolation": plan.isolation,
    })
}

/// Create a plan.
#[utoipa::path(
    post,
    path = "/api/v1/admin/plans",
    tag = "plans",
    operation_id = "createPlan",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = PlanInput,
    responses(
        (status = 201, description = "Created", body = PlanView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`plan_name_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid fields, an unknown feature set, or `exceeds_node_capacity`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_plan(
    State(state): State<PlansState>,
    RequireRecent(who, _): RequireRecent<PlansManage>,
    Json(input): Json<PlanInput>,
) -> Result<(StatusCode, Json<PlanView>), AppError> {
    let declared = declared_limits(&state);
    let checked = check(&input, &declared)?;
    against_nodes(
        &checked,
        plans::node_maximum(&state.db).await.map_err(internal)?,
    )?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let id = plans::insert(&mut tx, &write(&input, &checked))
        .await
        .map_err(write_error)?;
    if input.is_default {
        plans::make_default(&mut tx, id).await.map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    let created = view(
        plans::get(&state.db, id)
            .await
            .map_err(internal)?
            .ok_or_else(|| internal(std::io::Error::other("created plan vanished")))?,
        &declared,
    );
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("admin.plan_created", Outcome::Success)
            .actor(who.account_id())
            .target("plan", id)
            .metadata(audit_snapshot(&created)),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(created)))
}

fn write<'a>(input: &'a PlanInput, checked: &'a Checked) -> PlanWrite<'a> {
    PlanWrite {
        name: &input.name,
        description: &input.description,
        feature_set_id: input.feature_set_id,
        cpu_millis: checked.cpu_millis,
        memory_bytes: checked.memory_bytes,
        pids: checked.pids,
        disk_bytes: checked.disk_bytes,
        inodes: checked.inodes,
        bandwidth_bytes: checked.bandwidth_bytes,
        limits: &checked.limits,
        isolation: &input.isolation,
    }
}

/// Replace a plan's contents.
#[utoipa::path(
    put,
    path = "/api/v1/admin/plans/{id}",
    tag = "plans",
    operation_id = "updatePlan",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Plan id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = PlanInput,
    responses(
        (status = 200, description = "Saved", body = PlanView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such plan", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`plan_name_taken` or `plan_is_default` (the default plan cannot stop being the default; make another plan the default instead)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid fields, an unknown feature set, or `exceeds_node_capacity`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn update_plan(
    State(state): State<PlansState>,
    RequireRecent(who, _): RequireRecent<PlansManage>,
    Path(id): Path<String>,
    Json(input): Json<PlanInput>,
) -> Result<Json<PlanView>, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::not_found("plan"))?;
    let declared = declared_limits(&state);
    let checked = check(&input, &declared)?;
    against_nodes(
        &checked,
        plans::node_maximum(&state.db).await.map_err(internal)?,
    )?;
    let before = view(
        plans::get(&state.db, id)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::not_found("plan"))?,
        &declared,
    );
    if before.is_default && !input.is_default {
        return Err(AppError::conflict(
            "plan_is_default",
            "This is the default plan. Make another plan the default instead.",
        ));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !plans::update(&mut tx, id, &write(&input, &checked))
        .await
        .map_err(write_error)?
    {
        return Err(AppError::not_found("plan"));
    }
    if input.is_default {
        plans::make_default(&mut tx, id).await.map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    let after = view(
        plans::get(&state.db, id)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::not_found("plan"))?,
        &declared,
    );
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("admin.plan_updated", Outcome::Success)
            .actor(who.account_id())
            .target("plan", id)
            .metadata(serde_json::json!({
                "before": audit_snapshot(&before),
                "after": audit_snapshot(&after),
            })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(after))
}

/// Delete a plan that no account is on.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/plans/{id}",
    tag = "plans",
    operation_id = "deletePlan",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Plan id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such plan", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`plan_built_in`, `plan_is_default` or `plan_in_use`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_plan(
    State(state): State<PlansState>,
    RequireRecent(who, _): RequireRecent<PlansManage>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::not_found("plan"))?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    match plans::delete(&mut tx, id).await.map_err(internal)? {
        DeleteOutcome::Unknown => return Err(AppError::not_found("plan")),
        DeleteOutcome::BuiltIn => {
            return Err(AppError::conflict(
                "plan_built_in",
                "Built-in plans cannot be deleted.",
            ));
        }
        DeleteOutcome::IsDefault => {
            return Err(AppError::conflict(
                "plan_is_default",
                "This is the default plan. Make another plan the default first.",
            ));
        }
        DeleteOutcome::InUse => {
            return Err(AppError::conflict(
                "plan_in_use",
                "Accounts are on this plan; move them first.",
            ));
        }
        DeleteOutcome::Deleted => {}
    }
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("admin.plan_deleted", Outcome::Success)
            .actor(who.account_id())
            .target("plan", id),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn input() -> PlanInput {
        PlanInput {
            name: "starter".into(),
            description: String::new(),
            feature_set_id: Uuid::nil(),
            is_default: false,
            resources: Resources {
                cpu_millis: 1000,
                memory_mb: 512,
                disk_mb: 5120,
                inodes: -1,
                bandwidth_mb: -1,
                pids: 128,
            },
            limits: BTreeMap::new(),
            isolation: "standard".into(),
        }
    }

    fn declared() -> Vec<LimitView> {
        vec![
            LimitView {
                key: "max_domains".into(),
                module: "domains".into(),
                module_title: BTreeMap::new(),
                category: "web".into(),
                default: 10,
                unlimited_allowed: true,
            },
            LimitView {
                key: "max_seats".into(),
                module: "seats".into(),
                module_title: BTreeMap::new(),
                category: "web".into(),
                default: 3,
                unlimited_allowed: false,
            },
        ]
    }

    fn fields(err: AppError) -> Vec<String> {
        match err {
            AppError::Validation { errors } => {
                errors.into_iter().map(|e| e.field.to_string()).collect()
            }
            other => panic!("not a validation error: {other:?}"),
        }
    }

    #[test]
    fn a_good_plan_is_converted_and_declared_limits_get_their_defaults() {
        let checked = check(&input(), &declared()).unwrap();
        assert_eq!(checked.cpu_millis, Some(1000));
        assert_eq!(checked.memory_bytes, Some(512 * MB));
        assert_eq!(checked.disk_bytes, Some(5120 * MB));
        assert_eq!(
            (checked.inodes, checked.bandwidth_bytes),
            (None, None),
            "-1 is NULL"
        );
        assert_eq!(
            checked.limits,
            serde_json::json!({"max_domains": 10, "max_seats": 3})
        );

        let mut custom = input();
        custom.limits.insert("max_domains".into(), -1);
        custom.limits.insert("mystery".into(), 7);
        let checked = check(&custom, &declared()).unwrap();
        assert_eq!(
            checked.limits,
            serde_json::json!({"max_domains": -1, "max_seats": 3, "mystery": 7})
        );
    }

    #[test]
    fn every_bad_field_is_named() {
        let mut bad = input();
        bad.name = "Bad Name".into();
        bad.isolation = "paranoid".into();
        bad.resources = Resources {
            cpu_millis: 5,
            memory_mb: 1,
            disk_mb: 0,
            inodes: 10,
            bandwidth_mb: -5,
            pids: 99_999,
        };
        bad.limits.insert("max_seats".into(), -1); // this limit cannot be unlimited
        bad.limits.insert("max_domains".into(), -2);
        bad.limits.insert("Bad Key".into(), 1);
        let mut found = fields(check(&bad, &declared()).unwrap_err());
        found.sort();
        assert_eq!(
            found,
            [
                "isolation",
                "limits",
                "limits.max_domains",
                "limits.max_seats",
                "name",
                "resources.bandwidth_mb",
                "resources.cpu_millis",
                "resources.disk_mb",
                "resources.inodes",
                "resources.memory_mb",
                "resources.pids"
            ]
        );
    }

    #[test]
    fn a_plan_cannot_promise_more_than_the_biggest_node() {
        let checked = check(&input(), &declared()).unwrap();
        let node = NodeMaximum {
            cpu_cores: Some(2),
            memory_bytes: Some(4096 * MB),
            disk_bytes: Some(100_000 * MB),
        };
        assert!(against_nodes(&checked, node).is_ok());
        let small = NodeMaximum {
            cpu_cores: Some(2),
            memory_bytes: Some(256 * MB),
            disk_bytes: Some(1024 * MB),
        };
        let mut found = fields(against_nodes(&checked, small).unwrap_err());
        found.sort();
        assert_eq!(found, ["resources.disk_mb", "resources.memory_mb"]);
        // Unlimited means "what the node has"; unknown nodes give nothing to compare with.
        let mut open = input();
        open.resources.cpu_millis = -1;
        open.resources.memory_mb = -1;
        open.resources.disk_mb = -1;
        assert!(against_nodes(&check(&open, &declared()).unwrap(), small).is_ok());
        assert!(against_nodes(&checked, NodeMaximum::default()).is_ok());
        let mut too_much_cpu = input();
        too_much_cpu.resources.cpu_millis = 2001;
        assert_eq!(
            fields(against_nodes(&check(&too_much_cpu, &declared()).unwrap(), node).unwrap_err()),
            ["resources.cpu_millis"]
        );
    }
}
