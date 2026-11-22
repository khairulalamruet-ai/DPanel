//! Domains (T05.03): the names a hosting account serves.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET    | `/api/v1/admin/domains` | `admin.domains.read` |
//! | POST   | `/api/v1/admin/domains` | `admin.domains.manage`, sudo mode |
//! | GET    | `/api/v1/admin/domains/{id}` | `admin.domains.read` |
//! | PATCH  | `/api/v1/admin/domains/{id}` | `admin.domains.manage`, sudo mode |
//! | DELETE | `/api/v1/admin/domains/{id}` | `admin.domains.manage`, sudo mode |
//! | POST   | `/api/v1/admin/domains/{id}/suspend` | `admin.domains.manage`, sudo mode |
//! | POST   | `/api/v1/admin/domains/{id}/unsuspend` | `admin.domains.manage`, sudo mode |
//! | GET, POST | `/api/v1/client/domains` | a client account, its own hosting |
//! | GET, PATCH, DELETE | `/api/v1/client/domains/{id}` | a client account, its own hosting |
//!
//! A client works on its own hosting account only: the account is found from the
//! session, never from the request, and a domain of another account answers
//! 404 as if it did not exist. Suspending and resuming a single domain is an
//! administrator's decision, so a client cannot undo it.
//!
//! Adding a domain checks, in this order: the plan includes the `domains`
//! feature (403 `feature_disabled`), the name is valid and not blocked (422),
//! the plan's limit has room (409 `limit_reached`: `max_domains` counts main,
//! addon and alias names, `max_subdomains` counts subdomains; a plan that does
//! not list a limit gets the module's default), and the ownership rules of
//! [`builtpanel_database::domains::add`] (409 `domain_taken`,
//! `domain_owned_by_other`). Counting and adding happen under one lock per
//! account, so two requests cannot both take the last place.
//!
//! The panel-wide settings of this module (`permit_subdomain_sharing`,
//! `blocked_domains`) are read on every add, so a change takes effect at once.

pub mod dns;
pub mod error_pages;
pub mod nameservers;
pub mod pagecache;
pub mod php;
pub mod php_settings;
pub mod policy;
pub mod previews;
pub mod records;
pub mod redirects;
pub mod services;
pub mod ssl;
pub mod tls;
pub mod vhost;
pub mod vhost_templates;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    accounts::DbAccountKind,
    domains::{self, AddError, DomainListing, Filter, NewDomain},
    plans::{self, Plan},
    sqlx::PgPool,
    tenants, users,
};
use builtpanel_domain::{
    Blocklist, Docroot, DomainError, DomainKind, DomainStatus, Fqdn, TenantStatus,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{DomainsManage, DomainsRead},
};
use builtpanel_session::{SessionStore, rfc3339};
use builtpanel_settings::{Scope, SettingsRegistry};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

/// The domains module's manifest text.
pub const MANIFEST: &str = include_str!("../../../../modules/domains/module.toml");
/// The module's settings, as a JSON Schema.
pub const SETTINGS_SCHEMA: &str = include_str!("../../../../modules/domains/settings.schema.json");

/// Entries per page when `limit` is not given.
pub const DEFAULT_LIMIT: i64 = 50;
/// Most entries per page.
pub const MAX_LIMIT: i64 = 200;
/// What a plan without the limit allows (the manifest's defaults).
const DEFAULT_MAX_DOMAINS: i64 = 10;
const DEFAULT_MAX_SUBDOMAINS: i64 = 20;

/// Dependencies of the domains handlers.
#[derive(Clone)]
pub struct DomainsState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The checked settings schemas of all modules.
    pub settings: Arc<SettingsRegistry>,
    /// Asks the public DNS which name servers a domain has.
    pub resolver: Arc<dyn builtpanel_dns::resolver::NsResolver>,
    /// Limits how often one address may ask whether a name may get a certificate.
    pub ask_limiter: builtpanel_cache::ratelimit::RateLimiter,
    /// Limits how often an account may restart its services.
    pub restart_limiter: builtpanel_cache::ratelimit::RateLimiter,
    /// Seals the private keys of uploaded certificates.
    pub master_key: builtpanel_auth_core::secretbox::MasterKey,
}

impl FromRef<DomainsState> for PgPool {
    fn from_ref(s: &DomainsState) -> Self {
        s.db.clone()
    }
}

impl FromRef<DomainsState> for SessionStore {
    fn from_ref(s: &DomainsState) -> Self {
        s.sessions.clone()
    }
}

