//! Containers a client runs from images the administrator approved (T12.12).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/admin/container-images` | `admin.apps.read`, `admin.apps.manage` |
//! | PUT, DELETE | `/api/v1/admin/container-images/{id}` | `admin.apps.manage` |
//! | GET | `/api/v1/client/container-images` | the owner; the `runtime_apps` feature |
//! | GET, POST | `/api/v1/client/container-apps` | the owner; the `runtime_apps` feature |
//! | PUT | `/api/v1/client/container-apps/{id}` | the owner |
//!
//! The catalog is empty until an administrator adds images, each pinned by digest. A client
//! chooses one and gives a name, a port, a memory limit, an environment and a domain; nothing
//! else. There is no way to ask for a privileged container, a mount of the host, extra
//! capabilities or another network: such a field in a request is refused as unknown, and the
//! node builds every container the same hardened way. The container is an application
//! (`runtime_apps` of kind `container`), so it is listed, redeployed and removed with the others
//! and shown on a domain the same way.

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    container_images::{self as images, Image, Removed},
    runtime_apps as store,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require,
    permissions::{AppsManage, AppsRead},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{AppsState, bad, internal};
use crate::runtime::{
    DEFAULT_MAX_APPS, DEFAULT_MEMORY_MB, answer, check_env, feature, limit_of, load_in, queue_set,
    seal, valid_name, view_all, write_error,
};

/// The administrator's routes.
pub fn admin_router() -> Router<AppsState> {
    Router::new()
        .route(
            "/api/v1/admin/container-images",
            get(admin_list).post(admin_add),
        )
        .route(
            "/api/v1/admin/container-images/{id}",
            put(admin_change).delete(admin_remove),
        )
}

/// The client's routes.
pub fn client_router() -> Router<AppsState> {
    Router::new()
        .route("/api/v1/client/container-images", get(client_images))
        .route(
            "/api/v1/client/container-apps",
            get(client_list).post(client_create),
        )
        .route("/api/v1/client/container-apps/{id}", put(client_update))
}

// ----------------------------------------------------------------------- catalog

/// An approved image.
#[derive(Debug, Serialize, ToSchema)]
pub struct ImageView {
    /// Id.
    pub id: Uuid,
    /// What clients see.
    pub name: String,
    /// `repo@sha256:<digest>`; shown to administrators only.
    pub reference: Option<String>,
    /// The port it listens on.
    pub default_port: i32,
    /// The memory it starts with (MiB).
    pub default_memory_mb: i32,
    /// A note for clients.
    pub note: String,
    /// Whether clients may choose it.
    pub enabled: bool,
}

fn image_view(i: Image, admin: bool) -> ImageView {
    ImageView {
        id: i.id,
        name: i.name,
        reference: admin.then_some(i.reference),
        default_port: i.default_port,
        default_memory_mb: i.default_memory_mb,
        note: i.note,
        enabled: i.enabled,
    }
}

/// An image to approve, or the changes to one.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetImage {
    /// What clients see (1-61 characters: letters, digits, space, `.`, `_`, `-`).
    pub name: String,
    /// `repo@sha256:<digest>`: pinned by digest. Not changeable: add another image instead.
    pub reference: Option<String>,
    /// The port it listens on (1024-65535).
    pub default_port: i32,
    /// The memory it starts with, MiB (32-4096; default 256).
    pub default_memory_mb: Option<i32>,
    /// A note for clients (at most 500 characters).
    pub note: Option<String>,
    /// Whether clients may choose it (default yes).
    pub enabled: Option<bool>,
}

