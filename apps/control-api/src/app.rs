//! HTTP application: shared state, routes and middleware stack.

use std::{any::Any, net::IpAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{FromRef, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use builtpanel_http_kit::{
    AppError,
    health::{self, Readiness},
};
use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer, limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
};

/// Renders the current metrics in Prometheus text format. A closure keeps the
/// router independent of the exporter type and easy to fake in tests.
pub type MetricsHandle = Arc<dyn Fn() -> String + Send + Sync>;

/// Whole-request timeout. Long-running work goes through the operations queue
/// (202 + operation_id), never a long HTTP request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Default maximum request body. An upload endpoint raises it for its own route
/// (`DefaultBodyLimit`), up to [`UPLOAD_LIMIT_BYTES`].
pub const BODY_LIMIT_BYTES: usize = 2 * 1024 * 1024;
/// The most any request body may be, whatever a route asks for (a chunk of an
/// SQL dump is at most 8 MiB).
pub const UPLOAD_LIMIT_BYTES: usize = 9 * 1024 * 1024;

/// State shared by all handlers.
#[derive(Clone)]
pub struct AppState {
    /// Metrics renderer for `/metrics`.
    pub metrics: MetricsHandle,
    /// Readiness probes for `/api/v1/public/ready`.
    pub readiness: Readiness,
    /// Reverse proxies whose `X-Forwarded-For` is trusted when resolving the
    /// client address (recorded in the audit log).
    pub trusted_proxies: Arc<[IpAddr]>,
    /// With a token and a database, `/metrics` also lists the fleet (T10.11) and asks for the
    /// token as a bearer credential.
    pub fleet: Option<FleetMetrics>,
}

/// What `/metrics` needs to show the fleet.
#[derive(Clone)]
pub struct FleetMetrics {
    /// The database.
    pub db: builtpanel_database::sqlx::PgPool,
    /// The bearer token Prometheus sends.
    pub token: Arc<secrecy::SecretString>,
}

impl FromRef<AppState> for Readiness {
    fn from_ref(state: &AppState) -> Self {
        state.readiness.clone()
    }
}

impl FromRef<AppState> for MetricsHandle {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.metrics)
    }
}

/// The routes that belong to no module (health, readiness, the API
/// description, metrics) and the Problem Details 404 for everything unknown.
/// Module routes are merged onto this by the module mount.
pub fn base_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/health", get(health::liveness))
        .route("/ready", get(health::readiness));

    let router = Router::new()
        .nest("/api/v1/public", public)
        .route("/api/v1/openapi.json", get(openapi_json))
        .route("/metrics", get(render_metrics));

    // Interactive API viewer for local development only; never in release builds.
    #[cfg(debug_assertions)]
    let router = {
        use utoipa_scalar::{Scalar, Servable as _};
        router.merge(Scalar::with_url("/api/docs", crate::openapi::document()))
    };

    router
        .fallback(|| async { AppError::not_found("route") })
        .with_state(state)
}

/// Builds the HTTP application around `routes` (the base router alone, or the
/// module mount's live router).
///
/// Layer order, outermost first:
/// correlation ID -> client address -> panic catcher -> metrics -> timeout -> body limit
/// (2 MiB for what handlers read, 9 MiB at most for any body) -> compression -> routes.
pub fn router(routes: Router, trusted_proxies: Arc<[IpAddr]>) -> Router {
    routes
        .layer(axum::middleware::from_fn(security_headers))
        .layer(CompressionLayer::new())
        .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT_BYTES))
        .layer(RequestBodyLimitLayer::new(UPLOAD_LIMIT_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(axum::middleware::from_fn(
            builtpanel_http_kit::metrics::track_http_metrics,
        ))
        .layer(CatchPanicLayer::custom(panic_response))
        .layer(axum::middleware::from_fn_with_state(
            trusted_proxies,
            builtpanel_http_kit::client_ip::client_ip_layer,
        ))
        .layer(axum::middleware::from_fn(
            builtpanel_http_kit::request_id::correlation_id_layer,
        ))
}

/// Headers every answer carries (ASVS V14): the browser is told not to guess types, not to put
/// the page in a frame, not to send a referrer, and not to keep what an API call answered; a
/// JSON answer also gets a policy that lets nothing load.
async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    use axum::http::{HeaderName, HeaderValue};
    let api = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    let mut set = |name: &'static str, value: &'static str| {
        headers
            .entry(HeaderName::from_static(name))
            .or_insert(HeaderValue::from_static(value));
    };
    set("x-content-type-options", "nosniff");
    set("x-frame-options", "DENY");
    set("referrer-policy", "no-referrer");
    if api {
        set("cache-control", "no-store");
    }
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.contains("json"));
    if is_json {
        response.headers_mut().insert(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        );
    }
    response
}

