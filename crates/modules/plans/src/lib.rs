//! Plans and feature sets (phase 04).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET    | `/api/v1/admin/features` | `admin.plans.read` |
//! | GET    | `/api/v1/admin/feature-sets` | `admin.plans.read` |
//! | POST   | `/api/v1/admin/feature-sets` | `admin.plans.manage`, sudo mode |
//! | GET    | `/api/v1/admin/feature-sets/{id}` | `admin.plans.read` |
//! | PUT    | `/api/v1/admin/feature-sets/{id}` | `admin.plans.manage`, sudo mode |
//! | DELETE | `/api/v1/admin/feature-sets/{id}` | `admin.plans.manage`, sudo mode |
//!
//! Plans (`/plan-limits`, `/plans`) are in [`plans`].
//!
//! A **feature** is a key a module declares in its manifest (`[features]`). A
//! **feature set** is a named list of them, or "all features". The catalogue
//! (`/features`) is what the installed modules declare, grouped by module and
//! category; it is what the interface offers as checkboxes. A set may hold a
//! well-formed key that no installed module declares (for example a module that
//! was switched off or has not arrived yet): it is kept, enables nothing, and
//! is reported in `unknown_features`.
//!
//! Built-in sets (the presets) can be edited but not renamed or deleted; a set
//! that a plan uses cannot be deleted. Every change is audited in the same
//! transaction.

pub mod plans;

use std::collections::BTreeSet;

use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    routing::get,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    feature_sets::{self, DeleteOutcome, FeatureSet, WriteError},
    sqlx::PgPool,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_kernel::ModuleMount;
use builtpanel_rbac::{
    Require, RequireRecent,
    permissions::{PlansManage, PlansRead},
};
use builtpanel_session::{SessionStore, rfc3339};
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

/// The plans module's manifest text.
pub const MANIFEST: &str = include_str!("../../../../modules/plans/module.toml");

/// Dependencies of the plans handlers.
#[derive(Clone)]
pub struct PlansState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The modules of this build (for the feature catalogue).
    pub mount: ModuleMount,
}

impl FromRef<PlansState> for PgPool {
    fn from_ref(s: &PlansState) -> Self {
        s.db.clone()
    }
}

impl FromRef<PlansState> for SessionStore {
    fn from_ref(s: &PlansState) -> Self {
        s.sessions.clone()
    }
}

/// Routes with full paths; merge into the application router.
pub fn router() -> Router<PlansState> {
    Router::new()
        .route("/api/v1/admin/features", get(list_features))
        .route("/api/v1/admin/plan-limits", get(plans::list_limits))
        .route(
            "/api/v1/admin/plans",
            get(plans::list_plans).post(plans::create_plan),
        )
        .route(
            "/api/v1/admin/plans/{id}",
            get(plans::get_plan)
                .put(plans::update_plan)
                .delete(plans::delete_plan),
        )
        .route(
            "/api/v1/admin/feature-sets",
            get(list_feature_sets).post(create_feature_set),
        )
        .route(
            "/api/v1/admin/feature-sets/{id}",
            get(get_feature_set)
                .put(update_feature_set)
                .delete(delete_feature_set),
        )
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

// ------------------------------------------------------------- the catalogue

/// A feature a module offers.
#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureView {
    /// The key, for example `redirects`.
    #[schema(example = "redirects")]
    pub key: String,
    /// The module that declares it.
    pub module: String,
    /// That module's display name by language code.
    #[schema(value_type = Object)]
    pub module_title: std::collections::BTreeMap<String, String>,
    /// The module's category (`web`, `db`, ...), for grouping.
    pub category: String,
}

/// Every feature of the installed modules.
#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureList {
    /// By category, module, then key.
    pub items: Vec<FeatureView>,
}

