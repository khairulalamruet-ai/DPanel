//! `GET /api/v1/admin/bootstrap` (T03.10) and `GET /api/v1/client/bootstrap`:
//! everything an interface needs to draw itself after sign-in, in one request.
//!
//! The answer is built from three sources, only one of which is the database
//! per request:
//! * the caller: profile, roles and permissions (two small queries);
//! * the modules that are being served right now, with their menu entries:
//!   from memory (the module mount and the manifests), no query;
//! * the panel's look: name and language from the settings, kept in memory for
//!   a few seconds and dropped at once when they are changed here.
//!
//! The answer is never cached by the browser or a proxy (`no-store`): it holds
//! the CSRF token.

use std::{
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse as _, Response},
};
use builtpanel_database::{
    accounts::{self, DbAccountKind},
    plans,
    rbac::load_access,
    tenants, users,
};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_session::Authenticated;
use builtpanel_settings::Scope;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::SystemState;

/// How long the panel's look is kept in memory.
pub const BRANDING_TTL: Duration = Duration::from_secs(5);

/// The panel's look, from the `system` settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Branding {
    /// Name shown on the sign-in page and in the browser tab.
    #[schema(example = "Built Panel")]
    pub panel_name: String,
    /// A notice for all users while it is not empty.
    pub maintenance_notice: String,
    /// Language used when the account has no preference (`en` or `bn`).
    #[schema(example = "en")]
    pub default_language: String,
}

impl Default for Branding {
    fn default() -> Self {
        Self {
            panel_name: "Built Panel".to_owned(),
            maintenance_notice: String::new(),
            default_language: "en".to_owned(),
        }
    }
}

/// The branding, remembered for [`BRANDING_TTL`].
#[derive(Default)]
pub struct BrandingCache {
    slot: Mutex<Option<(Instant, Branding)>>,
}

