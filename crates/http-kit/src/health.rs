//! Liveness and readiness endpoints.
//!
//! * Liveness (`/health`) only proves the process answers HTTP.
//! * Readiness (`/ready`) runs every registered [`Probe`] concurrently, each
//!   bounded by a timeout, and answers 503 if any fails.
//!
//! Both endpoints are unauthenticated, so the readiness body only reports
//! check names and `ok` / `failed` / `timeout`. Failure details are logged,
//! never returned, because they may contain hostnames or other internals.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;

/// Future returned by a probe. `Err` carries a log-only description.
pub type ProbeFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;

/// A readiness check, e.g. "can I run `SELECT 1` on PostgreSQL".
pub type Probe = Arc<dyn Fn() -> ProbeFuture + Send + Sync + 'static>;

/// Default per-probe timeout.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Registered readiness probes.
#[derive(Clone)]
pub struct Readiness {
    probes: Vec<(&'static str, Probe)>,
    timeout: Duration,
}

impl Default for Readiness {
    fn default() -> Self {
        Self::new(DEFAULT_PROBE_TIMEOUT)
    }
}

impl std::fmt::Debug for Readiness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Readiness")
            .field(
                "probes",
                &self.probes.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Outcome of one probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Probe succeeded.
    Ok,
    /// Probe returned an error.
    Failed,
    /// Probe did not finish within the timeout.
    Timeout,
}

/// Public result for one probe.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CheckResult {
    /// Probe name, e.g. `database`.
    pub name: &'static str,
    /// Outcome.
    pub status: CheckStatus,
    /// Time taken, in milliseconds.
    pub duration_ms: u64,
}

/// Body of `/ready`.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ReadinessReport {
    /// `ready` or `not_ready`.
    #[schema(example = "ready")]
    pub status: &'static str,
    /// One entry per registered probe, sorted by name.
    pub checks: Vec<CheckResult>,
}

impl ReadinessReport {
    /// True if every probe succeeded.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.status == "ready"
    }
}

impl Readiness {
    /// Empty set of probes with the given per-probe timeout.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            probes: Vec::new(),
            timeout,
        }
    }

    /// Adds a probe.
    #[must_use]
    pub fn with_probe(mut self, name: &'static str, probe: Probe) -> Self {
        self.probes.push((name, probe));
        self
    }

    /// Runs all probes concurrently and collects the results.
    pub async fn check(&self) -> ReadinessReport {
        let mut set = tokio::task::JoinSet::new();
        for (name, probe) in &self.probes {
            let fut = probe();
            let name = *name;
            let timeout = self.timeout;
            set.spawn(async move {
                let started = std::time::Instant::now();
                let status = match tokio::time::timeout(timeout, fut).await {
                    Ok(Ok(())) => CheckStatus::Ok,
                    Ok(Err(detail)) => {
                        tracing::warn!(check = name, error = %detail, "readiness check failed");
                        CheckStatus::Failed
                    }
                    Err(_) => {
                        tracing::warn!(
                            check = name,
                            timeout_ms = timeout.as_millis(),
                            "readiness check timed out"
                        );
                        CheckStatus::Timeout
                    }
                };
                CheckResult {
                    name,
                    status,
                    duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                }
            });
        }

        let mut checks = Vec::with_capacity(self.probes.len());
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(result) => checks.push(result),
                Err(err) => {
                    // A probe panicked: treat as failed, keep serving.
                    tracing::error!(error = %err, "readiness probe task failed");
                    checks.push(CheckResult {
                        name: "<panicked>",
                        status: CheckStatus::Failed,
                        duration_ms: 0,
                    });
                }
            }
        }
        checks.sort_by_key(|c| c.name);

        let ready = checks.iter().all(|c| c.status == CheckStatus::Ok);
        ReadinessReport {
            status: if ready { "ready" } else { "not_ready" },
            checks,
        }
    }
}

/// Body of `/health`.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Liveness {
    /// Always `ok`.
    #[schema(example = "ok")]
    pub status: &'static str,
    /// Build version.
    pub version: &'static str,
}

/// `GET /health` handler.
#[utoipa::path(
    get,
    path = "/health",
    tag = "public",
    operation_id = "getHealth",
    summary = "Liveness: the process answers HTTP",
    responses((status = 200, description = "Process is alive", body = Liveness))
)]
pub async fn liveness() -> Json<Liveness> {
    Json(Liveness {
        status: "ok",
        version: builtpanel_version(),
    })
}

/// `GET /ready` handler. Needs `Readiness` reachable via `State` (use `FromRef`).
#[utoipa::path(
    get,
    path = "/ready",
    tag = "public",
    operation_id = "getReadiness",
    summary = "Readiness: all dependencies (database, redis, ...) are reachable",
    responses(
        (status = 200, description = "All checks passed", body = ReadinessReport),
        (status = 503, description = "At least one check failed or timed out", body = ReadinessReport)
    )
)]
pub async fn readiness(State(readiness): State<Readiness>) -> (StatusCode, Json<ReadinessReport>) {
    let report = readiness.check().await;
    let code = if report.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(report))
}

fn builtpanel_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// OpenAPI fragment for the unauthenticated health endpoints. Nest it under
/// the prefix the router mounts them at (e.g. `/api/v1/public`).
#[derive(utoipa::OpenApi)]
#[openapi(
    paths(liveness, readiness),
    components(schemas(Liveness, ReadinessReport, CheckResult, CheckStatus)),
    tags((name = "public", description = "Unauthenticated health endpoints"))
)]
pub struct PublicHealthApi;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ok() -> Probe {
        Arc::new(|| Box::pin(async { Ok(()) }))
    }
    fn fail() -> Probe {
        Arc::new(|| Box::pin(async { Err("connection refused to db.internal:5432".to_owned()) }))
    }
    fn slow() -> Probe {
        Arc::new(|| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(())
            })
        })
    }

    #[tokio::test]
    async fn no_probes_is_ready() {
        let r = Readiness::default().check().await;
        assert!(r.is_ready());
        assert!(r.checks.is_empty());
    }

    #[tokio::test]
    async fn all_ok_is_ready_and_sorted() {
        let r = Readiness::default()
            .with_probe("redis", ok())
            .with_probe("database", ok())
            .check()
            .await;
        assert!(r.is_ready());
        let names: Vec<_> = r.checks.iter().map(|c| c.name).collect();
        assert_eq!(names, ["database", "redis"]);
    }

    #[tokio::test]
    async fn failure_is_not_ready_and_hides_detail() {
        let r = Readiness::default()
            .with_probe("database", fail())
            .with_probe("redis", ok())
            .check()
            .await;
        assert!(!r.is_ready());
        assert_eq!(r.checks[0].status, CheckStatus::Failed);
        let body = serde_json::to_string(&r).unwrap();
        assert!(!body.contains("db.internal"), "{body}");
    }

    #[tokio::test]
    async fn slow_probe_times_out() {
        let r = Readiness::new(Duration::from_millis(50))
            .with_probe("database", slow())
            .check()
            .await;
        assert!(!r.is_ready());
        assert_eq!(r.checks[0].status, CheckStatus::Timeout);
        assert!(r.checks[0].duration_ms < 1000);
    }
}
