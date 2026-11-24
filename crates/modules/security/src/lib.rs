//! The web application firewall's policies (T11.02).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, PUT, DELETE | `/api/v1/admin/waf/policy` | `admin.security.read`, `.manage` |
//! | GET, PUT, DELETE | `/api/v1/admin/waf/tenants/{id}` | the same |
//! | GET, PUT, DELETE | `/api/v1/admin/waf/domains/{id}` | the same |
//! | GET, POST | `/api/v1/admin/security/user-agents` | `admin.security.read`, `.manage` |
//! | DELETE | `/api/v1/admin/security/user-agents/{id}` | `admin.security.manage` |
//! | GET, PUT, DELETE | `/api/v1/admin/nodes/{id}/firewall` | `admin.security.read`, `.manage` |
//! | GET, POST | `/api/v1/admin/tenants/{id}/malware-scans` | `admin.security.read`, `.manage` |
//! | GET, POST | `/api/v1/client/malware-scans` | a client |
//! | GET, POST | `/api/v1/admin/tenants/{id}/vulnerability-scans` | `admin.security.read`, `.manage` |
//! | GET, POST | `/api/v1/client/vulnerability-scans` | a client |
//! | GET | `/api/v1/admin/security/images`, `/images/vulnerabilities?reference=` | `admin.security.read` |
//! | POST | `/api/v1/admin/security/images/scan` | `admin.security.manage` |
//! | GET, POST, DELETE | `/api/v1/admin/security/wp-advisories[/{id}]` | `admin.security.read`, `.manage` |
//! | GET | `/api/v1/client/waf/domains` | a client |
//! | PUT, DELETE | `/api/v1/client/waf/domains/{id}` | the domain's owner |
//! | GET, POST | `/api/v1/client/waf/domains/{id}/ip-blocks` | the domain's owner |
//! | DELETE | `/api/v1/client/waf/domains/{id}/ip-blocks/{block_id}` | the domain's owner |
//!
//! A policy is a mode (`off`, `detect`, `block`) and a list of rule ids that do not apply (the
//! way out of a false positive). The most specific policy that exists decides the mode (domain,
//! then account, then global; `detect` when there is none); the rules switched off are those of
//! all three together. A client sets the policy of their own domains, in `detect` or `block`
//! only (`off` is for administrators). Every change makes the account's routes be sent to its
//! node again (the global one, every account's), where the firewall is configured from them.

use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    routing::get,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    accounts::DbAccountKind,
    domains, ip_blocks,
    sqlx::PgPool,
    tenants, users,
    waf::{self, Mode, Policy, Scope},
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require,
    permissions::{SecurityManage, SecurityRead},
};
use builtpanel_session::SessionStore;
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/security/module.toml");

/// What the module's handlers share.
#[derive(Clone)]
pub struct SecurityState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
}

impl FromRef<SecurityState> for PgPool {
    fn from_ref(s: &SecurityState) -> Self {
        s.db.clone()
    }
}

impl FromRef<SecurityState> for SessionStore {
    fn from_ref(s: &SecurityState) -> Self {
        s.sessions.clone()
    }
}

/// Routes for administrators.
pub fn admin_router() -> Router<SecurityState> {
    Router::new()
        .route(
            "/api/v1/admin/waf/policy",
            get(admin_get_global)
                .put(admin_put_global)
                .delete(admin_delete_global),
        )
        .route(
            "/api/v1/admin/waf/tenants/{id}",
            get(admin_get_tenant)
                .put(admin_put_tenant)
                .delete(admin_delete_tenant),
        )
        .route(
            "/api/v1/admin/security/user-agents",
            get(list_user_agents).post(add_user_agent),
        )
        .route(
            "/api/v1/admin/security/user-agents/{id}",
            axum::routing::delete(remove_user_agent),
        )
        .route(
            "/api/v1/admin/nodes/{id}/firewall",
            get(get_node_firewall)
                .put(put_node_firewall)
                .delete(reset_node_firewall),
        )
        .route(
            "/api/v1/admin/tenants/{id}/malware-scans",
            get(admin_list_scans).post(admin_start_scan),
        )
        .route(
            "/api/v1/admin/tenants/{id}/vulnerability-scans",
            get(admin_list_vuln).post(admin_start_vuln),
        )
        .route("/api/v1/admin/security/images", get(list_images))
        .route(
            "/api/v1/admin/security/images/vulnerabilities",
            get(image_vulnerabilities),
        )
        .route(
            "/api/v1/admin/security/images/scan",
            axum::routing::post(scan_image_now),
        )
        .route(
            "/api/v1/admin/security/wp-advisories",
            get(list_advisories).post(add_advisory),
        )
        .route(
            "/api/v1/admin/security/wp-advisories/{id}",
            axum::routing::delete(remove_advisory),
        )
        .route(
            "/api/v1/admin/waf/domains/{id}",
            get(admin_get_domain)
                .put(admin_put_domain)
                .delete(admin_delete_domain),
        )
}

/// Routes for hosting accounts.
pub fn client_router() -> Router<SecurityState> {
    Router::new()
        .route("/api/v1/client/waf/domains", get(client_list))
        .route(
            "/api/v1/client/vulnerability-scans",
            get(client_list_vuln).post(client_start_vuln),
        )
        .route(
            "/api/v1/client/malware-scans",
            get(client_list_scans).post(client_start_scan),
        )
        .route(
            "/api/v1/client/waf/domains/{id}",
            axum::routing::put(client_put).delete(client_delete),
        )
        .route(
            "/api/v1/client/waf/domains/{id}/ip-blocks",
            get(client_list_ip_blocks).post(client_add_ip_block),
        )
        .route(
            "/api/v1/client/waf/domains/{id}/ip-blocks/{block_id}",
            axum::routing::delete(client_remove_ip_block),
        )
}

fn field_error(field: &str, code: &str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(field, code, message)])
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

/// A policy as written.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyRequest {
    /// `off`, `detect` or `block`.
    pub mode: String,
    /// Rule ids that do not apply (such as `942100`), at most 200.
    #[serde(default)]
    pub disabled_rules: Vec<i32>,
}

/// A policy as stored.
#[derive(Debug, Serialize, ToSchema)]
pub struct WafPolicyView {
    /// `off`, `detect` or `block`.
    pub mode: String,
    /// Rule ids that do not apply.
    pub disabled_rules: Vec<i32>,
}

/// What applies in the end.
#[derive(Debug, Serialize, ToSchema)]
pub struct EffectiveView {
    /// `off`, `detect` or `block`.
    pub mode: String,
    /// Which policy decided the mode: `domain`, `tenant`, `global` or `default`.
    pub mode_from: String,
    /// The rules switched off, all policies together.
    pub disabled_rules: Vec<i32>,
}

/// A policy and what applies.
#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyState {
    /// The policy set exactly here (`null`: none).
    pub policy: Option<WafPolicyView>,
    /// What really applies (for a domain; for the others the policy or the default).
    pub effective: EffectiveView,
}

fn view(p: &Policy) -> WafPolicyView {
    WafPolicyView {
        mode: p.mode.as_str().to_owned(),
        disabled_rules: p.disabled_rules.clone(),
    }
}

fn effective_view(e: waf::Effective) -> EffectiveView {
    EffectiveView {
        mode: e.mode.as_str().to_owned(),
        mode_from: e.mode_from.to_owned(),
        disabled_rules: e.disabled_rules,
    }
}

