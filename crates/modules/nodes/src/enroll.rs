//! `POST /api/v1/public/nodes/enroll`: an agent trades its one-time token and a
//! certificate signing request for a node certificate.
//!
//! This endpoint is open to the world, so it is built defensively:
//!
//! * **The token is the only credential.** Unknown, malformed, already used,
//!   cancelled and expired tokens all answer the same 401, so nothing can be
//!   learned about which tokens exist.
//! * **Failures are counted per client address** (an IPv6 /64) in Redis; after
//!   [`crate::ENROLL_MAX_FAILURES`] the address gets 429 with `Retry-After`
//!   *before any work is done*.
//! * **The token is used up only if enrolment succeeds.** Certificate request
//!   and fields are checked first; then token, node row, certificate record and
//!   audit entry are written in one transaction. A bad request or a name clash
//!   leaves the token usable.
//! * **Only the public key from the CSR is used** (see `builtpanel-pki`): the node
//!   cannot choose its identity, roles or lifetime.
//! * Input sizes are bounded; the token is never logged or audited.

use axum::{
    Json,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    node_enrollments::{attach_node, consume_token},
    nodes::{InsertNodeError, NewNode, NewNodeCertificate, insert_node, insert_node_certificate},
};
use builtpanel_http_kit::{
    AppError, FieldError, ProblemDetails,
    client_ip::{self, rate_limit_key},
};
use builtpanel_node_protocol::PROTOCOL_VERSION;
pub use builtpanel_node_protocol::{EnrollRequest, EnrollResponse};
use builtpanel_pki::{DEFAULT_NODE_VALIDITY, PkiError, format_rfc3339};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{NodesState, token};

/// Largest accepted certificate signing request, in bytes.
pub const MAX_CSR_BYTES: usize = 8 * 1024;
/// Largest accepted token field, in bytes (a real token is 47).
const MAX_TOKEN_BYTES: usize = 256;

fn invalid_token() -> AppError {
    AppError::authentication_failed(
        "invalid_enrollment_token",
        "The enrollment token is not valid. Create a new one in the panel.",
    )
}

fn plausible_hostname(s: &str) -> bool {
    (1..=253).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

fn plausible_version(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic())
}

/// Checks the descriptive fields; returns them trimmed, `None` when empty.
fn clean_fields(req: &EnrollRequest) -> Result<(Option<String>, Option<String>), AppError> {
    let mut errors = Vec::new();
    let hostname = req
        .hostname
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if hostname.is_some_and(|h| !plausible_hostname(h)) {
        errors.push(FieldError::new(
            "hostname",
            "invalid_format",
            "Use letters, digits, '.' and '-' (at most 253 characters).",
        ));
    }
    let version = req
        .agent_version
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if version.is_some_and(|v| !plausible_version(v)) {
        errors.push(FieldError::new(
            "agent_version",
            "invalid_format",
            "Use printable characters without spaces (at most 64).",
        ));
    }
    if req.csr_pem.len() > MAX_CSR_BYTES {
        errors.push(FieldError::new(
            "csr_pem",
            "too_long",
            "The certificate request is too large.",
        ));
    }
    if errors.is_empty() {
        Ok((hostname.map(str::to_owned), version.map(str::to_owned)))
    } else {
        Err(AppError::validation(errors))
    }
}

/// Counts a failed attempt and, only for the attempt that trips the limit,
/// logs it. Counting trouble must not turn a 401 into a 500.
async fn count_failure(state: &NodesState, subject: &str) {
    match state.limiter.record_failure(subject).await {
        Ok(r) if r.locked => {
            tracing::warn!(
                subject,
                "node enrollment locked out after repeated failures"
            );
        }
        Ok(_) => {}
        Err(err) => tracing::error!(error = %err, "enrollment failure not counted"),
    }
}

async fn audit_failure(state: &NodesState, reason: &'static str) {
    state
        .audit
        .record_or_log(
            AuditEntry::new("node.enrollment_failed", Outcome::Denied)
                .metadata(serde_json::json!({ "reason": reason })),
        )
        .await;
}