/// Routes for administrators, with full paths.
pub fn admin_router() -> Router<DomainsState> {
    error_pages::admin_router()
        .merge(php::admin_router())
        .merge(php_settings::admin_router())
        .merge(pagecache::admin_router())
        .merge(policy::admin_router())
        .merge(vhost::admin_router())
        .merge(vhost_templates::admin_router())
        .merge(redirects::admin_router())
        .merge(records::admin_router())
        .merge(nameservers::admin_router())
        .merge(ssl::admin_router())
        .merge(dns::admin_router())
        .route("/api/v1/admin/domains", get(admin_list).post(admin_add))
        .route(
            "/api/v1/admin/domains/{id}",
            get(admin_get).patch(admin_update).delete(admin_delete),
        )
        .route("/api/v1/admin/domains/{id}/suspend", post(admin_suspend))
        .route(
            "/api/v1/admin/domains/{id}/unsuspend",
            post(admin_unsuspend),
        )
}

/// Routes for clients, with full paths.
pub fn client_router() -> Router<DomainsState> {
    php::client_router()
        .merge(previews::client_router())
        .merge(services::client_router())
        .merge(php_settings::client_router())
        .merge(pagecache::client_router())
        .merge(policy::client_router())
        .merge(vhost::client_router())
        .merge(redirects::client_router())
        .merge(records::client_router())
        .merge(nameservers::client_router())
        .merge(ssl::client_router())
        .route("/api/v1/client/domains", get(client_list).post(client_add))
        .route(
            "/api/v1/client/domains/{id}",
            get(client_get).patch(client_update).delete(client_delete),
        )
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

// ------------------------------------------------------------------- views

/// A domain.
#[derive(Debug, Serialize, ToSchema)]
pub struct DomainView {
    /// Id.
    pub id: Uuid,
    /// The name as stored (lower-case ASCII; punycode for international names).
    #[schema(example = "example.com")]
    pub fqdn: String,
    /// The name as a person reads it.
    pub unicode: String,
    /// `primary`, `addon`, `sub` or `alias`.
    #[schema(example = "addon")]
    pub kind: String,
    /// The domain a `sub` or `alias` belongs to.
    pub parent_domain_id: Option<Uuid>,
    /// Where its files are, relative to the web directory (empty: the directory itself).
    pub docroot: String,
    /// `active` or `suspended`.
    pub status: String,
    /// Visitors on plain HTTP are sent to HTTPS.
    pub force_https: bool,
    /// Answers over HTTPS carry the HSTS header (one year). Needs `force_https`.
    pub hsts: bool,
    /// When it was suspended (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub suspended_at: Option<String>,
    /// The hosting account it belongs to.
    pub tenant_id: Uuid,
    /// That account's system name.
    pub tenant_username: String,
    /// Creation time (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
}

fn view(listing: DomainListing) -> DomainView {
    let d = listing.domain;
    let unicode = Fqdn::parse(&d.fqdn).map_or_else(|_| d.fqdn.clone(), |f| f.unicode());
    DomainView {
        id: d.id,
        unicode,
        fqdn: d.fqdn,
        kind: d.kind.as_str().to_owned(),
        parent_domain_id: d.parent_domain_id,
        docroot: d.docroot,
        status: d.status.as_str().to_owned(),
        force_https: d.force_https,
        hsts: d.hsts,
        suspended_at: listing.suspended_at.map(rfc3339),
        tenant_id: d.tenant_id,
        tenant_username: listing.tenant_username,
        created_at: rfc3339(d.created_at),
    }
}

/// A page of domains.
#[derive(Debug, Serialize, ToSchema)]
pub struct DomainList {
    /// The domains, by name.
    pub items: Vec<DomainView>,
    /// How many match in all.
    pub total: i64,
}

// ----------------------------------------------------------------- context

/// The hosting account a request works on.
struct Hosting {
    tenant: tenants::Tenant,
    plan: Plan,
}

async fn hosting(state: &DomainsState, tenant_id: Uuid) -> Result<Hosting, AppError> {
    let tenant = tenants::get(&state.db, tenant_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))?;
    let plan = plans::get(&state.db, tenant.plan_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("the plan of a tenant vanished")))?;
    Ok(Hosting { tenant, plan })
}