fn check(req: &PolicyRequest, client: bool) -> Result<(Mode, Vec<i32>), AppError> {
    let bad = |name: &'static str, code: &'static str, message: &str| {
        AppError::validation(vec![FieldError::new(name, code, message)])
    };
    let mode =
        Mode::parse(&req.mode).ok_or_else(|| bad("mode", "invalid", "off, detect or block."))?;
    if client && mode == Mode::Off {
        return Err(bad(
            "mode",
            "not_allowed",
            "Only an administrator can switch it off.",
        ));
    }
    let limit = if client { 50 } else { 200 };
    if req.disabled_rules.len() > limit {
        return Err(bad(
            "disabled_rules",
            "too_many",
            &format!("At most {limit} rules."),
        ));
    }
    if req
        .disabled_rules
        .iter()
        .any(|r| !(1..=999_999).contains(r))
    {
        return Err(bad(
            "disabled_rules",
            "invalid",
            "A rule id is a number from 1 to 999999.",
        ));
    }
    Ok((mode, req.disabled_rules.clone()))
}

/// The routes of these accounts are to be sent to their nodes again.
async fn resend(pool: &PgPool, tenant: Option<Uuid>) -> Result<(), AppError> {
    match tenant {
        Some(t) => {
            builtpanel_database::sqlx::query(
                "UPDATE tenants SET routes_synced_version = 0 WHERE id = $1",
            )
            .bind(t)
            .execute(pool)
            .await
        }
        None => {
            builtpanel_database::sqlx::query("UPDATE tenants SET routes_synced_version = 0")
                .execute(pool)
                .await
        }
    }
    .map_err(internal)?;
    Ok(())
}

async fn record(
    state: &SecurityState,
    action: &'static str,
    who: &Principal,
    tenant: Option<Uuid>,
    target: Uuid,
    detail: serde_json::Value,
) -> Result<(), AppError> {
    let mut entry = AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target("waf_policy", target)
        .metadata(detail);
    if let Some(t) = tenant {
        entry = entry.tenant(t);
    }
    builtpanel_audit::insert(&state.db, &entry)
        .await
        .map_err(internal)
}

async fn state_of(
    state: &SecurityState,
    scope: Scope,
    domain: Option<(Uuid, Uuid)>,
) -> Result<PolicyState, AppError> {
    let policy = waf::get(&state.db, scope).await.map_err(internal)?;
    let effective = match domain {
        Some((tenant_id, domain_id)) => waf::effective(&state.db, tenant_id, domain_id)
            .await
            .map_err(internal)?,
        None => {
            let global = waf::get(&state.db, Scope::Global).await.map_err(internal)?;
            let tenant = match scope {
                Scope::Tenant(t) => waf::get(&state.db, Scope::Tenant(t))
                    .await
                    .map_err(internal)?,
                _ => None,
            };
            waf::combine(global.as_ref(), tenant.as_ref(), None)
        }
    };
    Ok(PolicyState {
        policy: policy.as_ref().map(view),
        effective: effective_view(effective),
    })
}

/// Which account or domain a request is about, checked to exist.
enum Target {
    Global,
    Tenant(Uuid),
    Domain(Uuid, Uuid),
}

impl Target {
    fn scope(&self) -> Scope {
        match *self {
            Self::Global => Scope::Global,
            Self::Tenant(t) => Scope::Tenant(t),
            Self::Domain(tenant_id, domain_id) => Scope::Domain {
                tenant_id,
                domain_id,
            },
        }
    }

    fn tenant(&self) -> Option<Uuid> {
        match *self {
            Self::Global => None,
            Self::Tenant(t) | Self::Domain(t, _) => Some(t),
        }
    }

    fn id(&self) -> Uuid {
        match *self {
            Self::Global => Uuid::nil(),
            Self::Tenant(t) => t,
            Self::Domain(_, d) => d,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Tenant(_) => "tenant",
            Self::Domain(..) => "domain",
        }
    }
}

async fn tenant_target(state: &SecurityState, id: Uuid) -> Result<Target, AppError> {
    tenants::get(&state.db, id)
        .await
        .map_err(internal)?
        .map(|t| Target::Tenant(t.id))
        .ok_or_else(|| AppError::not_found("tenant"))
}

async fn domain_target(state: &SecurityState, id: Uuid) -> Result<Target, AppError> {
    domains::get(&state.db, id)
        .await
        .map_err(internal)?
        .map(|d| Target::Domain(d.tenant_id, d.id))
        .ok_or_else(|| AppError::not_found("domain"))
}

async fn show(state: &SecurityState, target: &Target) -> Result<Json<PolicyState>, AppError> {
    let domain = match *target {
        Target::Domain(t, d) => Some((t, d)),
        _ => None,
    };
    Ok(Json(state_of(state, target.scope(), domain).await?))
}

async fn put_policy(
    state: &SecurityState,
    who: &Principal,
    target: &Target,
    req: &PolicyRequest,
    client: bool,
) -> Result<Json<PolicyState>, AppError> {
    let (mode, rules) = check(req, client)?;
    waf::set(
        &state.db,
        target.scope(),
        mode,
        &rules,
        Some(who.account_id()),
    )
    .await
    .map_err(internal)?;
    resend(&state.db, target.tenant()).await?;
    record(
        state,
        "waf_policy.set",
        who,
        target.tenant(),
        target.id(),
        serde_json::json!({ "scope": target.label(), "mode": mode.as_str(), "disabled_rules": rules }),
    )
    .await?;
    show(state, target).await
}