async fn openapi_json() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        crate::openapi::json(),
    )
}

async fn render_metrics(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Response {
    use secrecy::ExposeSecret as _;
    let mut body = (state.metrics)();
    if let Some(fleet) = &state.fleet {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        let wanted = fleet.token.expose_secret();
        // Compared without stopping at the first difference.
        let same = given.len() == wanted.len()
            && given
                .bytes()
                .zip(wanted.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        if !same {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                "a bearer token is needed\n",
            )
                .into_response();
        }
        // A slow database must not hold up the scrape: the process's own numbers still go.
        match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            builtpanel_database::usage::prometheus_text(&fleet.db),
        )
        .await
        {
            Ok(Ok(text)) => body.push_str(&text),
            Ok(Err(error)) => tracing::warn!(%error, "could not read the fleet metrics"),
            Err(_) => tracing::warn!("reading the fleet metrics took too long"),
        }
    }
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

/// Turns a handler panic into a Problem Details 500 instead of dropping the connection.
fn panic_response(payload: Box<dyn Any + Send + 'static>) -> Response {
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned());
    AppError::internal(std::io::Error::other(format!(
        "handler panicked: {message}"
    )))
    .into_response()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use axum::{body::Body, http::Request};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    fn state(readiness: Readiness) -> AppState {
        AppState {
            metrics: Arc::new(|| "http_requests_total 1\n".to_owned()),
            readiness,
            trusted_proxies: Arc::from(Vec::new()),
            fleet: None,
        }
    }

    /// The application without any module.
    fn plain(state: AppState) -> Router {
        let proxies = state.trusted_proxies.clone();
        router(base_router(state), proxies)
    }

    async fn get_json(app: Router, uri: &str) -> (StatusCode, Value) {
        let res = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        // The OpenAPI document alone is well over 64 KiB.
        let bytes = axum::body::to_bytes(res.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn health_is_ok_with_version() {
        let (status, body) =
            get_json(plain(state(Readiness::default())), "/api/v1/public/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert!(body["version"].is_string());
    }

    #[tokio::test]
    async fn ready_is_200_when_all_probes_pass() {
        let readiness =
            Readiness::default().with_probe("database", Arc::new(|| Box::pin(async { Ok(()) })));
        let (status, body) = get_json(plain(state(readiness)), "/api/v1/public/ready").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ready");
        assert_eq!(body["checks"][0]["name"], "database");
        assert_eq!(body["checks"][0]["status"], "ok");
    }

    #[tokio::test]
    async fn ready_is_503_when_a_probe_fails() {
        let readiness = Readiness::default().with_probe(
            "database",
            Arc::new(|| Box::pin(async { Err("down".to_owned()) })),
        );
        let (status, body) = get_json(plain(state(readiness)), "/api/v1/public/ready").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "not_ready");
        assert_eq!(body["checks"][0]["status"], "failed");
    }

    #[tokio::test]
    async fn openapi_endpoint_serves_the_document() {
        let (status, body) =
            get_json(plain(state(Readiness::default())), "/api/v1/openapi.json").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["info"]["title"], "Built Panel API");
        assert!(body["paths"]["/api/v1/public/health"].is_object());
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_text() {
        let res = plain(state(Readiness::default()))
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let ct = res.headers()[header::CONTENT_TYPE].to_str().unwrap();
        assert!(ct.starts_with("text/plain"), "{ct}");
        assert!(res.headers().contains_key("x-request-id"));
    }

    #[tokio::test]
    async fn unknown_route_is_problem_json_with_correlation_id() {
        let res = plain(state(Readiness::default()))
            .oneshot(
                Request::builder()
                    .uri("/no/such/route")
                    .header("x-request-id", "corr-abc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            res.headers()[header::CONTENT_TYPE],
            builtpanel_http_kit::error::PROBLEM_JSON
        );
        assert_eq!(res.headers()["x-request-id"], "corr-abc");
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "not_found");
        assert_eq!(body["correlation_id"], "corr-abc");
    }

    #[tokio::test]
    async fn panic_becomes_problem_500_and_keeps_correlation_id() {
        async fn boom() -> &'static str {
            panic!("secret internal detail")
        }
        // Same panic + correlation layers as the real router, plus a route that panics.
        let app = Router::new()
            .route("/boom", get(boom))
            .layer(CatchPanicLayer::custom(panic_response))
            .layer(axum::middleware::from_fn(
                builtpanel_http_kit::request_id::correlation_id_layer,
            ));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/boom")
                    .header("x-request-id", "corr-panic")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(res.headers()["x-request-id"], "corr-panic");
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains(r#""code":"internal_error""#), "{text}");
        assert!(!text.contains("secret internal detail"), "{text}");
    }

    #[tokio::test]
    async fn oversized_body_is_rejected() {
        let app = router(
            Router::new()
                .route(
                    "/echo",
                    axum::routing::post(|body: String| async move { body }),
                )
                .route(
                    "/upload",
                    axum::routing::post(
                        |body: axum::body::Bytes| async move { body.len().to_string() },
                    )
                    .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024)),
                ),
            Arc::from(Vec::new()),
        );
        // A route that asks for more gets it (8 MiB), but not past the absolute cap.
        let post = |uri: &str, n: usize| {
            Request::builder()
                .method("POST")
                .uri(uri.to_owned())
                .body(Body::from("x".repeat(n)))
                .unwrap()
        };
        let res = app
            .clone()
            .oneshot(post("/upload", 8 * 1024 * 1024))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res = app
            .clone()
            .oneshot(post("/upload", UPLOAD_LIMIT_BYTES + 1))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let res = app
            .clone()
            .oneshot(post("/echo", BODY_LIMIT_BYTES + 1))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let big = "x".repeat(BODY_LIMIT_BYTES + 1);
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/echo")
                    .body(Body::from(big))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod fleet_metrics_tests {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use super::*;

    fn app(token: &str) -> Router {
        // A database that is not there: the answer must not depend on it being so.
        let db = builtpanel_database::sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect_lazy("postgres://x:x@127.0.0.1:1/x")
            .expect("a lazy pool");
        let state = AppState {
            metrics: Arc::new(|| "http_requests_total 1\n".to_owned()),
            readiness: Readiness::new(std::time::Duration::from_secs(1)),
            trusted_proxies: Arc::from(Vec::new()),
            fleet: Some(FleetMetrics {
                db,
                token: Arc::new(secrecy::SecretString::from(token.to_owned())),
            }),
        };
        let proxies = state.trusted_proxies.clone();
        router(base_router(state), proxies)
    }

    async fn ask(app: Router, auth: Option<&str>) -> (StatusCode, String) {
        let mut req = Request::builder().uri("/metrics");
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn the_fleet_needs_the_token_and_the_process_metrics_stay_whole_without_the_database() {
        let secret = "s3cret-token-for-prometheus";
        assert_eq!(ask(app(secret), None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(
            ask(app(secret), Some("Bearer nope")).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            ask(app(secret), Some(secret)).await.0,
            StatusCode::UNAUTHORIZED
        );
        let (status, body) = ask(app(secret), Some(&format!("Bearer {secret}"))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("http_requests_total 1"), "{body}");
    }
}