/// The hosting account of the signed-in client.
async fn client_hosting(state: &DomainsState, who: &Principal) -> Result<Hosting, AppError> {
    if who.kind != DbAccountKind::Client {
        return Err(AppError::forbidden(
            "client_only",
            "This is for hosting accounts. Administrators use /api/v1/admin/domains.",
        ));
    }
    let user = users::get(&state.db, who.account_id())
        .await
        .map_err(internal)?;
    let tenant = user.and_then(|u| u.tenant).ok_or_else(|| {
        AppError::conflict("no_hosting", "This account has no hosting account yet.")
    })?;
    hosting(state, tenant.id).await
}

/// Panel-wide settings of this module.
struct Policy {
    permit_subdomain_sharing: bool,
    blocklist: Blocklist,
}

async fn policy(state: &DomainsState) -> Result<Policy, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "domains", Scope::Global)
        .await
        .map_err(internal)?;
    // One text: names separated by line breaks, commas or spaces.
    let entries: Vec<&str> = values
        .get("blocked_domains")
        .and_then(serde_json::Value::as_str)
        .map(|text| {
            text.split(|c: char| c.is_whitespace() || c == ',')
                .filter(|e| !e.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(Policy {
        permit_subdomain_sharing: values
            .get("permit_subdomain_sharing")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        // Entries were validated as names when they were saved; one that still
        // fails now (the rules changed) closes nothing rather than everything.
        blocklist: Blocklist::new(entries).unwrap_or_default(),
    })
}

/// What a plan allows of a kind of limit; `None` is unlimited.
fn plan_limit(plan: &Plan, key: &str, default: i64) -> Option<i64> {
    let value = plan
        .limits
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(default);
    (value >= 0).then_some(value)
}

fn field(name: &'static str, error: &DomainError) -> AppError {
    AppError::validation(vec![FieldError::new(name, error.code(), error.to_string())])
}

fn add_error(err: AddError) -> AppError {
    match err {
        AddError::Taken => AppError::conflict("domain_taken", "This domain is already in use."),
        AddError::OwnedByOther => AppError::conflict(
            "domain_owned_by_other",
            "This domain lies below a domain that belongs to someone else.",
        ),
        AddError::PrimaryExists => AppError::conflict(
            "primary_exists",
            "This account already has a primary domain.",
        ),
        AddError::ParentMismatch => AppError::validation(vec![FieldError::new(
            "parent_domain_id",
            "parent_mismatch",
            "A subdomain or alias needs the domain it belongs to; other kinds must not name one.",
        )]),
        AddError::ParentNotFound => AppError::validation(vec![FieldError::new(
            "parent_domain_id",
            "parent_not_found",
            "There is no such domain among yours.",
        )]),
        AddError::NotBelowParent => AppError::validation(vec![FieldError::new(
            "fqdn",
            "not_below_parent",
            "A subdomain must lie below the domain it belongs to.",
        )]),
        AddError::UnknownTenant => AppError::not_found("user"),
        AddError::Database(db) => internal(db),
    }
}

fn entry(action: &'static str, who: &Principal, hosting: &Hosting, id: Uuid) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .tenant(hosting.tenant.id)
        .target("domain", id)
}

// ---------------------------------------------------------------- core logic

/// What a request asks to add.
struct Wanted {
    fqdn: String,
    kind: Option<String>,
    parent_domain_id: Option<Uuid>,
    docroot: Option<String>,
}

async fn add_domain(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    wanted: Wanted,
) -> Result<DomainView, AppError> {
    if !matches!(
        hosting.tenant.status,
        TenantStatus::Active | TenantStatus::Suspended | TenantStatus::Provisioning
    ) {
        return Err(AppError::conflict(
            "tenant_unavailable",
            "This hosting account cannot take new domains now.",
        ));
    }
    if !plans::includes_feature(&state.db, hosting.plan.id, "domains")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Domains are not included in this plan.",
        ));
    }
    let mut errors = Vec::new();
    let fqdn = Fqdn::parse(&wanted.fqdn)
        .map_err(|e| errors.push(FieldError::new("fqdn", e.code(), e.to_string())))
        .ok();
    let kind = wanted
        .kind
        .as_deref()
        .map(DomainKind::parse)
        .transpose()
        .map_err(|e| errors.push(FieldError::new("kind", e.code(), e.to_string())))
        .ok()
        .flatten();
    let docroot = Docroot::parse(wanted.docroot.as_deref().unwrap_or_default())
        .map_err(|e| errors.push(FieldError::new("docroot", e.code(), e.to_string())))
        .ok();
    let (Some(fqdn), Some(docroot), true) = (fqdn, docroot, errors.is_empty()) else {
        return Err(AppError::validation(errors));
    };
    let policy = policy(state).await?;
    policy
        .blocklist
        .check(&fqdn)
        .map_err(|e| field("fqdn", &e))?;

    let mut tx = state.db.begin().await.map_err(internal)?;
    domains::lock_tenant(&mut tx, hosting.tenant.id)
        .await
        .map_err(internal)?;
    let (primary, addon, sub, alias) = domains::counts(&mut tx, hosting.tenant.id)
        .await
        .map_err(internal)?;
    // The first domain is the main one; later ones are addons unless said otherwise.
    let kind = kind.unwrap_or(if primary == 0 {
        DomainKind::Primary
    } else {
        DomainKind::Addon
    });
    let (key, default, current) = if kind == DomainKind::Sub {
        ("max_subdomains", DEFAULT_MAX_SUBDOMAINS, sub)
    } else {
        ("max_domains", DEFAULT_MAX_DOMAINS, primary + addon + alias)
    };
    if let Some(max) = plan_limit(&hosting.plan, key, default)
        && current + 1 > max
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("The plan allows at most {max} of these ({key})."),
        ));
    }
    let added = domains::add(
        &mut tx,
        &NewDomain {
            tenant_id: hosting.tenant.id,
            fqdn: &fqdn,
            kind,
            parent_domain_id: wanted.parent_domain_id,
            docroot: &docroot,
        },
        policy.permit_subdomain_sharing,
    )
    .await
    .map_err(add_error)?;
    let dns_note = dns::provision(&mut tx, hosting, &added, &fqdn).await?;
    builtpanel_events::publish(
        &mut tx,
        &builtpanel_kernel::Event::DomainAdded {
            tenant_id: hosting.tenant.id,
            domain_id: added.id,
            name: added.fqdn.clone(),
        },
    )
    .await
    .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("domain.added", who, hosting, added.id).metadata(serde_json::json!({
            "fqdn": added.fqdn,
            "kind": added.kind.as_str(),
            "docroot": added.docroot,
            "dns": dns_note,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(view(DomainListing {
        tenant_username: hosting.tenant.username.clone(),
        suspended_at: None,
        domain: added,
    }))
}

/// A domain of the tenant (or 404: somebody else's domain looks the same as none).
async fn owned_domain(
    state: &DomainsState,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<DomainListing, AppError> {
    let (rows, _) = domains::list(
        &state.db,
        &Filter {
            tenant_id: Some(tenant_id),
            ..Filter::default()
        },
        MAX_LIMIT,
        0,
    )
    .await
    .map_err(internal)?;
    // A tenant has few domains; the list is a single indexed query.
    rows.into_iter()
        .find(|r| r.domain.id == id)
        .ok_or_else(|| AppError::not_found("domain"))
}

async fn delete_domain(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    id: Uuid,
) -> Result<(), AppError> {
    let target = owned_domain(state, hosting.tenant.id, id).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    domains::lock_tenant(&mut tx, hosting.tenant.id)
        .await
        .map_err(internal)?;
    if target.domain.kind == DomainKind::Primary {
        let (_, addon, sub, alias) = domains::counts(&mut tx, hosting.tenant.id)
            .await
            .map_err(internal)?;
        if addon + sub + alias > 0 {
            return Err(AppError::conflict(
                "primary_has_domains",
                "Remove the other domains first, or the main domain stays.",
            ));
        }
    }
    // A subdomain's address records live in its parent's zone; take them out.
    if target.domain.kind == DomainKind::Sub {
        dns::forget_subdomain(&mut tx, hosting, &target.domain.fqdn).await?;
    }
    if !domains::delete(&mut tx, id).await.map_err(internal)? {
        return Err(AppError::not_found("domain"));
    }
    builtpanel_events::publish(
        &mut tx,
        &builtpanel_kernel::Event::DomainRemoved {
            tenant_id: hosting.tenant.id,
            domain_id: id,
            name: target.domain.fqdn.clone(),
        },
    )
    .await
    .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("domain.deleted", who, hosting, id).metadata(serde_json::json!({
            "fqdn": target.domain.fqdn,
            "kind": target.domain.kind.as_str(),
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(())
}

async fn change_domain(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    id: Uuid,
    req: UpdateDomain,
) -> Result<DomainView, AppError> {
    if req.docroot.is_none() && req.force_https.is_none() && req.hsts.is_none() {
        return Err(AppError::validation(vec![FieldError::new(
            "docroot",
            "nothing_to_change",
            "Give a document root, force_https or hsts to change.",
        )]));
    }
    let docroot = req
        .docroot
        .as_deref()
        .map(|d| Docroot::parse(d).map_err(|e| field("docroot", &e)))
        .transpose()?;
    let before = owned_domain(state, hosting.tenant.id, id).await?;
    let force_https = req.force_https.unwrap_or(before.domain.force_https);
    let hsts = req.hsts.unwrap_or(before.domain.hsts);
    if hsts && !force_https {
        return Err(AppError::validation(vec![FieldError::new(
            "hsts",
            "hsts_requires_https",
            "HSTS needs forced HTTPS: turn on force_https too (and turn HSTS off before forced HTTPS).",
        )]));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    if let Some(docroot) = &docroot {
        domains::set_docroot(&mut tx, id, docroot)
            .await
            .map_err(internal)?;
        builtpanel_audit::insert(
            &mut *tx,
            &entry("domain.docroot_changed", who, hosting, id).metadata(serde_json::json!({
                "fqdn": before.domain.fqdn,
                "from": before.domain.docroot,
                "to": docroot.as_str(),
            })),
        )
        .await
        .map_err(internal)?;
    }
    if force_https != before.domain.force_https || hsts != before.domain.hsts {
        domains::set_https(&mut tx, id, force_https, hsts)
            .await
            .map_err(internal)?;
        builtpanel_audit::insert(
            &mut *tx,
            &entry("domain.https_changed", who, hosting, id).metadata(serde_json::json!({
                "fqdn": before.domain.fqdn,
                "force_https": force_https,
                "hsts": hsts,
            })),
        )
        .await
        .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(view(owned_domain(state, hosting.tenant.id, id).await?))
}

async fn change_status(
    state: &DomainsState,
    who: &Principal,
    hosting: &Hosting,
    id: Uuid,
    status: DomainStatus,
) -> Result<DomainView, AppError> {
    let before = owned_domain(state, hosting.tenant.id, id).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    domains::set_status(&mut tx, id, status)
        .await
        .map_err(internal)?;
    let action = match status {
        DomainStatus::Suspended => "domain.suspended",
        DomainStatus::Active => "domain.unsuspended",
    };
    builtpanel_audit::insert(
        &mut *tx,
        &entry(action, who, hosting, id)
            .metadata(serde_json::json!({ "fqdn": before.domain.fqdn })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(view(owned_domain(state, hosting.tenant.id, id).await?))
}

// ------------------------------------------------------------------- inputs

/// A domain to add.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddDomain {
    /// The name; international names may be written in Unicode.
    #[schema(example = "example.com")]
    pub fqdn: String,
    /// `primary` (the first domain, by default), `addon` (by default after
    /// that), `sub` or `alias`.
    pub kind: Option<String>,
    /// The domain a `sub` or `alias` belongs to.
    pub parent_domain_id: Option<Uuid>,
    /// Where its files are, relative to the web directory (default: the directory itself).
    pub docroot: Option<String>,
}

/// A domain to add for a user.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminAddDomain {
    /// The user (account id) it is added for.
    pub user_id: Uuid,
    /// The name; international names may be written in Unicode.
    #[schema(example = "example.com")]
    pub fqdn: String,
    /// `primary` (the first domain, by default), `addon` (by default after
    /// that), `sub` or `alias`.
    pub kind: Option<String>,
    /// The domain a `sub` or `alias` belongs to.
    pub parent_domain_id: Option<Uuid>,
    /// Where its files are, relative to the web directory.
    pub docroot: Option<String>,
}

/// Changes to a domain.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateDomain {
    /// The new document root, relative to the web directory (empty: the directory itself).
    pub docroot: Option<String>,
    /// Send visitors who arrive over plain HTTP to HTTPS.
    pub force_https: Option<bool>,
    /// Tell browsers to use HTTPS for this name for a year (the
    /// `Strict-Transport-Security` header on HTTPS answers). Only with `force_https`;
    /// it cannot be turned on without it, nor can forced HTTPS be turned off while
    /// it is on.
    pub hsts: Option<bool>,
}

/// Filters and paging of the lists.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// Entries per page, 1..=200 (default 50).
    pub limit: Option<String>,
    /// Entries to skip (default 0).
    pub offset: Option<String>,
    /// Only this kind: `primary`, `addon`, `sub` or `alias`.
    pub kind: Option<String>,
    /// Only this status: `active` or `suspended`.
    pub status: Option<String>,
    /// Text to find in the name.
    pub q: Option<String>,
    /// Only this user's domains (administrators only; clients always see their own).
    pub user_id: Option<Uuid>,
}

async fn list_domains(
    state: &DomainsState,
    q: &ListQuery,
    tenant_id: Option<Uuid>,
) -> Result<Json<DomainList>, AppError> {
    let mut errors = Vec::new();
    let mut number = |name: &'static str, text: &Option<String>, default, min, max| match text
        .as_deref()
        .filter(|t| !t.is_empty())
    {
        None => default,
        Some(t) => match t.parse::<i64>() {
            Ok(n) if (min..=max).contains(&n) => n,
            _ => {
                errors.push(FieldError::new(
                    name,
                    "out_of_range",
                    format!("Expected a number from {min} to {max}."),
                ));
                default
            }
        },
    };
    let limit = number("limit", &q.limit, DEFAULT_LIMIT, 1, MAX_LIMIT);
    let offset = number("offset", &q.offset, 0, 0, 10_000_000);
    let kind = q
        .kind
        .as_deref()
        .filter(|k| !k.is_empty())
        .map(DomainKind::parse)
        .transpose()
        .map_err(|e| errors.push(FieldError::new("kind", e.code(), e.to_string())))
        .ok()
        .flatten();
    let status = q
        .status
        .as_deref()
        .filter(|k| !k.is_empty())
        .map(DomainStatus::parse)
        .transpose()
        .map_err(|e| errors.push(FieldError::new("status", e.code(), e.to_string())))
        .ok()
        .flatten();
    let search = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if search.is_some_and(|s| s.chars().count() > 100) {
        errors.push(FieldError::new("q", "too_long", "At most 100 characters."));
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let (rows, total) = domains::list(
        &state.db,
        &Filter {
            tenant_id,
            kind,
            status,
            search,
        },
        limit,
        offset,
    )
    .await
    .map_err(internal)?;
    Ok(Json(DomainList {
        items: rows.into_iter().map(view).collect(),
        total,
    }))
}

/// The hosting account of a user (an account id), for administrators.
async fn user_hosting(state: &DomainsState, user_id: Uuid) -> Result<Hosting, AppError> {
    let user = users::get(&state.db, user_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))?;
    let tenant = user
        .tenant
        .ok_or_else(|| AppError::conflict("no_hosting", "This user has no hosting account yet."))?;
    hosting(state, tenant.id).await
}

/// The tenant of a domain, whoever has it (for administrators).
async fn tenant_of_domain(state: &DomainsState, id: Uuid) -> Result<Hosting, AppError> {
    let found = domains::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("domain"))?;
    hosting(state, found.tenant_id).await
}

// ------------------------------------------------------------- admin handlers

/// List every domain on the panel.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains",
    tag = "domains",
    operation_id = "adminListDomains",
    security(("session" = [])),
    params(ListQuery),
    responses(
        (status = 200, description = "A page of domains", body = DomainList),
        (status = 403, description = "`permission_denied` (needs admin.domains.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid paging or filter", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_list(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Query(q): Query<ListQuery>,
) -> Result<Json<DomainList>, AppError> {
    let tenant_id = match q.user_id {
        Some(user_id) => Some(user_hosting(&state, user_id).await?.tenant.id),
        None => None,
    };
    list_domains(&state, &q, tenant_id).await
}

/// One domain.
#[utoipa::path(
    get,
    path = "/api/v1/admin/domains/{id}",
    tag = "domains",
    operation_id = "adminGetDomain",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The domain", body = DomainView),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_get(
    State(state): State<DomainsState>,
    _who: Require<DomainsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<DomainView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok(Json(view(owned_domain(&state, h.tenant.id, id).await?)))
}

/// Add a domain to a user.
#[utoipa::path(
    post,
    path = "/api/v1/admin/domains",
    tag = "domains",
    operation_id = "adminAddDomain",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = AdminAddDomain,
    responses(
        (status = 201, description = "Added", body = DomainView),
        (status = 403, description = "`permission_denied`, `reauth_required`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`domain_taken`, `domain_owned_by_other`, `primary_exists`, `limit_reached`, `no_hosting` or `tenant_unavailable`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "An invalid or blocked name, kind, parent or document root", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_add(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Json(req): Json<AdminAddDomain>,
) -> Result<(StatusCode, Json<DomainView>), AppError> {
    let h = user_hosting(&state, req.user_id).await?;
    let added = add_domain(
        &state,
        &who,
        &h,
        Wanted {
            fqdn: req.fqdn,
            kind: req.kind,
            parent_domain_id: req.parent_domain_id,
            docroot: req.docroot,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(added)))
}

/// Change a domain's document root.
#[utoipa::path(
    patch,
    path = "/api/v1/admin/domains/{id}",
    tag = "domains",
    operation_id = "adminUpdateDomain",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UpdateDomain,
    responses(
        (status = 200, description = "Updated", body = DomainView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid document root", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_update(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateDomain>,
) -> Result<Json<DomainView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok(Json(change_domain(&state, &who, &h, id, req).await?))
}

/// Remove a domain and what hangs off it.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/domains/{id}",
    tag = "domains",
    operation_id = "adminDeleteDomain",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`primary_has_domains`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_delete(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    delete_domain(&state, &who, &h, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Stop serving a domain.
#[utoipa::path(
    post,
    path = "/api/v1/admin/domains/{id}/suspend",
    tag = "domains",
    operation_id = "adminSuspendDomain",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Suspended", body = DomainView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_suspend(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<DomainView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok(Json(
        change_status(&state, &who, &h, id, DomainStatus::Suspended).await?,
    ))
}

/// Serve a suspended domain again.
#[utoipa::path(
    post,
    path = "/api/v1/admin/domains/{id}/unsuspend",
    tag = "domains",
    operation_id = "adminUnsuspendDomain",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Active again", body = DomainView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_unsuspend(
    State(state): State<DomainsState>,
    RequireRecent(who, _): RequireRecent<DomainsManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<DomainView>, AppError> {
    let h = tenant_of_domain(&state, id).await?;
    Ok(Json(
        change_status(&state, &who, &h, id, DomainStatus::Active).await?,
    ))
}

// ------------------------------------------------------------ client handlers

/// List the signed-in client's domains.
#[utoipa::path(
    get,
    path = "/api/v1/client/domains",
    tag = "domains",
    operation_id = "clientListDomains",
    security(("session" = [])),
    params(ListQuery),
    responses(
        (status = 200, description = "A page of domains", body = DomainList),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_hosting`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid paging or filter", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_list(
    State(state): State<DomainsState>,
    who: Principal,
    Query(q): Query<ListQuery>,
) -> Result<Json<DomainList>, AppError> {
    let h = client_hosting(&state, &who).await?;
    list_domains(&state, &q, Some(h.tenant.id)).await
}

/// One of the signed-in client's domains.
#[utoipa::path(
    get,
    path = "/api/v1/client/domains/{id}",
    tag = "domains",
    operation_id = "clientGetDomain",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Domain id")),
    responses(
        (status = 200, description = "The domain", body = DomainView),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_get(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<DomainView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok(Json(view(owned_domain(&state, h.tenant.id, id).await?)))
}

/// Add a domain to the signed-in client's hosting account.
#[utoipa::path(
    post,
    path = "/api/v1/client/domains",
    tag = "domains",
    operation_id = "clientAddDomain",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = AddDomain,
    responses(
        (status = 201, description = "Added", body = DomainView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`domain_taken`, `domain_owned_by_other`, `primary_exists`, `limit_reached`, `no_hosting` or `tenant_unavailable`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "An invalid or blocked name, kind, parent or document root", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_add(
    State(state): State<DomainsState>,
    who: Principal,
    Json(req): Json<AddDomain>,
) -> Result<(StatusCode, Json<DomainView>), AppError> {
    let h = client_hosting(&state, &who).await?;
    let added = add_domain(
        &state,
        &who,
        &h,
        Wanted {
            fqdn: req.fqdn,
            kind: req.kind,
            parent_domain_id: req.parent_domain_id,
            docroot: req.docroot,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(added)))
}

/// Change the document root of one of the signed-in client's domains.
#[utoipa::path(
    patch,
    path = "/api/v1/client/domains/{id}",
    tag = "domains",
    operation_id = "clientUpdateDomain",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UpdateDomain,
    responses(
        (status = 200, description = "Updated", body = DomainView),
        (status = 403, description = "`client_only` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid document root", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_update(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateDomain>,
) -> Result<Json<DomainView>, AppError> {
    let h = client_hosting(&state, &who).await?;
    Ok(Json(change_domain(&state, &who, &h, id, req).await?))
}

/// Remove one of the signed-in client's domains and what hangs off it.
#[utoipa::path(
    delete,
    path = "/api/v1/client/domains/{id}",
    tag = "domains",
    operation_id = "clientDeleteDomain",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Domain id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`client_only` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such domain (or it is not yours)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`primary_has_domains`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_delete(
    State(state): State<DomainsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = client_hosting(&state, &who).await?;
    delete_domain(&state, &who, &h, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// OpenAPI fragment (full paths; merge into the root document).
#[derive(OpenApi)]
#[openapi(
    paths(
        admin_list, admin_get, admin_add, admin_update, admin_delete, admin_suspend,
        admin_unsuspend, client_list, client_get, client_add, client_update, client_delete,
        php::admin_get, php::admin_set, php::client_get, php::client_set_account,
        previews::list, previews::create, previews::remove, error_pages::list, error_pages::set, error_pages::reset, services::restart, php::client_set_domain, php_settings::admin_get, php_settings::admin_set,
        php_settings::client_get, php_settings::client_set,
        pagecache::admin_get, pagecache::admin_set, pagecache::admin_purge, pagecache::client_get, pagecache::client_set, pagecache::client_purge,
        policy::admin_get, policy::admin_set, policy::client_get, policy::client_set,
        vhost::admin_get, vhost::admin_set, vhost::client_get, vhost::client_set,
        vhost_templates::list, vhost_templates::show, vhost_templates::create,
        vhost_templates::update, vhost_templates::remove, vhost_templates::versions,
        vhost_templates::set_default, vhost_templates::preview, vhost_templates::apply,
        redirects::admin_list, redirects::admin_add, redirects::admin_update,
        redirects::admin_delete, redirects::client_list, redirects::client_add,
        redirects::client_update, redirects::client_delete,
        dns::list_templates, dns::get_template, dns::create_template, dns::update_template,
        dns::delete_template, records::admin_list, records::admin_add, records::admin_update,
        records::admin_delete, records::client_list, records::client_add, records::client_update,
        records::client_delete, nameservers::admin_nameservers, nameservers::client_nameservers,
        nameservers::admin_check, nameservers::client_check, tls::ask, ssl::admin_ssl,
        ssl::client_ssl, ssl::admin_upload, ssl::admin_remove, ssl::client_upload,
        ssl::client_remove
    ),
    components(schemas(
        DomainView, DomainList, AddDomain, AdminAddDomain, UpdateDomain,
        previews::PreviewView, previews::PreviewList, error_pages::PageView, error_pages::PageList, error_pages::PageInput, services::Restarted, php::PhpView, php::DomainPhp, php::SetVersion, php_settings::PhpSettingsView,
        php_settings::OptionView, php_settings::CapsView, php_settings::SetPhpSettings, policy::PolicyView, policy::HeaderView, pagecache::PageCacheView, pagecache::SetPageCache, vhost::VhostView, vhost::SetVhost, vhost_templates::TemplateSummary,
        vhost_templates::TemplateView, vhost_templates::TemplateList, vhost_templates::TemplateInput,
        vhost_templates::VersionView, vhost_templates::VersionList, vhost_templates::PreviewInput,
        vhost_templates::Preview, vhost_templates::Applied, redirects::RedirectView, redirects::RedirectList, redirects::AddRedirect,
        redirects::UpdateRedirect, dns::TemplateLine, dns::DnsTemplateView, dns::DnsTemplateList,
        dns::DnsTemplateInput, records::RecordView, records::RecordList, records::ZoneView,
        records::AddRecord, records::UpdateRecord, nameservers::Nameservers,
        nameservers::DelegationView, ssl::SslView, ssl::UploadCertificate
    )),
    tags((name = "domains", description = "The names a hosting account serves"))
)]
pub struct DomainsApi;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_limits_use_the_stored_number_the_default_or_unlimited() {
        let plan = |limits: serde_json::Value| Plan {
            id: Uuid::nil(),
            name: String::new(),
            description: String::new(),
            feature_set_id: Uuid::nil(),
            feature_set_name: String::new(),
            is_default: false,
            cpu_millis: None,
            memory_bytes: None,
            pids: None,
            disk_bytes: None,
            inodes: None,
            bandwidth_bytes: None,
            limits,
            isolation: "standard".into(),
            built_in: false,
            users: 0,
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(
            plan_limit(
                &plan(serde_json::json!({"max_domains": 3})),
                "max_domains",
                10
            ),
            Some(3)
        );
        assert_eq!(
            plan_limit(
                &plan(serde_json::json!({"max_domains": 0})),
                "max_domains",
                10
            ),
            Some(0)
        );
        assert_eq!(
            plan_limit(
                &plan(serde_json::json!({"max_domains": -1})),
                "max_domains",
                10
            ),
            None
        );
        assert_eq!(
            plan_limit(&plan(serde_json::json!({})), "max_domains", 10),
            Some(10),
            "a plan without the key gets the default"
        );
    }
}