async fn delete_policy(
    state: &SecurityState,
    who: &Principal,
    target: &Target,
) -> Result<StatusCode, AppError> {
    if !waf::clear(&state.db, target.scope())
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("waf_policy"));
    }
    resend(&state.db, target.tenant()).await?;
    record(
        state,
        "waf_policy.removed",
        who,
        target.tenant(),
        target.id(),
        serde_json::json!({ "scope": target.label() }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The panel-wide policy.
#[utoipa::path(get, path = "/api/v1/admin/waf/policy", tag = "security", operation_id = "adminGetWafPolicy",
    security(("session" = [])),
    responses((status = 200, description = "The policy and what applies", body = PolicyState),
              (status = 403, description = "`permission_denied` (needs admin.security.read)", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_get_global(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
) -> Result<Json<PolicyState>, AppError> {
    show(&state, &Target::Global).await
}

/// Set the panel-wide policy.
#[utoipa::path(put, path = "/api/v1/admin/waf/policy", tag = "security", operation_id = "adminPutWafPolicy",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = PolicyRequest,
    responses((status = 200, description = "Set", body = PolicyState),
              (status = 422, description = "A mode or rule that is not usable", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_put_global(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Json(req): Json<PolicyRequest>,
) -> Result<Json<PolicyState>, AppError> {
    put_policy(&state, &who.0, &Target::Global, &req, false).await
}

/// Remove the panel-wide policy (the default, `detect`, applies).
#[utoipa::path(delete, path = "/api/v1/admin/waf/policy", tag = "security", operation_id = "adminDeleteWafPolicy",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 204, description = "Removed"),
              (status = 404, description = "There is none", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_delete_global(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
) -> Result<StatusCode, AppError> {
    delete_policy(&state, &who.0, &Target::Global).await
}

/// One account's policy.
#[utoipa::path(get, path = "/api/v1/admin/waf/tenants/{id}", tag = "security", operation_id = "adminGetTenantWafPolicy",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Account id")),
    responses((status = 200, description = "The policy and what applies", body = PolicyState),
              (status = 404, description = "No such account", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_get_tenant(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<PolicyState>, AppError> {
    let target = tenant_target(&state, id).await?;
    show(&state, &target).await
}

/// Set one account's policy.
#[utoipa::path(put, path = "/api/v1/admin/waf/tenants/{id}", tag = "security", operation_id = "adminPutTenantWafPolicy",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Account id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = PolicyRequest,
    responses((status = 200, description = "Set", body = PolicyState),
              (status = 404, description = "No such account", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A mode or rule that is not usable", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_put_tenant(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<PolicyRequest>,
) -> Result<Json<PolicyState>, AppError> {
    let target = tenant_target(&state, id).await?;
    put_policy(&state, &who.0, &target, &req, false).await
}

/// Remove one account's policy.
#[utoipa::path(delete, path = "/api/v1/admin/waf/tenants/{id}", tag = "security", operation_id = "adminDeleteTenantWafPolicy",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Account id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 204, description = "Removed"),
              (status = 404, description = "No such account or no policy", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_delete_tenant(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let target = tenant_target(&state, id).await?;
    delete_policy(&state, &who.0, &target).await
}

/// One domain's policy.
#[utoipa::path(get, path = "/api/v1/admin/waf/domains/{id}", tag = "security", operation_id = "adminGetDomainWafPolicy",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Domain id")),
    responses((status = 200, description = "The policy and what applies", body = PolicyState),
              (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_get_domain(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<PolicyState>, AppError> {
    let target = domain_target(&state, id).await?;
    show(&state, &target).await
}

/// Set one domain's policy.
#[utoipa::path(put, path = "/api/v1/admin/waf/domains/{id}", tag = "security", operation_id = "adminPutDomainWafPolicy",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Domain id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = PolicyRequest,
    responses((status = 200, description = "Set", body = PolicyState),
              (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A mode or rule that is not usable", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_put_domain(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<PolicyRequest>,
) -> Result<Json<PolicyState>, AppError> {
    let target = domain_target(&state, id).await?;
    put_policy(&state, &who.0, &target, &req, false).await
}

/// Remove one domain's policy.
#[utoipa::path(delete, path = "/api/v1/admin/waf/domains/{id}", tag = "security", operation_id = "adminDeleteDomainWafPolicy",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Domain id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 204, description = "Removed"),
              (status = 404, description = "No such domain or no policy", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_delete_domain(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let target = domain_target(&state, id).await?;
    delete_policy(&state, &who.0, &target).await
}

async fn client_tenant(
    state: &SecurityState,
    who: &Principal,
) -> Result<tenants::Tenant, AppError> {
    if who.kind != DbAccountKind::Client {
        return Err(AppError::forbidden(
            "client_only",
            "This is for hosting accounts.",
        ));
    }
    let user = users::get(&state.db, who.account_id())
        .await
        .map_err(internal)?;
    let owned = user.and_then(|u| u.tenant).ok_or_else(|| {
        AppError::conflict("no_hosting", "This account has no hosting account yet.")
    })?;
    tenants::get(&state.db, owned.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))
}

/// One of the account's own domains, or `404`.
async fn own_domain(
    state: &SecurityState,
    tenant: &tenants::Tenant,
    id: Uuid,
) -> Result<Target, AppError> {
    domains::get(&state.db, id)
        .await
        .map_err(internal)?
        .filter(|d| d.tenant_id == tenant.id)
        .map(|d| Target::Domain(tenant.id, d.id))
        .ok_or_else(|| AppError::not_found("domain"))
}

/// A domain with its firewall.
#[derive(Debug, Serialize, ToSchema)]
pub struct DomainWaf {
    /// The domain's id.
    pub domain_id: Uuid,
    /// Its name.
    pub fqdn: String,
    /// The domain's own policy (`null`: none).
    pub policy: Option<WafPolicyView>,
    /// What really applies.
    pub effective: EffectiveView,
}

/// The domains of the account.
#[derive(Debug, Serialize, ToSchema)]
pub struct DomainWafList {
    /// By name.
    pub items: Vec<DomainWaf>,
}

/// The account's domains with their firewall policy and what applies to each.
#[utoipa::path(get, path = "/api/v1/client/waf/domains", tag = "security", operation_id = "clientListWaf",
    security(("session" = [])),
    responses((status = 200, description = "The domains", body = DomainWafList),
              (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_list(
    State(state): State<SecurityState>,
    who: Principal,
) -> Result<Json<DomainWafList>, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    let rows = builtpanel_database::sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, fqdn FROM domains WHERE tenant_id = $1 ORDER BY fqdn",
    )
    .bind(tenant.id)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;
    let mut items = Vec::new();
    for (domain_id, fqdn) in rows {
        let policy = waf::get(
            &state.db,
            Scope::Domain {
                tenant_id: tenant.id,
                domain_id,
            },
        )
        .await
        .map_err(internal)?;
        let effective = waf::effective(&state.db, tenant.id, domain_id)
            .await
            .map_err(internal)?;
        items.push(DomainWaf {
            domain_id,
            fqdn,
            policy: policy.as_ref().map(view),
            effective: effective_view(effective),
        });
    }
    Ok(Json(DomainWafList { items }))
}

/// Set the policy of one of the account's domains (`detect` or `block`).
#[utoipa::path(put, path = "/api/v1/client/waf/domains/{id}", tag = "security", operation_id = "clientPutWaf",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Domain id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = PolicyRequest,
    responses((status = 200, description = "Set", body = PolicyState),
              (status = 404, description = "Not one of the account's domains", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "`off`, a mode or a rule that is not usable", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_put(
    State(state): State<SecurityState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<PolicyRequest>,
) -> Result<Json<PolicyState>, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    let target = own_domain(&state, &tenant, id).await?;
    put_policy(&state, &who, &target, &req, true).await
}

/// Remove the policy of one of the account's domains (the account's or the panel's applies).
#[utoipa::path(delete, path = "/api/v1/client/waf/domains/{id}", tag = "security", operation_id = "clientDeleteWaf",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Domain id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 204, description = "Removed"),
              (status = 404, description = "Not one of the account's domains, or no policy", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_delete(
    State(state): State<SecurityState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    let target = own_domain(&state, &tenant, id).await?;
    delete_policy(&state, &who, &target).await
}

/// An address or a network a domain turns away.
#[derive(Debug, Serialize, ToSchema)]
pub struct IpBlockView {
    /// Id.
    pub id: Uuid,
    /// The address (`203.0.113.9`) or the network (`203.0.113.0/24`).
    pub cidr: String,
    /// Why, if said.
    pub note: Option<String>,
    /// When it was added (Unix seconds).
    pub created_at: i64,
}

/// A domain's turned-away addresses.
#[derive(Debug, Serialize, ToSchema)]
pub struct IpBlockList {
    /// Oldest first.
    pub items: Vec<IpBlockView>,
    /// Most one domain may list.
    pub limit: i64,
}

/// An address or a network to turn away.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddIpBlock {
    /// An IPv4 or IPv6 address, or a network in CIDR form (`203.0.113.0/24`). Networks larger
    /// than a /8 (IPv4) or a /16 (IPv6) are refused.
    #[schema(example = "203.0.113.0/24")]
    pub cidr: String,
    /// Why (at most 200 characters).
    pub note: Option<String>,
}

fn ip_block_view(b: ip_blocks::IpBlock) -> IpBlockView {
    IpBlockView {
        id: b.id,
        cidr: b.cidr,
        note: b.note,
        created_at: b.created_at,
    }
}

/// The addresses one of the account's domains turns away.
#[utoipa::path(get, path = "/api/v1/client/waf/domains/{id}/ip-blocks", tag = "security", operation_id = "clientListIpBlocks",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Domain id")),
    responses((status = 200, description = "The list", body = IpBlockList),
              (status = 404, description = "Not one of the account's domains", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_list_ip_blocks(
    State(state): State<SecurityState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<IpBlockList>, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    own_domain(&state, &tenant, id).await?;
    let items = ip_blocks::list(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(ip_block_view)
        .collect();
    Ok(Json(IpBlockList {
        items,
        limit: ip_blocks::MAX_PER_DOMAIN,
    }))
}

/// Turn an address or a network away from one of the account's domains: the web server answers
/// it `403` within seconds, before the firewall or the site sees the request.
#[utoipa::path(post, path = "/api/v1/client/waf/domains/{id}/ip-blocks", tag = "security", operation_id = "clientAddIpBlock",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Domain id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = AddIpBlock,
    responses((status = 201, description = "Added", body = IpBlockView),
              (status = 404, description = "Not one of the account's domains", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`ip_block_exists` or `ip_block_limit`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Not an address or network that can be turned away", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_add_ip_block(
    State(state): State<SecurityState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<AddIpBlock>,
) -> Result<(StatusCode, Json<IpBlockView>), AppError> {
    let tenant = client_tenant(&state, &who).await?;
    own_domain(&state, &tenant, id).await?;
    let cidr = builtpanel_node_protocol::normalize_ip_block(&req.cidr)
        .map_err(|reason| field_error("cidr", "invalid", reason))?;
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > 200 || n.chars().any(char::is_control)) {
        return Err(field_error(
            "note",
            "invalid",
            "At most 200 characters, no control characters.",
        ));
    }
    let added = ip_blocks::add(&state.db, tenant.id, id, &cidr, note, who.account_id())
        .await
        .map_err(|e| match e {
            ip_blocks::AddError::Taken => {
                AppError::conflict("ip_block_exists", "That address is already turned away.")
            }
            ip_blocks::AddError::Full => AppError::conflict(
                "ip_block_limit",
                "This domain already turns away as many addresses as it may.",
            ),
            ip_blocks::AddError::Database(e) => internal(e),
        })?;
    resend(&state.db, Some(tenant.id)).await?;
    audit_ip_block(&state, &who, tenant.id, id, "ip_block.added", &cidr).await?;
    Ok((StatusCode::CREATED, Json(ip_block_view(added))))
}

/// Stop turning an address away.
#[utoipa::path(delete, path = "/api/v1/client/waf/domains/{id}/ip-blocks/{block_id}", tag = "security", operation_id = "clientRemoveIpBlock",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Domain id"), ("block_id" = Uuid, Path, description = "Entry id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 204, description = "Removed"),
              (status = 404, description = "Not one of the account's", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_remove_ip_block(
    State(state): State<SecurityState>,
    who: Principal,
    Path((id, block_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    own_domain(&state, &tenant, id).await?;
    if !ip_blocks::remove(&state.db, tenant.id, id, block_id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("ip_block"));
    }
    resend(&state.db, Some(tenant.id)).await?;
    audit_ip_block(&state, &who, tenant.id, id, "ip_block.removed", "").await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn audit_ip_block(
    state: &SecurityState,
    who: &Principal,
    tenant: Uuid,
    domain: Uuid,
    action: &'static str,
    cidr: &str,
) -> Result<(), AppError> {
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new(action, Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant)
            .target("domain", domain)
            .metadata(serde_json::json!({ "cidr": cidr })),
    )
    .await
    .map_err(internal)?;
    Ok(())
}

/// A User-Agent text that is refused.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserAgentView {
    /// Id.
    pub id: Uuid,
    /// The text a User-Agent contains (any case).
    pub pattern: String,
    /// Why it is listed.
    pub note: Option<String>,
    /// When it was added (Unix seconds).
    pub created_at: i64,
}

/// The list.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserAgentList {
    /// Oldest first.
    pub items: Vec<UserAgentView>,
}

/// A text to refuse.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddUserAgent {
    /// The text (1-100 characters, no control characters): a request whose User-Agent
    /// contains it, in any case, is answered `403`.
    #[schema(example = "badbot")]
    pub pattern: String,
    /// Why (at most 200 characters).
    pub note: Option<String>,
}

/// The User-Agent texts the web servers refuse.
#[utoipa::path(get, path = "/api/v1/admin/security/user-agents", tag = "security", operation_id = "adminListUserAgents",
    security(("session" = [])),
    responses((status = 200, description = "The list", body = UserAgentList),
              (status = 403, description = "`permission_denied` (needs admin.security.read)", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn list_user_agents(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
) -> Result<Json<UserAgentList>, AppError> {
    let items = builtpanel_database::user_agents::list(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|b| UserAgentView {
            id: b.id,
            pattern: b.pattern,
            note: b.note,
            created_at: b.created_at,
        })
        .collect();
    Ok(Json(UserAgentList { items }))
}

/// Refuse a User-Agent text on every node (they are told within seconds).
#[utoipa::path(post, path = "/api/v1/admin/security/user-agents", tag = "security", operation_id = "adminAddUserAgent",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = AddUserAgent,
    responses((status = 201, description = "Listed", body = UserAgentView),
              (status = 409, description = "`already_listed` or `list_full`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A text that is empty, too long or has a control character", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn add_user_agent(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Json(req): Json<AddUserAgent>,
) -> Result<(StatusCode, Json<UserAgentView>), AppError> {
    let pattern = req.pattern.trim().to_owned();
    let bad = |name: &'static str, message: &str| {
        AppError::validation(vec![FieldError::new(name, "invalid", message)])
    };
    if pattern.is_empty() || pattern.chars().count() > 100 || pattern.chars().any(char::is_control)
    {
        return Err(bad(
            "pattern",
            "1 to 100 characters, no control characters.",
        ));
    }
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > 200) {
        return Err(bad("note", "At most 200 characters."));
    }
    let id = builtpanel_database::user_agents::add(&state.db, &pattern, note, who.0.account_id())
        .await
        .map_err(|e| match e {
            builtpanel_database::user_agents::AddError::Exists => {
                AppError::conflict("already_listed", "This text is listed already.")
            }
            builtpanel_database::user_agents::AddError::Full => {
                AppError::conflict("list_full", "The list holds at most 500 texts.")
            }
            builtpanel_database::user_agents::AddError::Database(db) => internal(db),
        })?;
    record(
        &state,
        "user_agent.blocked",
        &who.0,
        None,
        id,
        serde_json::json!({ "pattern": pattern }),
    )
    .await?;
    let made = builtpanel_database::user_agents::list(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|b| b.id == id)
        .ok_or_else(|| AppError::not_found("user_agent"))?;
    Ok((
        StatusCode::CREATED,
        Json(UserAgentView {
            id: made.id,
            pattern: made.pattern,
            note: made.note,
            created_at: made.created_at,
        }),
    ))
}

/// Stop refusing a User-Agent text.
#[utoipa::path(delete, path = "/api/v1/admin/security/user-agents/{id}", tag = "security", operation_id = "adminRemoveUserAgent",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 204, description = "Removed"),
              (status = 404, description = "Not listed", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn remove_user_agent(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if !builtpanel_database::user_agents::remove(&state.db, id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("user_agent"));
    }
    record(
        &state,
        "user_agent.unblocked",
        &who.0,
        None,
        id,
        serde_json::json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What to scan.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    /// Only this folder of the files (relative to `www`; `wp-content/uploads`); all when absent.
    #[serde(default)]
    pub folder: Option<String>,
    /// Move what is found to the trash (the file manager puts it back).
    #[serde(default)]
    pub quarantine: bool,
}

/// One scan.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScanView {
    /// Id.
    pub id: Uuid,
    /// The folder scanned (`null`: everything).
    pub folder: Option<String>,
    /// Infected files were moved to the trash.
    pub quarantine: bool,
    /// `admin`, `client` or `schedule`.
    pub source: String,
    /// `queued`, `running`, `done` or `failed`.
    pub state: String,
    /// When it was asked for (Unix seconds).
    pub requested_at: i64,
    /// When it ended (Unix seconds).
    pub finished_at: Option<i64>,
    /// Files looked at.
    pub scanned: Option<i64>,
    /// What was found.
    pub findings: Vec<builtpanel_node_protocol::Infection>,
    /// More was found than listed.
    pub more: bool,
    /// Why it failed.
    pub error: Option<String>,
}

/// A tenant's scans, newest first.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScanList {
    /// The scans.
    pub items: Vec<ScanView>,
}

fn scan_view(s: builtpanel_database::malware::Scan) -> ScanView {
    ScanView {
        id: s.id,
        folder: s.folder,
        quarantine: s.quarantine,
        source: s.source,
        state: s.state,
        requested_at: s.requested_at,
        finished_at: s.finished_at,
        scanned: s.scanned,
        findings: serde_json::from_value(s.findings).unwrap_or_default(),
        more: s.more,
        error: s.error,
    }
}

async fn list_scans(state: &SecurityState, tenant: Uuid) -> Result<Json<ScanList>, AppError> {
    let items = builtpanel_database::malware::list(&state.db, tenant, "malware", 50)
        .await
        .map_err(internal)?
        .into_iter()
        .map(scan_view)
        .collect();
    Ok(Json(ScanList { items }))
}

async fn start_scan(
    state: &SecurityState,
    who: &Principal,
    tenant: Uuid,
    source: &str,
    req: ScanRequest,
) -> Result<(StatusCode, Json<ScanView>), AppError> {
    let folder = req
        .folder
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty());
    if let Some(f) = folder
        && (f.len() > 1024
            || f.contains(['\0', '\\'])
            || f.starts_with('/')
            || f.split('/').any(|p| p.is_empty() || p == "." || p == ".."))
    {
        return Err(AppError::validation(vec![FieldError::new(
            "folder",
            "invalid",
            "A folder inside your files, such as wp-content/uploads.",
        )]));
    }
    let Some(id) = builtpanel_database::malware::request(
        &state.db,
        tenant,
        "malware",
        folder,
        req.quarantine,
        source,
        Some(who.account_id()),
    )
    .await
    .map_err(internal)?
    else {
        return Err(AppError::conflict(
            "scan_under_way",
            "A scan of these files is queued or running already.",
        ));
    };
    let entry = AuditEntry::new("malware_scan.requested", Outcome::Success)
        .actor(who.account_id())
        .tenant(tenant)
        .target("malware_scan", id)
        .metadata(serde_json::json!({ "folder": folder, "quarantine": req.quarantine }));
    builtpanel_audit::insert(&state.db, &entry)
        .await
        .map_err(internal)?;
    let made = builtpanel_database::malware::list(&state.db, tenant, "malware", 1)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| AppError::not_found("malware_scan"))?;
    Ok((StatusCode::ACCEPTED, Json(scan_view(made))))
}

/// A hosting account's malware scans.
#[utoipa::path(get, path = "/api/v1/admin/tenants/{id}/malware-scans", tag = "security", operation_id = "adminListMalwareScans",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Hosting account id")),
    responses((status = 200, description = "The scans", body = ScanList),
              (status = 404, description = "No such hosting account", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_list_scans(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScanList>, AppError> {
    tenants::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("tenant"))?;
    list_scans(&state, id).await
}

/// Scan a hosting account's files for malware (the node does it in the next seconds).
#[utoipa::path(post, path = "/api/v1/admin/tenants/{id}/malware-scans", tag = "security", operation_id = "adminStartMalwareScan",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Hosting account id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ScanRequest,
    responses((status = 202, description = "Queued", body = ScanView),
              (status = 404, description = "No such hosting account", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`scan_under_way`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A folder that is not inside the files", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_start_scan(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<ScanRequest>,
) -> Result<(StatusCode, Json<ScanView>), AppError> {
    tenants::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("tenant"))?;
    start_scan(&state, &who.0, id, "admin", req).await
}

/// The account's own malware scans.
#[utoipa::path(get, path = "/api/v1/client/malware-scans", tag = "security", operation_id = "clientListMalwareScans",
    security(("session" = [])),
    responses((status = 200, description = "The scans", body = ScanList),
              (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_list_scans(
    State(state): State<SecurityState>,
    who: Principal,
) -> Result<Json<ScanList>, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    list_scans(&state, tenant.id).await
}

/// Scan the account's own files for malware.
#[utoipa::path(post, path = "/api/v1/client/malware-scans", tag = "security", operation_id = "clientStartMalwareScan",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ScanRequest,
    responses((status = 202, description = "Queued", body = ScanView),
              (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`scan_under_way`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A folder that is not inside the files", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_start_scan(
    State(state): State<SecurityState>,
    who: Principal,
    Json(req): Json<ScanRequest>,
) -> Result<(StatusCode, Json<ScanView>), AppError> {
    let tenant = client_tenant(&state, &who).await?;
    start_scan(&state, &who, tenant.id, "client", req).await
}

/// A WordPress version known to be vulnerable.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdvisoryView {
    /// Id.
    pub id: Uuid,
    /// `core`, `plugin` or `theme`.
    pub component: String,
    /// The folder name (`wordpress` for the core).
    pub slug: String,
    /// Versions below this are affected.
    pub fixed_in: String,
    /// `low`, `medium`, `high` or `critical`.
    pub severity: String,
    /// What it is.
    pub title: String,
    /// Where to read more.
    pub reference: Option<String>,
}

/// The advisories.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdvisoryList {
    /// By component and slug.
    pub items: Vec<AdvisoryView>,
}

/// An advisory to add.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddAdvisory {
    /// `core`, `plugin` or `theme`.
    pub component: String,
    /// The folder name (`wordpress` for the core).
    pub slug: String,
    /// Versions below this are affected.
    pub fixed_in: String,
    /// `low`, `medium`, `high` or `critical`.
    pub severity: String,
    /// What it is (1 to 200 characters).
    pub title: String,
    /// Where to read more (`https://…`).
    #[serde(default)]
    pub reference: Option<String>,
}

fn advisory_view(a: builtpanel_database::wp_advisories::Advisory) -> AdvisoryView {
    AdvisoryView {
        id: a.id,
        component: a.component,
        slug: a.slug,
        fixed_in: a.fixed_in,
        severity: a.severity,
        title: a.title,
        reference: a.reference,
    }
}

/// The advisories the vulnerability scans compare with.
#[utoipa::path(get, path = "/api/v1/admin/security/wp-advisories", tag = "security", operation_id = "adminListWpAdvisories",
    security(("session" = [])),
    responses((status = 200, description = "The advisories", body = AdvisoryList)))]
pub async fn list_advisories(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
) -> Result<Json<AdvisoryList>, AppError> {
    let items = builtpanel_database::wp_advisories::list(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .map(advisory_view)
        .collect();
    Ok(Json(AdvisoryList { items }))
}

/// Add an advisory: every version of the component below `fixed_in` is reported.
#[utoipa::path(post, path = "/api/v1/admin/security/wp-advisories", tag = "security", operation_id = "adminAddWpAdvisory",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = AddAdvisory,
    responses((status = 201, description = "Added", body = AdvisoryView),
              (status = 409, description = "`already_listed` or `list_full`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A field that is not valid", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn add_advisory(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Json(req): Json<AddAdvisory>,
) -> Result<(StatusCode, Json<AdvisoryView>), AppError> {
    let bad = |name: &'static str, message: &str| {
        AppError::validation(vec![FieldError::new(name, "invalid", message)])
    };
    let slug = req.slug.trim().to_owned();
    let fixed_in = req.fixed_in.trim().to_owned();
    let title = req.title.trim().to_owned();
    if !["core", "plugin", "theme"].contains(&req.component.as_str()) {
        return Err(bad("component", "core, plugin or theme."));
    }
    if slug.is_empty()
        || slug.len() > 100
        || !slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err(bad("slug", "The folder name: letters, digits, . _ -"));
    }
    if fixed_in.len() > 40
        || !fixed_in.starts_with(|c: char| c.is_ascii_digit())
        || !fixed_in
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".+_-".contains(c))
    {
        return Err(bad("fixed_in", "A version such as 6.4.3."));
    }
    if !["low", "medium", "high", "critical"].contains(&req.severity.as_str()) {
        return Err(bad("severity", "low, medium, high or critical."));
    }
    if title.is_empty() || title.chars().count() > 200 || title.chars().any(char::is_control) {
        return Err(bad("title", "1 to 200 characters, no control characters."));
    }
    let reference = req
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    if reference.is_some_and(|r| {
        r.len() > 300
            || !(r.starts_with("https://") || r.starts_with("http://"))
            || r.chars().any(char::is_control)
    }) {
        return Err(bad(
            "reference",
            "An http(s) address of at most 300 characters.",
        ));
    }
    let a = builtpanel_database::wp_advisories::Advisory {
        id: Uuid::nil(),
        component: req.component,
        slug,
        fixed_in,
        severity: req.severity,
        title,
        reference: reference.map(str::to_owned),
    };
    let id = builtpanel_database::wp_advisories::add(&state.db, &a, who.0.account_id())
        .await
        .map_err(|e| match e {
            builtpanel_database::wp_advisories::AddError::Exists => {
                AppError::conflict("already_listed", "This advisory is listed already.")
            }
            builtpanel_database::wp_advisories::AddError::Full => {
                AppError::conflict("list_full", "The list is full.")
            }
            builtpanel_database::wp_advisories::AddError::Database(db) => internal(db),
        })?;
    let entry = AuditEntry::new("wp_advisory.added", Outcome::Success)
        .actor(who.0.account_id())
        .target("wp_advisory", id)
        .metadata(
            serde_json::json!({ "component": a.component, "slug": a.slug, "fixed_in": a.fixed_in }),
        );
    builtpanel_audit::insert(&state.db, &entry)
        .await
        .map_err(internal)?;
    Ok((
        StatusCode::CREATED,
        Json(advisory_view(
            builtpanel_database::wp_advisories::Advisory { id, ..a },
        )),
    ))
}

/// Take an advisory away.
#[utoipa::path(delete, path = "/api/v1/admin/security/wp-advisories/{id}", tag = "security", operation_id = "adminRemoveWpAdvisory",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 204, description = "Removed"),
              (status = 404, description = "Not listed", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn remove_advisory(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if !builtpanel_database::wp_advisories::remove(&state.db, id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("wp_advisory"));
    }
    let entry = AuditEntry::new("wp_advisory.removed", Outcome::Success)
        .actor(who.0.account_id())
        .target("wp_advisory", id);
    builtpanel_audit::insert(&state.db, &entry)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// One problem a vulnerability scan found: a version below the one an advisory says is safe.
#[derive(Debug, Serialize, ToSchema)]
pub struct VulnIssue {
    /// The installation's folder.
    pub path: String,
    /// `core`, `plugin` or `theme`.
    pub component: String,
    /// The folder name (`wordpress` for the core).
    pub slug: String,
    /// The version found.
    pub version: String,
    /// The first safe version.
    pub fixed_in: String,
    /// `low`, `medium`, `high` or `critical`.
    pub severity: String,
    /// What it is.
    pub title: String,
    /// Where to read more.
    pub reference: Option<String>,
}

/// A WordPress vulnerability scan.
#[derive(Debug, Serialize, ToSchema)]
pub struct VulnScanView {
    /// Id.
    pub id: Uuid,
    /// `admin`, `client` or `schedule`.
    pub source: String,
    /// `queued`, `running`, `done` or `failed`.
    pub state: String,
    /// When it was asked for (Unix seconds).
    pub requested_at: i64,
    /// When it ended (Unix seconds).
    pub finished_at: Option<i64>,
    /// The installations found, with their versions.
    pub installs: Vec<builtpanel_node_protocol::WpInstall>,
    /// What is out of date or known vulnerable, worst first (judged by the advisories now in
    /// force, not those of the day of the scan).
    pub issues: Vec<VulnIssue>,
    /// Why it failed.
    pub error: Option<String>,
}

/// A tenant's vulnerability scans, newest first.
#[derive(Debug, Serialize, ToSchema)]
pub struct VulnScanList {
    /// The scans.
    pub items: Vec<VulnScanView>,
}

/// Whether version `found` is older than `fixed`: dotted numbers compared one by one, and a
/// pre-release (`6.5-beta2`, `1.0rc1`) before its release.
#[must_use]
pub fn is_older(found: &str, fixed: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, bool) {
        let lower = v.to_ascii_lowercase();
        let pre = ["alpha", "beta", "rc", "dev"]
            .iter()
            .any(|w| lower.contains(w));
        let mut nums = Vec::new();
        for piece in lower.split(['.', '-', '_', '+']) {
            let digits: String = piece.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() {
                break;
            }
            nums.push(digits.parse().unwrap_or(0));
            if digits.len() != piece.len() {
                break;
            }
        }
        (nums, pre)
    }
    let (mut a, a_pre) = parts(found);
    let (mut b, _) = parts(fixed);
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    match a.cmp(&b) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => a_pre,
    }
}

fn severity_rank(s: &str) -> u8 {
    match s {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        _ => 3,
    }
}

fn issues_of(
    installs: &[builtpanel_node_protocol::WpInstall],
    advisories: &[builtpanel_database::wp_advisories::Advisory],
) -> Vec<VulnIssue> {
    let mut out = Vec::new();
    for install in installs {
        let mut check = |component: &str, slug: &str, version: &str| {
            if version.is_empty() {
                return;
            }
            for a in advisories {
                if a.component == component
                    && a.slug.eq_ignore_ascii_case(slug)
                    && is_older(version, &a.fixed_in)
                {
                    out.push(VulnIssue {
                        path: install.path.clone(),
                        component: component.to_owned(),
                        slug: slug.to_owned(),
                        version: version.to_owned(),
                        fixed_in: a.fixed_in.clone(),
                        severity: a.severity.clone(),
                        title: a.title.clone(),
                        reference: a.reference.clone(),
                    });
                }
            }
        };
        check("core", "wordpress", &install.core);
        for p in &install.plugins {
            check("plugin", &p.slug, &p.version);
        }
        for t in &install.themes {
            check("theme", &t.slug, &t.version);
        }
    }
    out.sort_by(|a, b| {
        severity_rank(&a.severity)
            .cmp(&severity_rank(&b.severity))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.slug.cmp(&b.slug))
    });
    out
}

async fn list_vuln(state: &SecurityState, tenant: Uuid) -> Result<Json<VulnScanList>, AppError> {
    let advisories = builtpanel_database::wp_advisories::list(&state.db)
        .await
        .map_err(internal)?;
    let items = builtpanel_database::malware::list(&state.db, tenant, "wordpress", 20)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|s| vuln_view(s, &advisories))
        .collect();
    Ok(Json(VulnScanList { items }))
}

fn vuln_view(
    s: builtpanel_database::malware::Scan,
    advisories: &[builtpanel_database::wp_advisories::Advisory],
) -> VulnScanView {
    let installs: Vec<builtpanel_node_protocol::WpInstall> =
        serde_json::from_value(s.findings).unwrap_or_default();
    let issues = issues_of(&installs, advisories);
    VulnScanView {
        id: s.id,
        source: s.source,
        state: s.state,
        requested_at: s.requested_at,
        finished_at: s.finished_at,
        installs,
        issues,
        error: s.error,
    }
}

async fn start_vuln(
    state: &SecurityState,
    who: &Principal,
    tenant: Uuid,
    source: &str,
) -> Result<(StatusCode, Json<VulnScanView>), AppError> {
    let Some(id) = builtpanel_database::malware::request(
        &state.db,
        tenant,
        "wordpress",
        None,
        false,
        source,
        Some(who.account_id()),
    )
    .await
    .map_err(internal)?
    else {
        return Err(AppError::conflict(
            "scan_under_way",
            "A scan of these files is queued or running already.",
        ));
    };
    let entry = AuditEntry::new("vulnerability_scan.requested", Outcome::Success)
        .actor(who.account_id())
        .tenant(tenant)
        .target("malware_scan", id);
    builtpanel_audit::insert(&state.db, &entry)
        .await
        .map_err(internal)?;
    let made = builtpanel_database::malware::list(&state.db, tenant, "wordpress", 1)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| AppError::not_found("vulnerability_scan"))?;
    Ok((StatusCode::ACCEPTED, Json(vuln_view(made, &[]))))
}

/// A hosting account's WordPress vulnerability scans.
#[utoipa::path(get, path = "/api/v1/admin/tenants/{id}/vulnerability-scans", tag = "security", operation_id = "adminListVulnerabilityScans",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Hosting account id")),
    responses((status = 200, description = "The scans", body = VulnScanList),
              (status = 404, description = "No such hosting account", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_list_vuln(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<VulnScanList>, AppError> {
    tenants::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("tenant"))?;
    list_vuln(&state, id).await
}

/// Look up the versions of the WordPress installations in a hosting account's files.
#[utoipa::path(post, path = "/api/v1/admin/tenants/{id}/vulnerability-scans", tag = "security", operation_id = "adminStartVulnerabilityScan",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Hosting account id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 202, description = "Queued", body = VulnScanView),
              (status = 404, description = "No such hosting account", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`scan_under_way`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_start_vuln(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<VulnScanView>), AppError> {
    tenants::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("tenant"))?;
    start_vuln(&state, &who.0, id, "admin").await
}

/// The account's own WordPress vulnerability scans.
#[utoipa::path(get, path = "/api/v1/client/vulnerability-scans", tag = "security", operation_id = "clientListVulnerabilityScans",
    security(("session" = [])),
    responses((status = 200, description = "The scans", body = VulnScanList),
              (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_list_vuln(
    State(state): State<SecurityState>,
    who: Principal,
) -> Result<Json<VulnScanList>, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    list_vuln(&state, tenant.id).await
}

/// Look up the versions of the WordPress installations in the account's own files.
#[utoipa::path(post, path = "/api/v1/client/vulnerability-scans", tag = "security", operation_id = "clientStartVulnerabilityScan",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 202, description = "Queued", body = VulnScanView),
              (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`scan_under_way`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_start_vuln(
    State(state): State<SecurityState>,
    who: Principal,
) -> Result<(StatusCode, Json<VulnScanView>), AppError> {
    let tenant = client_tenant(&state, &who).await?;
    start_vuln(&state, &who, tenant.id, "client").await
}

/// An image of the allow-list and what is known about it.
#[derive(Debug, Serialize, ToSchema)]
pub struct SecurityImageView {
    /// The pinned reference (`name@sha256:…`).
    pub reference: String,
    /// The role it plays (`web`, `php`, `db`, …).
    pub role: String,
    /// The moving tag it was pinned from.
    pub tag: Option<String>,
    /// What it is.
    pub note: String,
    /// What the registry says the tag is now.
    pub latest_digest: Option<String>,
    /// A newer image exists: the tag now has another digest than the pin.
    pub update_available: bool,
    /// When the registry was asked (Unix seconds).
    pub checked_at: Option<i64>,
    /// When it was scanned (Unix seconds); `null`: not yet.
    pub scanned_at: Option<i64>,
    /// Findings by severity (`null`: not scanned).
    pub critical: Option<i32>,
    /// Findings by severity.
    pub high: Option<i32>,
    /// Findings by severity.
    pub medium: Option<i32>,
    /// Findings by severity.
    pub low: Option<i32>,
}

/// The images of the allow-list.
#[derive(Debug, Serialize, ToSchema)]
pub struct ImageList {
    /// By role.
    pub items: Vec<SecurityImageView>,
}

/// The worst findings of one image.
#[derive(Debug, Serialize, ToSchema)]
pub struct ImageVulns {
    /// The image.
    pub reference: String,
    /// Worst first.
    pub items: Vec<builtpanel_node_protocol::ImageVulnerability>,
}

/// Which image to scan.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanImageRequest {
    /// The pinned reference, as the list shows it.
    pub reference: String,
}

fn image_view(i: builtpanel_database::images::Image) -> SecurityImageView {
    let pinned = i.reference.split_once('@').map(|(_, d)| d.to_owned());
    let update_available = match (&i.latest_digest, &pinned) {
        (Some(latest), Some(pinned)) => latest != pinned,
        _ => false,
    };
    SecurityImageView {
        reference: i.reference,
        role: i.role,
        tag: i.tag,
        note: i.note,
        latest_digest: i.latest_digest,
        update_available,
        checked_at: i.checked_at,
        scanned_at: i.scanned_at,
        critical: i.critical,
        high: i.high,
        medium: i.medium,
        low: i.low,
    }
}

/// The images of the allow-list: what a registry says is newer and what the last scan found.
#[utoipa::path(get, path = "/api/v1/admin/security/images", tag = "security", operation_id = "adminListImages",
    security(("session" = [])),
    responses((status = 200, description = "The images", body = ImageList)))]
pub async fn list_images(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
) -> Result<Json<ImageList>, AppError> {
    let items = builtpanel_database::images::list(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .map(image_view)
        .collect();
    Ok(Json(ImageList { items }))
}

/// The worst findings of one image.
#[utoipa::path(get, path = "/api/v1/admin/security/images/vulnerabilities", tag = "security", operation_id = "adminImageVulnerabilities",
    security(("session" = [])), params(("reference" = String, Query, description = "The pinned reference")),
    responses((status = 200, description = "The findings", body = ImageVulns),
              (status = 404, description = "Not on the list", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn image_vulnerabilities(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
    axum::extract::Query(q): axum::extract::Query<ScanImageRequest>,
) -> Result<Json<ImageVulns>, AppError> {
    let image = builtpanel_database::images::get(&state.db, &q.reference)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("image"))?;
    Ok(Json(ImageVulns {
        reference: image.reference,
        items: serde_json::from_value(image.top).unwrap_or_default(),
    }))
}

/// Scan an image now (a node does it in the next minutes; the first scan fetches the
/// vulnerability database, which takes longer).
#[utoipa::path(post, path = "/api/v1/admin/security/images/scan", tag = "security", operation_id = "adminScanImage",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ScanImageRequest,
    responses((status = 202, description = "Asked for"),
              (status = 404, description = "Not on the list", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn scan_image_now(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Json(req): Json<ScanImageRequest>,
) -> Result<StatusCode, AppError> {
    if !builtpanel_database::images::request_scan(&state.db, &req.reference)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("image"));
    }
    let entry = AuditEntry::new("image_scan.requested", Outcome::Success)
        .actor(who.0.account_id())
        .metadata(serde_json::json!({ "reference": req.reference }));
    builtpanel_audit::insert(&state.db, &entry)
        .await
        .map_err(internal)?;
    Ok(StatusCode::ACCEPTED)
}

/// A node's firewall policy.
#[derive(Debug, Serialize, ToSchema)]
pub struct NodeFirewall {
    /// The policy (`null`: the panel manages no firewall on this node).
    pub policy: Option<builtpanel_node_protocol::FirewallPolicy>,
}

fn firewall_of(
    value: serde_json::Value,
) -> Result<builtpanel_node_protocol::FirewallPolicy, AppError> {
    serde_json::from_value(value).map_err(internal)
}

/// A node's firewall policy.
#[utoipa::path(get, path = "/api/v1/admin/nodes/{id}/firewall", tag = "security", operation_id = "adminGetNodeFirewall",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Node id")),
    responses((status = 200, description = "The policy", body = NodeFirewall),
              (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn get_node_firewall(
    State(state): State<SecurityState>,
    _who: Require<SecurityRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeFirewall>, AppError> {
    if !builtpanel_database::node_firewalls::node_exists(&state.db, id)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("node"));
    }
    let policy = builtpanel_database::node_firewalls::get(&state.db, id)
        .await
        .map_err(internal)?
        .map(firewall_of)
        .transpose()?;
    Ok(Json(NodeFirewall { policy }))
}

/// Set a node's firewall policy; the node is told within seconds (or when it connects). A
/// policy that denies by default needs an `always_allow` address, so the panel stays reachable.
#[utoipa::path(put, path = "/api/v1/admin/nodes/{id}/firewall", tag = "security", operation_id = "adminSetNodeFirewall",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = builtpanel_node_protocol::FirewallPolicy,
    responses((status = 200, description = "Stored (the checked form: addresses written plainly)", body = NodeFirewall),
              (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A policy that is not valid", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn put_node_firewall(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<builtpanel_node_protocol::FirewallPolicy>,
) -> Result<Json<NodeFirewall>, AppError> {
    let policy = req.checked().map_err(|message| {
        AppError::validation(vec![FieldError::new("policy", "invalid", &message)])
    })?;
    store_firewall(&state, &who.0, id, policy).await
}

/// Take every rule away from a node's firewall (the panel keeps managing it: an empty policy).
#[utoipa::path(delete, path = "/api/v1/admin/nodes/{id}/firewall", tag = "security", operation_id = "adminResetNodeFirewall",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 200, description = "Emptied", body = NodeFirewall),
              (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn reset_node_firewall(
    State(state): State<SecurityState>,
    who: Require<SecurityManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeFirewall>, AppError> {
    store_firewall(
        &state,
        &who.0,
        id,
        builtpanel_node_protocol::FirewallPolicy::default(),
    )
    .await
}

async fn store_firewall(
    state: &SecurityState,
    who: &Principal,
    id: Uuid,
    policy: builtpanel_node_protocol::FirewallPolicy,
) -> Result<Json<NodeFirewall>, AppError> {
    let value = serde_json::to_value(&policy).map_err(internal)?;
    if !builtpanel_database::node_firewalls::set(&state.db, id, &value, who.account_id())
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("node"));
    }
    let entry = AuditEntry::new("node_firewall.set", Outcome::Success)
        .actor(who.account_id())
        .target("node", id)
        .metadata(serde_json::json!({
            "default_deny": policy.default_deny,
            "always_allow": policy.always_allow.len(),
            "allowed": policy.allowed.len(),
            "blocked": policy.blocked.len(),
            "open_ports": policy.open_ports.len(),
            "rate_limits": policy.rate_limits.len(),
        }));
    builtpanel_audit::insert(&state.db, &entry)
        .await
        .map_err(internal)?;
    Ok(Json(NodeFirewall {
        policy: Some(policy),
    }))
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(
        admin_get_global, admin_put_global, admin_delete_global, admin_get_tenant, admin_put_tenant,
        admin_delete_tenant, admin_get_domain, admin_put_domain, admin_delete_domain, client_list,
        client_put, client_delete, client_list_ip_blocks, client_add_ip_block, client_remove_ip_block, list_user_agents, add_user_agent, remove_user_agent,
        get_node_firewall, put_node_firewall, reset_node_firewall,
        admin_list_scans, admin_start_scan, client_list_scans, client_start_scan,
        admin_list_vuln, admin_start_vuln, client_list_vuln, client_start_vuln,
        list_advisories, add_advisory, remove_advisory,
        list_images, image_vulnerabilities, scan_image_now
    ),
    components(schemas(SecurityImageView, ImageList, ImageVulns, ScanImageRequest, builtpanel_node_protocol::ImageVulnerability, VulnScanView, VulnScanList, VulnIssue, AdvisoryView, AdvisoryList, AddAdvisory, builtpanel_node_protocol::WpInstall, builtpanel_node_protocol::WpComponent, ScanRequest, ScanView, ScanList, builtpanel_node_protocol::Infection, NodeFirewall, builtpanel_node_protocol::FirewallPolicy, builtpanel_node_protocol::PortRule, builtpanel_node_protocol::RateLimit, UserAgentView, UserAgentList, AddUserAgent, PolicyRequest, WafPolicyView, EffectiveView, PolicyState, DomainWaf, DomainWafList, IpBlockView, IpBlockList, AddIpBlock)),
    tags((name = "security", description = "The web application firewall"))
)]
pub struct SecurityApi;

#[cfg(test)]
mod tests {
    use super::is_older;

    #[test]
    fn versions_are_compared_number_by_number_and_a_pre_release_comes_first() {
        for (found, fixed) in [
            ("1.2.3", "1.2.4"),
            ("1.9", "1.10"),
            ("6.4.3", "6.5"),
            ("2.0", "2.0.1"),
            ("6.5-beta2", "6.5"),
            ("1.0rc1", "1.0"),
        ] {
            assert!(is_older(found, fixed), "{found} < {fixed}");
            assert!(!is_older(fixed, found), "{fixed} !< {found}");
        }
        for (found, fixed) in [
            ("1.2.3", "1.2.3"),
            ("2.0", "2.0.0"),
            ("10.0", "9.9.9"),
            ("", "1.0"),
        ] {
            assert!(
                !is_older(found, fixed) || found.is_empty(),
                "{found} vs {fixed}"
            );
        }
    }
}
