//! The search of the command palette: domains and accounts that match a few letters.
//!
//! An administrator finds every domain and account; a customer finds only the
//! domains of their own tenant. Matching is a case-insensitive "contains".

use axum::{
    Json,
    extract::{Query, State},
};
use builtpanel_database::{
    accounts::{self, DbAccountKind},
    sqlx, users,
};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_session::Authenticated;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::SystemState;

/// What is searched for.
#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    /// A few letters of a domain name, a username or an email address (two at least).
    pub q: String,
}

/// One match.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchHit {
    /// Id of the thing found.
    pub id: String,
    /// What to show.
    pub title: String,
    /// A second line: the owner of a domain, the email of an account.
    pub subtitle: Option<String>,
    /// `domains` or `users`.
    pub category: &'static str,
    /// Where it is in the interface.
    pub href: String,
}

/// Find domains and accounts by a few letters.
#[utoipa::path(
    get,
    path = "/api/v1/search",
    tag = "system",
    params(SearchQuery),
    responses(
        (status = 200, description = "At most ten matches of each kind; none when fewer than two letters are given", body = Vec<SearchHit>),
        (status = 401, description = "Not signed in", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn search(
    State(state): State<SystemState>,
    Authenticated(session): Authenticated,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<SearchHit>>, AppError> {
    let needle = query.q.trim().to_lowercase();
    if needle.chars().count() < 2 {
        return Ok(Json(Vec::new()));
    }
    // `%` and `_` in what was typed must match themselves.
    let pattern = format!(
        "%{}%",
        needle
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let profile = accounts::find_profile(&state.db, session.account_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;

    let mut hits = Vec::new();
    match profile.kind {
        DbAccountKind::Admin => {
            let domains: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
                "SELECT d.id, d.fqdn, a.username FROM domains d \
                 JOIN tenants t ON t.id = d.tenant_id \
                 JOIN accounts a ON a.id = t.owner_account_id \
                 WHERE d.fqdn LIKE $1 ESCAPE '\\' ORDER BY d.fqdn LIMIT 10",
            )
            .bind(&pattern)
            .fetch_all(&state.db)
            .await
            .map_err(AppError::internal)?;
            for (id, fqdn, owner) in domains {
                hits.push(SearchHit {
                    id: id.to_string(),
                    title: fqdn,
                    subtitle: Some(owner),
                    category: "domains",
                    href: "/domains".to_owned(),
                });
            }
            let accounts: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
                "SELECT id, username, email FROM accounts \
                 WHERE username LIKE $1 ESCAPE '\\' OR email LIKE $1 ESCAPE '\\' \
                 ORDER BY username LIMIT 10",
            )
            .bind(&pattern)
            .fetch_all(&state.db)
            .await
            .map_err(AppError::internal)?;
            for (id, username, email) in accounts {
                hits.push(SearchHit {
                    id: id.to_string(),
                    title: username,
                    subtitle: Some(email),
                    category: "users",
                    href: format!("/users/{id}"),
                });
            }
        }
        _ => {
            let tenant = users::get(&state.db, session.account_id)
                .await
                .map_err(AppError::internal)?
                .and_then(|u| u.tenant);
            if let Some(tenant) = tenant {
                let domains: Vec<(uuid::Uuid, String)> = sqlx::query_as(
                    "SELECT id, fqdn FROM domains \
                     WHERE tenant_id = $1 AND fqdn LIKE $2 ESCAPE '\\' ORDER BY fqdn LIMIT 10",
                )
                .bind(tenant.id)
                .bind(&pattern)
                .fetch_all(&state.db)
                .await
                .map_err(AppError::internal)?;
                for (id, fqdn) in domains {
                    hits.push(SearchHit {
                        id: id.to_string(),
                        title: fqdn,
                        subtitle: None,
                        category: "domains",
                        href: "/domains".to_owned(),
                    });
                }
            }
        }
    }
    Ok(Json(hits))
}
