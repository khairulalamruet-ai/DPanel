//! OpenAPI document for the control plane.
//!
//! * Served at `GET /api/v1/openapi.json`.
//! * Committed as `openapi.json` at the repository root; the frontend generates
//!   its types from that file. The `openapi_json_is_up_to_date` test fails when
//!   the committed file drifts; regenerate with
//!   `BUILTPANEL_UPDATE_OPENAPI=1 cargo test -p builtpanel-control-api openapi_json`.
//! * Debug builds also serve an interactive viewer at `/api/docs`.

use builtpanel_http_kit::{FieldError, ProblemDetails, health::PublicHealthApi};
use utoipa::OpenApi;

/// Root OpenAPI document. Each module nests its own fragment here.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Built Panel API",
        description = "Control plane API for the Built Panel hosting panel. \
                       Errors use RFC 9457 Problem Details (`application/problem+json`).",
        license(name = "Proprietary")
    ),
    servers((url = "/", description = "Same origin as the panel")),
    nest((path = "/api/v1/public", api = PublicHealthApi)),
    components(schemas(ProblemDetails, FieldError))
)]
pub struct ApiDoc;

/// The full document: root info plus every module's fragment.
#[must_use]
pub fn document() -> utoipa::openapi::OpenApi {
    let mut doc = ApiDoc::openapi();
    doc.merge(builtpanel_mod_auth::AuthApi::openapi());
    doc.merge(builtpanel_audit::AuditApi::openapi());
    doc.merge(builtpanel_mod_admin::AdminApi::openapi());
    doc.merge(builtpanel_mod_nodes::NodesApi::openapi());
    doc.merge(builtpanel_mod_system::SystemApi::openapi());
    doc.merge(builtpanel_mod_plans::PlansApi::openapi());
    doc.merge(builtpanel_mod_users::UsersApi::openapi());
    doc.merge(builtpanel_mod_users::ResellersApi::openapi());
    doc.merge(builtpanel_mod_domains::DomainsApi::openapi());
    doc.merge(builtpanel_mod_databases::DatabasesApi::openapi());
    doc.merge(builtpanel_mod_files::FilesApi::openapi());
    doc.merge(builtpanel_mod_backups::BackupsApi::openapi());
    doc.merge(builtpanel_mod_apps::AppsApi::openapi());
    doc.merge(builtpanel_mod_license::LicenseApi::openapi());
    doc.merge(builtpanel_mod_cron::CronApi::openapi());
    doc.merge(builtpanel_mod_mail::MailApi::openapi());
    doc.merge(builtpanel_mod_migration::MigrationApi::openapi());
    doc.merge(builtpanel_mod_logs::LogsApi::openapi());
    doc.merge(builtpanel_mod_notifications::NotificationsApi::openapi());
    doc.merge(builtpanel_mod_usage::UsageApi::openapi());
    doc.merge(builtpanel_mod_services::ServicesApi::openapi());
    doc.merge(builtpanel_mod_security::SecurityApi::openapi());
    doc
}

/// The document as pretty JSON with a trailing newline (stable for diffs).
#[must_use]
pub fn json() -> String {
    let mut out = document()
        .to_pretty_json()
        .unwrap_or_else(|e| format!("{{\"error\":\"openapi serialization failed: {e}\"}}"));
    out.push('\n');
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn committed_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../openapi.json")
    }

    #[test]
    fn openapi_json_is_up_to_date() {
        let generated = json();
        let path = committed_path();
        if std::env::var_os("BUILTPANEL_UPDATE_OPENAPI").is_some() {
            std::fs::write(&path, &generated).expect("write openapi.json");
            return;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == generated,
            "openapi.json is stale; run: BUILTPANEL_UPDATE_OPENAPI=1 cargo test -p builtpanel-control-api openapi_json"
        );
    }

    #[test]
    fn document_lists_public_health_paths_and_problem_schema() {
        let doc: serde_json::Value = serde_json::from_str(&json()).expect("valid json");
        assert!(
            doc["openapi"]
                .as_str()
                .unwrap_or_default()
                .starts_with("3.")
        );
        assert!(doc["paths"]["/api/v1/public/health"]["get"].is_object());
        assert!(doc["paths"]["/api/v1/public/ready"]["get"]["responses"]["503"].is_object());
        assert!(doc["components"]["schemas"]["ProblemDetails"].is_object());
        assert!(doc["components"]["schemas"]["ReadinessReport"].is_object());
        // `/metrics` is operational, not part of the public API contract.
        assert!(doc["paths"]["/metrics"].is_null());
    }
}