/// Exchange an enrollment token and a certificate request for a node certificate.
#[utoipa::path(
    post,
    path = "/api/v1/public/nodes/enroll",
    tag = "nodes",
    operation_id = "enrollNode",
    request_body = EnrollRequest,
    responses(
        (status = 201, description = "Enrolled; the token is used up", body = EnrollResponse),
        (status = 401, description = "`invalid_enrollment_token`: unknown, used, cancelled or expired (deliberately indistinguishable)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`node_name_taken`: a node with that name appeared meanwhile; the token is still usable once the name is free", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid certificate request or fields; the token is not used up", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many failed attempts from this address", body = ProblemDetails, content_type = "application/problem+json",
         headers(("retry-after" = u64, description = "Seconds until the lock expires"))),
        (status = 503, description = "`enrollment_unavailable`: this control plane has no node CA yet (run `bpctl ca init`)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn enroll_node(
    State(state): State<NodesState>,
    Json(req): Json<EnrollRequest>,
) -> Result<Response, AppError> {
    let Some(ca) = state.ca.clone() else {
        return Err(AppError::unavailable(
            "enrollment_unavailable",
            "Node enrollment is not set up on this control plane.",
        ));
    };
    let ip = client_ip::current();
    let subject = ip.map_or_else(|| "unknown".to_owned(), rate_limit_key);

    // Locked-out addresses get nothing, not even a look at their input.
    if let Some(left) = state
        .limiter
        .locked_for(&subject)
        .await
        .map_err(AppError::internal)?
    {
        return Err(AppError::too_many_requests(
            "too_many_attempts",
            "Too many failed enrollment attempts. Try again later.",
            left,
        ));
    }

    let Some(hash) = (req.token.len() <= MAX_TOKEN_BYTES)
        .then(|| token::hash(req.token.trim()))
        .flatten()
    else {
        count_failure(&state, &subject).await;
        audit_failure(&state, "malformed_token").await;
        return Err(invalid_token());
    };
    let (hostname, agent_version) = match clean_fields(&req) {
        Ok(fields) => fields,
        Err(err) => {
            count_failure(&state, &subject).await;
            return Err(err);
        }
    };

    // Check the request and make the certificate before touching the token, so
    // a malformed request does not burn it. Nothing is stored or returned until
    // the transaction below commits.
    let node_id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    let issued = match ca.issue_node_certificate(&req.csr_pem, node_id, now, DEFAULT_NODE_VALIDITY)
    {
        Ok(issued) => issued,
        Err(err @ (PkiError::InvalidCsr(_) | PkiError::UnsupportedKey)) => {
            count_failure(&state, &subject).await;
            let (code, message) = if matches!(err, PkiError::UnsupportedKey) {
                (
                    "unsupported_key",
                    "The request must use an ECDSA P-256 key.",
                )
            } else {
                ("invalid_csr", "The certificate request is not valid.")
            };
            return Err(AppError::validation(vec![FieldError::new(
                "csr_pem", code, message,
            )]));
        }
        Err(err) => return Err(AppError::internal(err)),
    };

    let mut tx = state.db.begin().await.map_err(AppError::internal)?;
    let Some(consumed) = consume_token(&mut tx, &hash)
        .await
        .map_err(AppError::internal)?
    else {
        drop(tx);
        count_failure(&state, &subject).await;
        audit_failure(&state, "invalid_token").await;
        return Err(invalid_token());
    };
    insert_node(
        &mut tx,
        &NewNode {
            id: node_id,
            name: &consumed.name,
            hostname: hostname.as_deref(),
            public_ip: ip,
            agent_version: agent_version.as_deref(),
        },
    )
    .await
    .map_err(|e| match e {
        // Rolled back below, so the token stays usable.
        InsertNodeError::NameTaken => AppError::conflict(
            "node_name_taken",
            "A node with this name already exists. Remove it, or create a new token with another name.",
        ),
        InsertNodeError::Database(db) => AppError::internal(db),
    })?;
    insert_node_certificate(
        &mut tx,
        &NewNodeCertificate {
            node_id,
            serial: &issued.serial_hex,
            fingerprint_sha256: &issued.fingerprint_sha256,
            not_before: issued.not_before,
            not_after: issued.not_after,
        },
    )
    .await
    .map_err(AppError::internal)?;
    attach_node(&mut tx, consumed.id, node_id)
        .await
        .map_err(AppError::internal)?;
    let mut entry = AuditEntry::new("node.enrolled", Outcome::Success)
        .target("node", node_id)
        .metadata(serde_json::json!({
            "name": consumed.name,
            "token_id": consumed.id,
            "certificate_fingerprint": issued.fingerprint_sha256,
            "certificate_serial": issued.serial_hex,
        }));
    if let Some(ip) = ip {
        entry = entry.actor_ip(ip);
    }
    builtpanel_audit::insert(&mut *tx, &entry)
        .await
        .map_err(AppError::internal)?;
    tx.commit().await.map_err(AppError::internal)?;

    let body = EnrollResponse {
        node_id,
        name: consumed.name,
        certificate_pem: issued.cert_pem,
        ca_certificate_pem: ca.certificate_pem().to_owned(),
        ca_fingerprint_sha256: ca.fingerprint_sha256().to_owned(),
        not_before: format_rfc3339(issued.not_before),
        not_after: format_rfc3339(issued.not_after),
        protocol_version: PROTOCOL_VERSION,
        stream_url: state.stream_url.as_deref().map(str::to_owned),
    };
    let mut response = (StatusCode::CREATED, Json(body)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn request(hostname: Option<&str>, version: Option<&str>, csr: &str) -> EnrollRequest {
        EnrollRequest {
            token: "dhn_x".into(),
            csr_pem: csr.into(),
            hostname: hostname.map(str::to_owned),
            agent_version: version.map(str::to_owned),
        }
    }

    #[test]
    fn descriptive_fields_are_checked_and_trimmed() {
        let (h, v) = clean_fields(&request(
            Some(" node1.example.com "),
            Some("0.1.0-rc.1"),
            "x",
        ))
        .unwrap();
        assert_eq!(
            (h.as_deref(), v.as_deref()),
            (Some("node1.example.com"), Some("0.1.0-rc.1"))
        );
        let (h, v) = clean_fields(&request(Some("  "), None, "x")).unwrap();
        assert_eq!((h, v), (None, None));
        for bad_host in [
            "bad host",
            "a_b.example",
            "x;rm -rf",
            &"h".repeat(254),
            "höst",
        ] {
            assert!(
                clean_fields(&request(Some(bad_host), None, "x")).is_err(),
                "{bad_host:?}"
            );
        }
        for bad_version in ["has space", "tab\tx", &"v".repeat(65), "naïve"] {
            assert!(
                clean_fields(&request(None, Some(bad_version), "x")).is_err(),
                "{bad_version:?}"
            );
        }
        assert!(clean_fields(&request(None, None, &"c".repeat(MAX_CSR_BYTES + 1))).is_err());
    }
}