fn declared_features(mount: &ModuleMount) -> Vec<FeatureView> {
    let mut items: Vec<FeatureView> = mount
        .registry()
        .modules()
        .iter()
        .flat_map(|m| {
            let manifest = m.manifest();
            manifest.features.keys.iter().map(move |key| FeatureView {
                key: key.clone(),
                module: manifest.module.name.clone(),
                module_title: manifest.module.title.clone(),
                category: manifest.module.category.as_str().to_owned(),
            })
        })
        .collect();
    items.sort_by(|a, b| (&a.category, &a.module, &a.key).cmp(&(&b.category, &b.module, &b.key)));
    items
}

/// List the features the installed modules offer (for the feature set editor).
#[utoipa::path(
    get,
    path = "/api/v1/admin/features",
    tag = "plans",
    operation_id = "listFeatures",
    security(("session" = [])),
    responses(
        (status = 200, description = "The feature catalogue", body = FeatureList),
        (status = 403, description = "`permission_denied` (needs admin.plans.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_features(
    State(state): State<PlansState>,
    _who: Require<PlansRead>,
) -> Json<FeatureList> {
    Json(FeatureList {
        items: declared_features(&state.mount),
    })
}

// -------------------------------------------------------------- feature sets

/// A feature set.
#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureSetView {
    /// Id.
    pub id: Uuid,
    /// Unique name.
    #[schema(example = "basic")]
    pub name: String,
    /// Description.
    pub description: String,
    /// Every feature of every module, including ones added later.
    pub all_features: bool,
    /// The feature keys (empty when `all_features`).
    pub features: Vec<String>,
    /// Keys in the list that no installed module declares: kept, but they
    /// enable nothing.
    pub unknown_features: Vec<String>,
    /// Created by the installation: cannot be renamed or deleted.
    pub built_in: bool,
    /// Plans that use it.
    pub plans: i64,
    /// Creation time (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
    /// Last change (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub updated_at: String,
}

fn view(set: FeatureSet, known: &BTreeSet<String>) -> FeatureSetView {
    let unknown_features = set
        .features
        .iter()
        .filter(|f| !known.contains(*f))
        .cloned()
        .collect();
    FeatureSetView {
        id: set.id,
        name: set.name,
        description: set.description,
        all_features: set.all_features,
        features: set.features,
        unknown_features,
        built_in: set.built_in,
        plans: set.plans,
        created_at: rfc3339(set.created_at),
        updated_at: rfc3339(set.updated_at),
    }
}

fn known_keys(mount: &ModuleMount) -> BTreeSet<String> {
    declared_features(mount)
        .into_iter()
        .map(|f| f.key)
        .collect()
}

/// All feature sets.
#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureSetList {
    /// By name.
    pub items: Vec<FeatureSetView>,
}

/// What a feature set is made of (create and replace).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FeatureSetInput {
    /// Unique name: 2-32 characters of `a-z`, `0-9` and `_`, starting with a
    /// letter. Ignored when updating a built-in set.
    #[schema(example = "web_only")]
    pub name: String,
    /// Description (at most 255 characters).
    #[serde(default)]
    pub description: String,
    /// Every feature of every module. Then `features` must be empty.
    #[serde(default)]
    pub all_features: bool,
    /// Feature keys (`a-z`, `0-9`, `_`; at most 256).
    #[serde(default)]
    pub features: Vec<String>,
}

fn field(name: &str, code: &str, message: &str) -> FieldError {
    FieldError::new(name, code, message)
}

