//! The signed-in account's own settings: its language and the menu entries it pinned.
//!
//! | Method | Path | Effect |
//! |---|---|---|
//! | GET | `/api/v1/auth/preferences` | the saved settings (the defaults when nothing is saved) |
//! | PUT | `/api/v1/auth/preferences` | replaces them |
//!
//! Every account (administrator, reseller, client) has only its own; nobody can read or
//! change another account's. A pinned entry is the key the web interface gives a menu item
//! (e.g. `files`, `domains/dns`); the panel keeps the list without interpreting it.

use axum::{Json, Router, extract::State, routing::get};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_session::Authenticated;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AuthState;

/// Most pinned entries.
pub const MAX_FAVORITES: usize = 30;
/// Longest key of a pinned entry.
const MAX_KEY: usize = 64;

pub(crate) fn router() -> Router<AuthState> {
    Router::new().route("/api/v1/auth/preferences", get(read).put(replace))
}

/// An account's settings.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    /// `en` or `bn` (`null`: none chosen, the interface decides).
    #[schema(example = "bn")]
    pub locale: Option<String>,
    /// Pinned menu entries, in the order the account wants them.
    #[schema(example = json!(["files", "domains"]))]
    pub favorites: Vec<String>,
}

fn validate(p: &Preferences) -> Result<(), AppError> {
    let mut errors = Vec::new();
    if let Some(locale) = &p.locale
        && !matches!(locale.as_str(), "en" | "bn")
    {
        errors.push(FieldError::new("locale", "invalid", "`en` or `bn`."));
    }
    if p.favorites.len() > MAX_FAVORITES {
        errors.push(FieldError::new(
            "favorites",
            "too_many",
            "At most 30 pinned entries.",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for key in &p.favorites {
        let plain = !key.is_empty()
            && key.len() <= MAX_KEY
            && key.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || matches!(b, b'_' | b'-' | b'/' | b'.')
            });
        if !plain {
            errors.push(FieldError::new(
                "favorites",
                "invalid",
                "Each entry is 1-64 of a-z, 0-9, `_`, `-`, `/` and `.`.",
            ));
            break;
        }
        if !seen.insert(key) {
            errors.push(FieldError::new("favorites", "duplicate", "An entry twice."));
            break;
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::validation(errors))
    }
}

/// The saved settings.
#[utoipa::path(
    get,
    path = "/api/v1/auth/preferences",
    tag = "auth",
    operation_id = "getPreferences",
    security(("session" = [])),
    responses(
        (status = 200, description = "The settings", body = Preferences),
        (status = 401, description = "Not logged in", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn read(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
) -> Result<Json<Preferences>, AppError> {
    let row: Option<(Option<String>, Vec<String>)> = builtpanel_database::sqlx::query_as(
        "SELECT locale, favorites FROM account_preferences WHERE account_id = $1",
    )
    .bind(session.account_id)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::internal)?;
    let (locale, favorites) = row.unwrap_or((None, Vec::new()));
    Ok(Json(Preferences { locale, favorites }))
}

/// Replace the settings.
#[utoipa::path(
    put,
    path = "/api/v1/auth/preferences",
    tag = "auth",
    operation_id = "putPreferences",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = Preferences,
    responses(
        (status = 200, description = "Saved", body = Preferences),
        (status = 401, description = "Not logged in", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A language or entry that is not usable", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn replace(
    State(state): State<AuthState>,
    Authenticated(session): Authenticated,
    Json(body): Json<Preferences>,
) -> Result<Json<Preferences>, AppError> {
    validate(&body)?;
    builtpanel_database::sqlx::query(
        "INSERT INTO account_preferences (account_id, locale, favorites) VALUES ($1, $2, $3)
         ON CONFLICT (account_id) DO UPDATE
         SET locale = EXCLUDED.locale, favorites = EXCLUDED.favorites, updated_at = now()",
    )
    .bind(session.account_id)
    .bind(&body.locale)
    .bind(&body.favorites)
    .execute(&state.db)
    .await
    .map_err(AppError::internal)?;
    Ok(Json(body))
}