impl BrandingCache {
    /// The remembered branding if it is fresh.
    fn fresh(&self) -> Option<Branding> {
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|(at, _)| at.elapsed() < BRANDING_TTL)
            .map(|(_, branding)| branding.clone())
    }

    fn store(&self, branding: &Branding) {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((Instant::now(), branding.clone()));
    }

    /// Forgets it; the next request reads the settings again.
    pub fn invalidate(&self) {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

async fn branding(state: &SystemState) -> Result<Branding, AppError> {
    if let Some(branding) = state.branding.fresh() {
        return Ok(branding);
    }
    let mut conn = state.db.acquire().await.map_err(AppError::internal)?;
    let values =
        match builtpanel_settings::get(&state.settings, &mut conn, "system", Scope::Global).await {
            Ok(values) => values,
            // A build without the schema falls back to the defaults rather than failing sign-in.
            Err(builtpanel_settings::SettingsError::UnknownModule(_)) => Default::default(),
            Err(err) => return Err(AppError::internal(std::io::Error::other(err.to_string()))),
        };
    let text = |key: &str, fallback: &str| {
        values
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or(fallback)
            .to_owned()
    };
    let defaults = Branding::default();
    let branding = Branding {
        panel_name: text("panel_name", &defaults.panel_name),
        maintenance_notice: text("maintenance_notice", &defaults.maintenance_notice),
        default_language: text("default_language", &defaults.default_language),
    };
    state.branding.store(&branding);
    Ok(branding)
}

/// The signed-in account.
#[derive(Debug, Serialize, ToSchema)]
pub struct BootstrapUser {
    /// Account id.
    pub id: Uuid,
    /// Login name.
    pub username: String,
    /// E-mail address.
    pub email: String,
    /// `admin`, `reseller` or `client`.
    pub kind: String,
}

/// Two-factor authentication state.
#[derive(Debug, Serialize, ToSchema)]
pub struct BootstrapMfa {
    /// A second factor is set up.
    pub enabled: bool,
    /// An administrator made it mandatory.
    pub required: bool,
    /// Mandatory but not set up: the interface should send the user to the
    /// security page; protected endpoints answer 403 `mfa_setup_required`.
    pub setup_required: bool,
}

/// A menu entry.
#[derive(Debug, Serialize, ToSchema)]
pub struct BootstrapNav {
    /// Menu group.
    pub group: String,
    /// Position in the group, lower first.
    pub order: u32,
    /// Icon name.
    pub icon: String,
}

/// A module that is being served.
#[derive(Debug, Serialize, ToSchema)]
pub struct BootstrapModule {
    /// Name; also the feature key of its interface module.
    pub name: String,
    /// Version of the build.
    pub version: String,
    /// Display name by language code.
    #[schema(value_type = Object)]
    pub title: std::collections::BTreeMap<String, String>,
    /// Category.
    pub category: String,
    /// Cannot be switched off.
    pub core: bool,
    /// Entry in the menu of whoever asked, if it has one.
    pub nav: Option<BootstrapNav>,
}

/// What the admin interface needs to start.
#[derive(Debug, Serialize, ToSchema)]
pub struct Bootstrap {
    /// The signed-in account.
    pub user: BootstrapUser,
    /// The language to use: the account's, else the panel's default.
    #[schema(example = "bn")]
    pub locale: String,
    /// Role names.
    pub roles: Vec<String>,
    /// Permission grants: exact keys, subtrees (`admin.*`) or `*`. For showing
    /// or hiding things; the server checks every request.
    pub permissions: Vec<String>,
    /// Two-factor authentication.
    pub mfa: BootstrapMfa,
    /// Modules being served right now, in dependency order.
    pub modules: Vec<BootstrapModule>,
    /// Plan features (the customer bootstrap only; administrators have no plan).
    pub features: Vec<String>,
    /// Plan limits by key, `-1` for no limit (the customer bootstrap only).
    #[schema(value_type = Object)]
    pub limits: std::collections::BTreeMap<String, i64>,
    /// The panel's look.
    pub branding: Branding,
    /// Send as `X-CSRF-Token` on POST/PUT/PATCH/DELETE. Keep it in memory only.
    pub csrf_token: String,
}

/// Everything the admin interface needs after sign-in.
#[utoipa::path(
    get,
    path = "/api/v1/admin/bootstrap",
    tag = "system",
    operation_id = "adminBootstrap",
    security(("session" = [])),
    responses(
        (status = 200, description = "The caller, their permissions, the modules being served, branding and the CSRF token. Never cached.", body = Bootstrap),
        (status = 401, description = "Not signed in", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "`not_an_administrator`: customer accounts use the customer bootstrap", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn admin_bootstrap(
    State(state): State<SystemState>,
    Authenticated(session): Authenticated,
) -> Result<Response, AppError> {
    build(&state, &session, Audience::Admin).await
}

/// Everything the customer interface needs after sign-in: the same as the
/// administrators' bootstrap, with the plan's features and limits.
#[utoipa::path(
    get,
    path = "/api/v1/client/bootstrap",
    tag = "system",
    operation_id = "clientBootstrap",
    security(("session" = [])),
    responses(
        (status = 200, description = "The caller, their permissions, the modules with a customer menu entry, the plan's features and limits, branding and the CSRF token. Never cached.", body = Bootstrap),
        (status = 401, description = "Not signed in", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "`not_a_customer`: administrators use the administrators' bootstrap", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn client_bootstrap(
    State(state): State<SystemState>,
    Authenticated(session): Authenticated,
) -> Result<Response, AppError> {
    build(&state, &session, Audience::Client).await
}

/// Whose interface is being started.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Audience {
    Admin,
    Client,
}

/// The feature keys of a plan; a plan with every feature gets all that the served modules declare.
async fn plan_features(
    state: &SystemState,
    plan_id: Uuid,
    served: &[String],
) -> Result<Vec<String>, AppError> {
    let row: Option<(bool, Vec<String>)> = builtpanel_database::sqlx::query_as(
        "SELECT f.all_features, f.features::text[] \
         FROM plans p JOIN feature_sets f ON f.id = p.feature_set_id WHERE p.id = $1",
    )
    .bind(plan_id)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::internal)?;
    Ok(match row {
        Some((true, _)) => served.to_vec(),
        Some((false, features)) => features,
        None => Vec::new(),
    })
}

/// The numeric limits of a plan, `-1` for no limit.
fn plan_limits(plan: &builtpanel_database::plans::Plan) -> std::collections::BTreeMap<String, i64> {
    plan.limits
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| value.as_i64().map(|n| (key.clone(), n)))
                .collect()
        })
        .unwrap_or_default()
}