fn valid_name(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn valid_feature_key(key: &str) -> bool {
    (1..=48).contains(&key.len())
        && key.starts_with(|c: char| c.is_ascii_lowercase())
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Checks the input and returns the list without duplicates, sorted.
fn check(input: &FeatureSetInput) -> Result<Vec<String>, AppError> {
    let mut errors = Vec::new();
    if !valid_name(&input.name) {
        errors.push(field(
            "name",
            "invalid_format",
            "Use 2-32 characters: a-z, 0-9 and '_', starting with a letter.",
        ));
    }
    if input.description.chars().count() > 255 || input.description.chars().any(char::is_control) {
        errors.push(field(
            "description",
            "invalid_format",
            "At most 255 characters, no control characters.",
        ));
    }
    if input.features.len() > 256 {
        errors.push(field("features", "too_many", "At most 256 features."));
    }
    for key in &input.features {
        if !valid_feature_key(key) {
            errors.push(field(
                "features",
                "invalid_format",
                "Feature keys use a-z, 0-9 and '_' (up to 48 characters), starting with a letter.",
            ));
            break;
        }
    }
    if input.all_features && !input.features.is_empty() {
        errors.push(field(
            "features",
            "conflicts_with_all_features",
            "Leave the list empty when all features are included.",
        ));
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let mut features: Vec<String> = input.features.clone();
    features.sort();
    features.dedup();
    Ok(features)
}

/// List the feature sets.
#[utoipa::path(
    get,
    path = "/api/v1/admin/feature-sets",
    tag = "plans",
    operation_id = "listFeatureSets",
    security(("session" = [])),
    responses(
        (status = 200, description = "All feature sets, by name", body = FeatureSetList),
        (status = 403, description = "`permission_denied` (needs admin.plans.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_feature_sets(
    State(state): State<PlansState>,
    _who: Require<PlansRead>,
) -> Result<Json<FeatureSetList>, AppError> {
    let known = known_keys(&state.mount);
    let items = feature_sets::list(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|s| view(s, &known))
        .collect();
    Ok(Json(FeatureSetList { items }))
}

/// One feature set.
#[utoipa::path(
    get,
    path = "/api/v1/admin/feature-sets/{id}",
    tag = "plans",
    operation_id = "getFeatureSet",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Feature set id")),
    responses(
        (status = 200, description = "The feature set", body = FeatureSetView),
        (status = 403, description = "`permission_denied` (needs admin.plans.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such feature set", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_feature_set(
    State(state): State<PlansState>,
    _who: Require<PlansRead>,
    Path(id): Path<String>,
) -> Result<Json<FeatureSetView>, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::not_found("feature_set"))?;
    let set = feature_sets::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("feature_set"))?;
    Ok(Json(view(set, &known_keys(&state.mount))))
}

fn write_error(err: WriteError) -> AppError {
    match err {
        WriteError::NameTaken => AppError::conflict(
            "feature_set_name_taken",
            "A feature set with this name already exists.",
        ),
        WriteError::Database(e) => internal(e),
    }
}

/// Create a feature set.
#[utoipa::path(
    post,
    path = "/api/v1/admin/feature-sets",
    tag = "plans",
    operation_id = "createFeatureSet",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = FeatureSetInput,
    responses(
        (status = 201, description = "Created", body = FeatureSetView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`feature_set_name_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid fields", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_feature_set(
    State(state): State<PlansState>,
    RequireRecent(who, _): RequireRecent<PlansManage>,
    Json(input): Json<FeatureSetInput>,
) -> Result<(StatusCode, Json<FeatureSetView>), AppError> {
    let features = check(&input)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let id = feature_sets::insert(
        &mut tx,
        &input.name,
        &input.description,
        input.all_features,
        &features,
    )
    .await
    .map_err(write_error)?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("admin.feature_set_created", Outcome::Success)
            .actor(who.account_id())
            .target("feature_set", id)
            .metadata(serde_json::json!({
                "name": input.name,
                "all_features": input.all_features,
                "features": features,
            })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let set = feature_sets::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("created feature set vanished")))?;
    Ok((
        StatusCode::CREATED,
        Json(view(set, &known_keys(&state.mount))),
    ))
}

/// Replace a feature set's contents.
#[utoipa::path(
    put,
    path = "/api/v1/admin/feature-sets/{id}",
    tag = "plans",
    operation_id = "updateFeatureSet",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Feature set id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = FeatureSetInput,
    responses(
        (status = 200, description = "Saved", body = FeatureSetView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such feature set", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`feature_set_name_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid fields", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn update_feature_set(
    State(state): State<PlansState>,
    RequireRecent(who, _): RequireRecent<PlansManage>,
    Path(id): Path<String>,
    Json(input): Json<FeatureSetInput>,
) -> Result<Json<FeatureSetView>, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::not_found("feature_set"))?;
    let features = check(&input)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let before = feature_sets::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("feature_set"))?;
    let found = feature_sets::update(
        &mut tx,
        id,
        &input.name,
        &input.description,
        input.all_features,
        &features,
    )
    .await
    .map_err(write_error)?;
    if !found {
        return Err(AppError::not_found("feature_set"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("admin.feature_set_updated", Outcome::Success)
            .actor(who.account_id())
            .target("feature_set", id)
            .metadata(serde_json::json!({
                "before": {
                    "name": before.name,
                    "all_features": before.all_features,
                    "features": before.features,
                },
                "after": {
                    "name": if before.built_in { before.name.clone() } else { input.name.clone() },
                    "all_features": input.all_features,
                    "features": features,
                },
            })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let set = feature_sets::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("feature_set"))?;
    Ok(Json(view(set, &known_keys(&state.mount))))
}

/// Delete a feature set that no plan uses.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/feature-sets/{id}",
    tag = "plans",
    operation_id = "deleteFeatureSet",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Feature set id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such feature set", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`feature_set_built_in` or `feature_set_in_use`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_feature_set(
    State(state): State<PlansState>,
    RequireRecent(who, _): RequireRecent<PlansManage>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::not_found("feature_set"))?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    match feature_sets::delete(&mut tx, id).await.map_err(internal)? {
        DeleteOutcome::Unknown => return Err(AppError::not_found("feature_set")),
        DeleteOutcome::BuiltIn => {
            return Err(AppError::conflict(
                "feature_set_built_in",
                "Built-in feature sets cannot be deleted.",
            ));
        }
        DeleteOutcome::InUse => {
            return Err(AppError::conflict(
                "feature_set_in_use",
                "Plans use this feature set; move them to another one first.",
            ));
        }
        DeleteOutcome::Deleted => {}
    }
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("admin.feature_set_deleted", Outcome::Success)
            .actor(who.account_id())
            .target("feature_set", id),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// OpenAPI fragment (full paths; merge into the root document).
#[derive(OpenApi)]
#[openapi(
    paths(
        list_features, list_feature_sets, get_feature_set, create_feature_set,
        update_feature_set, delete_feature_set, plans::list_limits, plans::list_plans,
        plans::get_plan, plans::create_plan, plans::update_plan, plans::delete_plan
    ),
    components(schemas(
        FeatureView, FeatureList, FeatureSetView, FeatureSetList, FeatureSetInput,
        plans::LimitView, plans::LimitList, plans::Resources, plans::PlanView, plans::PlanList,
        plans::PlanInput
    )),
    tags((name = "plans", description = "Feature sets and hosting plans"))
)]
pub struct PlansApi;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn input(name: &str, features: &[&str], all: bool) -> FeatureSetInput {
        FeatureSetInput {
            name: name.into(),
            description: String::new(),
            all_features: all,
            features: features.iter().map(|f| (*f).to_owned()).collect(),
        }
    }

    #[test]
    fn input_is_checked_and_cleaned() {
        let list = check(&input("web_only", &["ssl", "dns", "ssl"], false)).unwrap();
        assert_eq!(list, ["dns", "ssl"], "sorted, no duplicates");
        assert!(check(&input("everything", &[], true)).is_ok());
        for bad in [
            input("X", &[], false),
            input("1abc", &[], false),
            input("has space", &[], false),
            input("ok_name", &["Bad Key"], false),
            input("ok_name", &["bad,key"], false),
            input("ok_name", &[""], false),
            input("ok_name", &["dns"], true),
        ] {
            assert!(check(&bad).is_err(), "{bad:?}");
        }
        let mut long = input("ok_name", &[], false);
        long.description = "x".repeat(256);
        assert!(check(&long).is_err());
        long.description = "line\nbreak".into();
        assert!(check(&long).is_err());
        let many: Vec<String> = (0..257).map(|i| format!("f{i}")).collect();
        let mut too_many = input("ok_name", &[], false);
        too_many.features = many;
        assert!(check(&too_many).is_err());
    }
}