fn pinned(reference: &str) -> bool {
    let Some((name, digest)) = reference.split_once("@sha256:") else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 200
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && !name.contains("..")
        && !name.contains("//")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._/:-".contains(&b))
        && digest.len() == 64
        && digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn check_image(req: &SetImage, creating: bool) -> Result<(), AppError> {
    let mut errors = Vec::new();
    let name = req.name.trim();
    if name.is_empty()
        || name.len() > 61
        || !name.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" ._-".contains(&b))
    {
        errors.push(FieldError::new(
            "name",
            "invalid_name",
            "Use 1 to 61 letters, digits, spaces and . _ -",
        ));
    }
    match (&req.reference, creating) {
        (Some(r), _) if !pinned(r) => errors.push(FieldError::new(
            "reference",
            "not_pinned",
            "Use repo@sha256:<64 hex digits>; a tag can move.",
        )),
        (None, true) => errors.push(FieldError::new(
            "reference",
            "required",
            "The image is needed.",
        )),
        (Some(_), false) => errors.push(FieldError::new(
            "reference",
            "not_changeable",
            "An image's reference does not change: add another image.",
        )),
        _ => {}
    }
    if !(1024..=65_535).contains(&req.default_port) {
        errors.push(FieldError::new(
            "default_port",
            "out_of_range",
            "1024 to 65535.",
        ));
    }
    if !(32..=4096).contains(&req.default_memory_mb.unwrap_or(256)) {
        errors.push(FieldError::new(
            "default_memory_mb",
            "out_of_range",
            "32 to 4096 MiB.",
        ));
    }
    if req.note.as_deref().is_some_and(|n| n.chars().count() > 500) {
        errors.push(FieldError::new(
            "note",
            "too_long",
            "At most 500 characters.",
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::validation(errors))
    }
}

pub(crate) fn audit(action: &'static str, who: &Principal, id: Uuid) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target("container_image", id)
}

/// The catalog, with each image's reference.
#[utoipa::path(get, path = "/api/v1/admin/container-images", tag = "apps",
    operation_id = "adminListContainerImages", security(("session" = [])),
    responses((status = 200, description = "The catalog", body = Vec<ImageView>),
              (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_list(
    State(state): State<AppsState>,
    _who: Require<AppsRead>,
) -> Result<Json<Vec<ImageView>>, AppError> {
    let rows = images::list(&state.db, false).await.map_err(internal)?;
    Ok(Json(
        rows.into_iter().map(|i| image_view(i, true)).collect(),
    ))
}

/// Approve an image.
#[utoipa::path(post, path = "/api/v1/admin/container-images", tag = "apps",
    operation_id = "adminAddContainerImage", security(("session" = [])),
    request_body = SetImage,
    responses((status = 201, description = "Added", body = ImageView),
              (status = 409, description = "`image_taken`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Invalid input", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_add(
    State(state): State<AppsState>,
    Require(who, _): Require<AppsManage>,
    Json(req): Json<SetImage>,
) -> Result<(StatusCode, Json<ImageView>), AppError> {
    check_image(&req, true)?;
    let made = images::insert(
        &state.db,
        &images::Write {
            name: req.name.trim(),
            reference: req.reference.as_deref().unwrap_or_default(),
            default_port: req.default_port,
            default_memory_mb: req.default_memory_mb.unwrap_or(256),
            note: req.note.as_deref().unwrap_or_default(),
            enabled: req.enabled.unwrap_or(true),
        },
    )
    .await
    .map_err(|e| match e {
        images::WriteError::Taken => AppError::conflict(
            "image_taken",
            "An image with this name, or this image, is in the catalog already.",
        ),
        images::WriteError::Database(db) => internal(db),
    })?;
    builtpanel_audit::insert(&state.db, &audit("container_image.added", &who, made.id))
        .await
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(image_view(made, true))))
}

/// Change what clients see of an image, or stop offering it (running containers keep running).
#[utoipa::path(put, path = "/api/v1/admin/container-images/{id}", tag = "apps",
    operation_id = "adminChangeContainerImage", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Image id")),
    request_body = SetImage,
    responses((status = 200, description = "Changed", body = ImageView),
              (status = 404, description = "No such image", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`image_taken`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Invalid input", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_change(
    State(state): State<AppsState>,
    Require(who, _): Require<AppsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SetImage>,
) -> Result<Json<ImageView>, AppError> {
    check_image(&req, false)?;
    let found = images::update(
        &state.db,
        id,
        &images::Write {
            name: req.name.trim(),
            reference: "",
            default_port: req.default_port,
            default_memory_mb: req.default_memory_mb.unwrap_or(256),
            note: req.note.as_deref().unwrap_or_default(),
            enabled: req.enabled.unwrap_or(true),
        },
    )
    .await
    .map_err(|e| match e {
        images::WriteError::Taken => AppError::conflict(
            "image_taken",
            "An image with this name is in the catalog already.",
        ),
        images::WriteError::Database(db) => internal(db),
    })?;
    if !found {
        return Err(AppError::not_found("image"));
    }
    builtpanel_audit::insert(&state.db, &audit("container_image.changed", &who, id))
        .await
        .map_err(internal)?;
    let image = images::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("image"))?;
    Ok(Json(image_view(image, true)))
}

/// Remove an image no container runs.
#[utoipa::path(delete, path = "/api/v1/admin/container-images/{id}", tag = "apps",
    operation_id = "adminRemoveContainerImage", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Image id")),
    responses((status = 204, description = "Removed"),
              (status = 404, description = "No such image", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`image_in_use`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_remove(
    State(state): State<AppsState>,
    Require(who, _): Require<AppsManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    match images::delete(&state.db, id).await.map_err(internal)? {
        Removed::NoSuchImage => Err(AppError::not_found("image")),
        Removed::InUse => Err(AppError::conflict(
            "image_in_use",
            "A container runs this image: stop offering it instead.",
        )),
        Removed::Yes => {
            builtpanel_audit::insert(&state.db, &audit("container_image.removed", &who, id))
                .await
                .map_err(internal)?;
            Ok(StatusCode::NO_CONTENT)
        }
    }
}

/// The images a client may run.
#[utoipa::path(get, path = "/api/v1/client/container-images", tag = "apps",
    operation_id = "clientListContainerImages", security(("session" = [])),
    responses((status = 200, description = "The approved images", body = Vec<ImageView>),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_images(
    State(state): State<AppsState>,
    who: Principal,
) -> Result<Json<Vec<ImageView>>, AppError> {
    feature(&state, &who).await?;
    let rows = images::list(&state.db, true).await.map_err(internal)?;
    Ok(Json(
        rows.into_iter().map(|i| image_view(i, false)).collect(),
    ))
}

// -------------------------------------------------------------------- containers

/// A container of the account.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContainerView {
    /// Id (the application's).
    pub id: Uuid,
    /// Short name.
    pub name: String,
    /// The image's id.
    pub image_id: Option<Uuid>,
    /// The image's name.
    pub image: Option<String>,
    /// The port it listens on.
    pub port: i32,
    /// Memory limit, MiB.
    pub memory_mb: i32,
    /// The names of its environment variables (the values are never shown).
    pub env_names: Vec<String>,
    /// The domain that shows it.
    pub domain_id: Option<Uuid>,
    /// That domain's name.
    pub domain: Option<String>,
    /// `pending`, `ready`, `failed` or `removing`.
    pub status: String,
    /// Why it failed.
    pub error: Option<String>,
}

async fn container_views(
    state: &AppsState,
    rows: Vec<store::RuntimeApp>,
) -> Result<Vec<ContainerView>, AppError> {
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let links: BTreeMap<Uuid, Uuid> = images::of_apps(&state.db, &ids)
        .await
        .map_err(internal)?
        .into_iter()
        .collect();
    let mut names = BTreeMap::new();
    for image in images::list(&state.db, false).await.map_err(internal)? {
        names.insert(image.id, image.name);
    }
    let views = view_all(state, rows).await?;
    Ok(views
        .into_iter()
        .map(|v| {
            let image_id = links.get(&v.id).copied();
            ContainerView {
                image: image_id.and_then(|i| names.get(&i).cloned()),
                image_id,
                id: v.id,
                name: v.name,
                port: v.port,
                memory_mb: v.memory_mb,
                env_names: v.env_names,
                domain_id: v.domain_id,
                domain: v.domain,
                status: v.status,
                error: v.error,
            }
        })
        .collect())
}

/// The account's containers.
#[utoipa::path(get, path = "/api/v1/client/container-apps", tag = "apps",
    operation_id = "clientListContainerApps", security(("session" = [])),
    responses((status = 200, description = "The containers", body = Vec<ContainerView>),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_list(
    State(state): State<AppsState>,
    who: Principal,
) -> Result<Json<Vec<ContainerView>>, AppError> {
    let (tenant, _) = feature(&state, &who).await?;
    let rows: Vec<_> = store::list(&state.db, tenant.id)
        .await
        .map_err(internal)?
        .into_iter()
        .filter(|r| r.kind == "container")
        .collect();
    Ok(Json(container_views(&state, rows).await?))
}

/// A container to run. Nothing here lets a client widen what the container may do: there is no
/// field for it, and an unknown field is refused.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateContainer {
    /// Short name (2-32 of a-z, 0-9, `-`).
    pub name: String,
    /// The approved image (see `/container-images`).
    pub image_id: Uuid,
    /// The port it listens on; the image's own when left out.
    pub port: Option<i32>,
    /// Memory limit in MiB; the image's own when left out.
    pub memory_mb: Option<i32>,
    /// The environment, name to value.
    pub env: Option<BTreeMap<String, String>>,
    /// The domain that shows it (one of the account's; one application each).
    pub domain_id: Option<Uuid>,
}

/// Changes to a container (it is run again with them).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeContainer {
    /// Another approved image.
    pub image_id: Option<Uuid>,
    /// The port it listens on.
    pub port: Option<i32>,
    /// Memory limit in MiB.
    pub memory_mb: Option<i32>,
    /// The environment; when given it replaces the whole environment.
    pub env: Option<BTreeMap<String, String>>,
    /// The domain that shows it.
    pub domain_id: Option<Uuid>,
}

/// Run a container from an approved image.
#[utoipa::path(post, path = "/api/v1/client/container-apps", tag = "apps",
    operation_id = "clientCreateContainerApp", security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = CreateContainer,
    responses((status = 202, description = "Queued", body = ContainerView),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`limit_reached`, `app_taken` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Invalid input, or a field that is not allowed (for example `privileged` or a mount)", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_create(
    State(state): State<AppsState>,
    who: Principal,
    Json(req): Json<CreateContainer>,
) -> Result<(StatusCode, Json<ContainerView>), AppError> {
    let (tenant, plan) = feature(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can change its applications.",
        ));
    }
    if !valid_name(&req.name) {
        return Err(bad(
            "name",
            "2 to 32 of a-z, 0-9 and -, starting with a letter.",
        ));
    }
    if let Some(env) = &req.env {
        check_env(env)?;
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    let image = images::get_in(&mut tx, req.image_id)
        .await
        .map_err(internal)?
        .filter(|i| i.enabled)
        .ok_or_else(|| bad("image_id", "This image is not offered."))?;
    let port = req.port.unwrap_or(image.default_port);
    let memory_mb = req.memory_mb.unwrap_or(image.default_memory_mb);
    if !(1024..=65_535).contains(&port) {
        return Err(bad("port", "1024 to 65535."));
    }
    if !(32..=4096).contains(&memory_mb) {
        return Err(bad("memory_mb", "32 to 4096 MiB."));
    }
    builtpanel_database::domains::lock_tenant(&mut tx, tenant.id)
        .await
        .map_err(internal)?;
    if let Some(limit) = limit_of(&plan, "max_runtime_apps", DEFAULT_MAX_APPS)
        && store::count(&mut tx, tenant.id).await.map_err(internal)? >= limit
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("This plan allows {limit} application(s)."),
        ));
    }
    if let Some(limit) = limit_of(&plan, "runtime_memory_mb", DEFAULT_MEMORY_MB)
        && store::memory_used(&mut tx, tenant.id, None)
            .await
            .map_err(internal)?
            + i64::from(memory_mb)
            > limit
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("This plan allows {limit} MiB of memory for applications in all."),
        ));
    }
    let env_names: Vec<String> = req
        .env
        .as_ref()
        .map_or_else(Vec::new, |e| e.keys().cloned().collect());
    let id = store::insert(
        &mut tx,
        tenant.id,
        &store::Write {
            name: &req.name,
            kind: "container",
            version: "0",
            dir: "",
            start_command: "-",
            install: false,
            port,
            memory_mb,
            domain_id: req.domain_id,
            env_names: &env_names,
        },
    )
    .await
    .map_err(write_error)?;
    images::set_for_app(&mut tx, id, image.id)
        .await
        .map_err(internal)?;
    let sealed = seal(&state, id, &req.env.clone().unwrap_or_default())?;
    store::set_env(&mut tx, id, &sealed)
        .await
        .map_err(internal)?;
    let row = load_in(&mut tx, tenant.id, id).await?;
    queue_set(&mut tx, &who, &tenant, &row, false).await?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("container_app.created", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("runtime_app", id)
            .metadata(serde_json::json!({ "name": req.name, "image": image.name })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let (status, _) = answer(&state, &tenant, id, StatusCode::ACCEPTED).await?;
    let row = store::get(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("application"))?;
    Ok((
        status,
        Json(container_views(&state, vec![row]).await?.remove(0)),
    ))
}

/// Change a container and run it again with the changes.
#[utoipa::path(put, path = "/api/v1/client/container-apps/{id}", tag = "apps",
    operation_id = "clientChangeContainerApp", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Container id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ChangeContainer,
    responses((status = 202, description = "Queued", body = ContainerView),
              (status = 404, description = "No such container", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`limit_reached`, `app_taken` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Invalid input", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_update(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangeContainer>,
) -> Result<(StatusCode, Json<ContainerView>), AppError> {
    let (tenant, plan) = feature(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can change its applications.",
        ));
    }
    let existing = store::get(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .filter(|a| a.status != "removing" && a.kind == "container")
        .ok_or_else(|| AppError::not_found("container"))?;
    if let Some(env) = &req.env {
        check_env(env)?;
    }
    let port = req.port.unwrap_or(existing.port);
    let memory_mb = req.memory_mb.unwrap_or(existing.memory_mb);
    if !(1024..=65_535).contains(&port) {
        return Err(bad("port", "1024 to 65535."));
    }
    if !(32..=4096).contains(&memory_mb) {
        return Err(bad("memory_mb", "32 to 4096 MiB."));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, tenant.id)
        .await
        .map_err(internal)?;
    if let Some(image_id) = req.image_id {
        let image = images::get_in(&mut tx, image_id)
            .await
            .map_err(internal)?
            .filter(|i| i.enabled)
            .ok_or_else(|| bad("image_id", "This image is not offered."))?;
        images::set_for_app(&mut tx, id, image.id)
            .await
            .map_err(internal)?;
    }
    if let Some(limit) = limit_of(&plan, "runtime_memory_mb", DEFAULT_MEMORY_MB)
        && store::memory_used(&mut tx, tenant.id, Some(id))
            .await
            .map_err(internal)?
            + i64::from(memory_mb)
            > limit
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("This plan allows {limit} MiB of memory for applications in all."),
        ));
    }
    let env_names: Vec<String> = match &req.env {
        Some(e) => e.keys().cloned().collect(),
        None => existing.env_names.clone(),
    };
    store::update(
        &mut tx,
        tenant.id,
        id,
        &store::Write {
            name: &existing.name,
            kind: "container",
            version: "0",
            dir: "",
            start_command: "-",
            install: false,
            port,
            memory_mb,
            domain_id: req.domain_id.or(existing.domain_id),
            env_names: &env_names,
        },
    )
    .await
    .map_err(write_error)?;
    if let Some(env) = &req.env {
        let sealed = seal(&state, id, env)?;
        store::set_env(&mut tx, id, &sealed)
            .await
            .map_err(internal)?;
    }
    let row = load_in(&mut tx, tenant.id, id).await?;
    queue_set(&mut tx, &who, &tenant, &row, false).await?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("container_app.updated", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("runtime_app", id),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let row = store::get(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("container"))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(container_views(&state, vec![row]).await?.remove(0)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_digest_pins_an_image() {
        let d = "a".repeat(64);
        assert!(pinned(&format!("nginxinc/nginx-unprivileged@sha256:{d}")));
        assert!(pinned(&format!("ghcr.io/acme/app@sha256:{d}")));
        assert!(!pinned("nginx:latest"));
        assert!(
            !pinned(&format!("nginx@sha256:{}", "A".repeat(64))),
            "lower case only"
        );
        assert!(!pinned(&format!("nginx@sha256:{}", "a".repeat(63))));
        assert!(!pinned(&format!("../x@sha256:{d}")));
        assert!(!pinned(&format!("a//b@sha256:{d}")));
        assert!(!pinned(&format!("@sha256:{d}")));
    }
}