async fn build(
    state: &SystemState,
    session: &builtpanel_session::SessionData,
    audience: Audience,
) -> Result<Response, AppError> {
    let account = session.account_id;
    let (profile, access, branding) = tokio::join!(
        accounts::find_profile(&state.db, account),
        load_access(&state.db, account),
        branding(state),
    );
    let (profile, access) = (
        profile.map_err(AppError::internal)?,
        access.map_err(AppError::internal)?,
    );
    let branding = branding?;
    let (Some(profile), Some(access)) = (profile, access) else {
        return Err(AppError::Unauthorized);
    };
    if access.status != builtpanel_database::accounts::DbAccountStatus::Active {
        return Err(AppError::Unauthorized);
    }
    let kind = match profile.kind {
        DbAccountKind::Admin => "admin",
        DbAccountKind::Reseller => "reseller",
        DbAccountKind::Client => "client",
    };
    match audience {
        Audience::Admin if kind != "admin" => {
            return Err(AppError::forbidden(
                "not_an_administrator",
                "This is the administrators' bootstrap.",
            ));
        }
        Audience::Client if kind != "client" => {
            return Err(AppError::forbidden(
                "not_a_customer",
                "This is the customers' bootstrap.",
            ));
        }
        _ => {}
    }

    let mounted = state.mount.mounted();
    let modules: Vec<BootstrapModule> = state
        .mount
        .registry()
        .modules()
        .iter()
        .filter(|m| mounted.contains(&m.manifest().module.name))
        .filter_map(|m| {
            let manifest = m.manifest();
            let nav = match audience {
                Audience::Admin => manifest.ui.admin_nav.as_ref(),
                Audience::Client => manifest.ui.client_nav.as_ref(),
            };
            // The customers' menu lists only what has an entry in it.
            if audience == Audience::Client && nav.is_none() {
                return None;
            }
            Some(BootstrapModule {
                name: manifest.module.name.clone(),
                version: manifest.module.version.clone(),
                title: manifest.module.title.clone(),
                category: manifest.module.category.as_str().to_owned(),
                core: manifest.module.core,
                nav: nav.map(|nav| BootstrapNav {
                    group: nav.group.clone(),
                    order: nav.order,
                    icon: nav.icon.clone(),
                }),
            })
        })
        .collect();

    let (features, limits) = if audience == Audience::Client {
        let user = users::get(&state.db, account)
            .await
            .map_err(AppError::internal)?;
        let tenant = match user.and_then(|u| u.tenant) {
            Some(owned) => tenants::get(&state.db, owned.id)
                .await
                .map_err(AppError::internal)?,
            None => None,
        };
        match tenant {
            Some(tenant) => {
                let plan = plans::get(&state.db, tenant.plan_id)
                    .await
                    .map_err(AppError::internal)?;
                // What "every feature" means: the features the served modules declare.
                let served: Vec<String> = state
                    .mount
                    .registry()
                    .modules()
                    .iter()
                    .filter(|m| mounted.contains(&m.manifest().module.name))
                    .flat_map(|m| m.manifest().features.keys.iter().cloned())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                match plan {
                    Some(plan) => (
                        plan_features(state, plan.id, &served).await?,
                        plan_limits(&plan),
                    ),
                    None => (Vec::new(), Default::default()),
                }
            }
            // A customer account whose hosting is not made yet has no plan to read.
            None => (Vec::new(), Default::default()),
        }
    } else {
        (Vec::new(), Default::default())
    };

    let locale = if profile.locale.is_empty() {
        branding.default_language.clone()
    } else {
        profile.locale
    };
    let body = Bootstrap {
        user: BootstrapUser {
            id: profile.id,
            username: profile.username,
            email: profile.email,
            kind: kind.to_owned(),
        },
        locale,
        roles: access.roles,
        permissions: access.permissions,
        mfa: BootstrapMfa {
            enabled: access.mfa_enabled,
            required: access.mfa_required,
            setup_required: access.mfa_required && !access.mfa_enabled,
        },
        modules,
        features,
        limits,
        branding,
        csrf_token: session.csrf_token.expose().to_owned(),
    };
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_look_is_remembered_until_it_is_dropped() {
        let cache = BrandingCache::default();
        assert_eq!(cache.fresh(), None);
        let custom = Branding {
            panel_name: "My Hosting".into(),
            ..Branding::default()
        };
        cache.store(&custom);
        assert_eq!(cache.fresh(), Some(custom));
        cache.invalidate();
        assert_eq!(cache.fresh(), None, "a change made here is seen at once");
        // An old entry is not served.
        *cache.slot.lock().unwrap() =
            Some((Instant::now() - BRANDING_TTL * 2, Branding::default()));
        assert_eq!(cache.fresh(), None);
    }
}
