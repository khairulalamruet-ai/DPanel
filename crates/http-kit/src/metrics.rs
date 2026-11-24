//! HTTP request metrics middleware.

use std::time::Instant;

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};

/// Label used when no route matched (404s), so unknown paths never create new series.
pub const UNMATCHED_ROUTE: &str = "<unmatched>";

/// Records `http_requests_total` and `http_request_duration_seconds` for every request.
///
/// Install with `Router::layer(axum::middleware::from_fn(track_http_metrics))`.
/// The `route` label is the matched route template, never the raw request path.
pub async fn track_http_metrics(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().as_str().to_owned();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| UNMATCHED_ROUTE.to_owned(), |p| p.as_str().to_owned());

    let response = next.run(request).await;

    builtpanel_observability::record_http_request(
        &method,
        &route,
        response.status().as_u16(),
        started.elapsed(),
    );
    response
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Body, http::Request, routing::get};
    use metrics_exporter_prometheus::PrometheusBuilder;
    use tower::ServiceExt;

    use super::*;

    fn app() -> Router {
        Router::new()
            .route("/items/{id}", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(track_http_metrics))
    }

    #[tokio::test]
    async fn uses_route_template_and_unmatched_label() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        // The recorder is installed for this thread only (current-thread runtime).
        let _guard = metrics::set_default_local_recorder(&recorder);

        for uri in ["/items/1", "/items/2", "/nope"] {
            let req = Request::builder()
                .uri(uri)
                .body(Body::empty())
                .unwrap_or_default();
            let _ = app().oneshot(req).await;
        }

        let text = handle.render();
        assert!(
            text.contains(
                r#"http_requests_total{method="GET",route="/items/{id}",status="200"} 2"#
            ),
            "{text}"
        );
        assert!(
            text.contains(
                r#"http_requests_total{method="GET",route="<unmatched>",status="404"} 1"#
            ),
            "{text}"
        );
        assert!(
            !text.contains("/items/1"),
            "raw paths must not become labels: {text}"
        );
    }
}
