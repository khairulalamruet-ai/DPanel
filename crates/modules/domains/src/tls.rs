//! The on-demand TLS `ask` endpoint (T05.15).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/public/tls/ask?domain=<name>` | nothing (limited per address) |
//!
//! The web server on a node (Caddy) gets a certificate the first time a visitor
//! asks for a name, but only after asking here whether the name may have one.
//! The answer is `200` for a domain that belongs to a hosting account that is
//! running (or suspended: its page is shown over HTTPS too) and is itself
//! active; `404` for every other name, so a stranger who points a name at a
//! node cannot make it request certificates for it. `400` for something that is
//! not a host name, `429` when an address asks too often.
//!
//! The endpoint tells nothing the DNS does not: that a name is served by this
//! panel is visible to anyone who looks it up. It carries no body and no
//! detail.

use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    routing::get,
};
use builtpanel_database::domains;
use builtpanel_domain::{DomainError, Fqdn};
use builtpanel_http_kit::{AppError, ProblemDetails};
use serde::Deserialize;
use utoipa::IntoParams;

use super::{DomainsState, internal};

/// Most questions one address may ask per minute.
pub const ASK_LIMIT_PER_MINUTE: u32 = 600;

/// Routes without a session.
pub fn public_router() -> Router<DomainsState> {
    Router::new().route("/api/v1/public/tls/ask", get(ask))
}

/// The name the web server asks about.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AskQuery {
    /// The host name a visitor asked for.
    pub domain: String,
}

/// May a certificate be issued for this name?
#[utoipa::path(
    get,
    path = "/api/v1/public/tls/ask",
    tag = "domains",
    operation_id = "tlsAsk",
    params(AskQuery),
    responses(
        (status = 200, description = "The name is an active domain of this panel: a certificate may be issued"),
        (status = 400, description = "Not a host name", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "Not a domain of this panel, or not active: no certificate"),
        (status = 429, description = "This address asks too often", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn ask(
    State(state): State<DomainsState>,
    Query(q): Query<AskQuery>,
) -> Result<StatusCode, AppError> {
    // Counted before anything else costs a query. Without a known address the
    // calls share one bucket.
    let subject = builtpanel_http_kit::client_ip::current().map_or_else(
        || "unknown".to_owned(),
        builtpanel_http_kit::client_ip::rate_limit_key,
    );
    if !state.ask_limiter.hit(&subject).await.map_err(internal)? {
        return Err(AppError::too_many_requests(
            "too_many_requests",
            "Too many questions from this address.",
            std::time::Duration::from_secs(60),
        ));
    }
    let name = match Fqdn::parse(&q.domain) {
        Ok(name) => name,
        // A public suffix or an unknown ending is a fine host name, just nobody's here.
        Err(DomainError::NotRegistrable | DomainError::UnknownSuffix | DomainError::Blocked) => {
            return Ok(StatusCode::NOT_FOUND);
        }
        Err(_) => {
            return Err(AppError::bad_request(
                "invalid_domain",
                "This is not a host name.",
            ));
        }
    };
    if domains::certificate_allowed(&state.db, name.as_str())
        .await
        .map_err(internal)?
    {
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}
