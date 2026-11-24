//! Tracing, OpenTelemetry and Prometheus metrics initialisation.
//!
//! Every Built Panel binary calls [`init`] once at startup and keeps the returned
//! [`Telemetry`] alive until shutdown. It installs:
//!
//! * a global `tracing` subscriber (JSON or human-readable lines, filtered by
//!   `RUST_LOG`-style directives),
//! * a global `metrics` recorder whose state is rendered in Prometheus text
//!   format by [`Telemetry::render_metrics`],
//! * optionally (cargo feature `otel` + an OTLP endpoint) an OpenTelemetry span
//!   exporter.
//!
//! This crate has no web-framework dependency; HTTP middleware lives in
//! `builtpanel-http-kit` and reports through [`record_http_request`].

pub mod ring;

use std::time::Duration;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Metric name for the HTTP request counter.
pub const HTTP_REQUESTS_TOTAL: &str = "http_requests_total";
/// Metric name for the HTTP request latency histogram (seconds).
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "http_request_duration_seconds";

/// Log line format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// One JSON object per line (production default).
    #[default]
    Json,
    /// Human-readable, coloured lines for local development.
    Pretty,
}

/// Telemetry settings. Normally built from the service configuration.
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Logical service name, e.g. `control-api`. Used as the OTel service name.
    pub service_name: String,
    /// Log line format.
    pub log_format: LogFormat,
    /// `EnvFilter` directives, e.g. `info,builtpanel=debug`.
    pub log_filter: String,
    /// OTLP/HTTP endpoint (e.g. `http://127.0.0.1:4318/v1/traces`).
    /// Ignored unless the crate is built with the `otel` feature.
    pub otlp_endpoint: Option<String>,
}

impl TelemetryConfig {
    /// Development-friendly defaults for the given service.
    #[must_use]
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            log_format: LogFormat::Json,
            log_filter: "info".to_owned(),
            otlp_endpoint: None,
        }
    }
}

/// Errors raised while installing telemetry.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// The log filter directives could not be parsed.
    #[error("invalid log filter `{filter}`: {source}")]
    InvalidFilter {
        /// The rejected directive string.
        filter: String,
        /// Parser error.
        #[source]
        source: tracing_subscriber::filter::ParseError,
    },
    /// A global tracing subscriber was already installed.
    #[error("tracing subscriber already installed: {0}")]
    SubscriberAlreadySet(#[from] tracing_subscriber::util::TryInitError),
    /// The Prometheus recorder could not be installed (usually: already installed).
    #[error("metrics recorder: {0}")]
    Metrics(#[from] metrics_exporter_prometheus::BuildError),
    /// The OTLP exporter could not be built.
    #[cfg(feature = "otel")]
    #[error("otlp exporter: {0}")]
    Otlp(#[from] opentelemetry_otlp::ExporterBuildError),
}

/// Handle to installed telemetry. Keep it alive for the life of the process and
/// call [`Telemetry::shutdown`] before exiting so buffered spans are flushed.
#[derive(Debug)]
pub struct Telemetry {
    prometheus: PrometheusHandle,
    #[cfg(feature = "otel")]
    tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Telemetry {
    /// Current metrics in Prometheus text exposition format.
    #[must_use]
    pub fn render_metrics(&self) -> String {
        self.prometheus.render()
    }

    /// A cloneable handle for serving `/metrics` from an HTTP handler.
    #[must_use]
    pub fn prometheus_handle(&self) -> PrometheusHandle {
        self.prometheus.clone()
    }

    /// Flushes and stops exporters. Safe to call once at shutdown.
    pub fn shutdown(self) {
        #[cfg(feature = "otel")]
        if let Some(provider) = self.tracer_provider
            && let Err(err) = provider.shutdown()
        {
            tracing::warn!(error = %err, "otel tracer provider shutdown failed");
        }
    }
}

/// Installs the global tracing subscriber and metrics recorder.
///
/// # Errors
/// Fails if the filter is invalid, if a subscriber or recorder is already
/// installed, or (with `otel`) if the OTLP exporter cannot be built.
pub fn init(config: &TelemetryConfig) -> Result<Telemetry, TelemetryError> {
    let filter =
        EnvFilter::try_new(&config.log_filter).map_err(|source| TelemetryError::InvalidFilter {
            filter: config.log_filter.clone(),
            source,
        })?;

    let json_layer = (config.log_format == LogFormat::Json).then(|| {
        tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .with_target(true)
    });
    let pretty_layer =
        (config.log_format == LogFormat::Pretty).then(tracing_subscriber::fmt::layer);

    #[cfg(feature = "otel")]
    let (otel_layer, tracer_provider) = match &config.otlp_endpoint {
        Some(endpoint) => {
            let (layer, provider) = otel::layer(&config.service_name, endpoint)?;
            (Some(layer), Some(provider))
        }
        None => (None, None),
    };

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(json_layer)
        .with(pretty_layer)
        .with(ring::RingLayer);
    #[cfg(feature = "otel")]
    let registry = registry.with(otel_layer);
    registry.try_init()?;

    let prometheus = PrometheusBuilder::new()
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full(HTTP_REQUEST_DURATION_SECONDS.to_owned()),
            &[
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ],
        )?
        .install_recorder()?;
    describe_metrics();

    #[cfg(not(feature = "otel"))]
    if config.otlp_endpoint.is_some() {
        tracing::warn!("otlp_endpoint is set but this binary was built without the `otel` feature");
    }

    tracing::info!(service = %config.service_name, "telemetry initialised");

    Ok(Telemetry {
        prometheus,
        #[cfg(feature = "otel")]
        tracer_provider,
    })
}

fn describe_metrics() {
    metrics::describe_counter!(
        HTTP_REQUESTS_TOTAL,
        "Total HTTP requests by method, route and status"
    );
    metrics::describe_histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        metrics::Unit::Seconds,
        "HTTP request latency by method and route"
    );
}

/// Records one finished HTTP request.
///
/// `route` must be the matched route template (e.g. `/api/v1/users/{id}`), never
/// the raw path, so label cardinality stays bounded.
pub fn record_http_request(method: &str, route: &str, status: u16, elapsed: Duration) {
    let labels = [
        ("method", method.to_owned()),
        ("route", route.to_owned()),
        ("status", status.to_string()),
    ];
    metrics::counter!(HTTP_REQUESTS_TOTAL, &labels).increment(1);
    metrics::histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        "method" => method.to_owned(),
        "route" => route.to_owned()
    )
    .record(elapsed.as_secs_f64());
}

#[cfg(feature = "otel")]
mod otel {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::{SpanExporter, WithExportConfig};
    use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};

    use super::TelemetryError;

    pub(super) fn layer<S>(
        service_name: &str,
        endpoint: &str,
    ) -> Result<
        (
            tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::SdkTracer>,
            SdkTracerProvider,
        ),
        TelemetryError,
    >
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        let exporter = SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .build()?;
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(
                Resource::builder()
                    .with_service_name(service_name.to_owned())
                    .build(),
            )
            .build();
        let tracer = provider.tracer("builtpanel");
        opentelemetry::global::set_tracer_provider(provider.clone());
        Ok((tracing_opentelemetry::layer().with_tracer(tracer), provider))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)] // panicking on failure is the point of a test
mod tests {
    use super::*;

    #[test]
    fn default_config_is_json_info() {
        let cfg = TelemetryConfig::new("test");
        assert_eq!(cfg.log_format, LogFormat::Json);
        assert_eq!(cfg.log_filter, "info");
        assert!(cfg.otlp_endpoint.is_none());
    }

    #[test]
    fn invalid_filter_is_rejected_before_anything_is_installed() {
        let mut cfg = TelemetryConfig::new("test");
        cfg.log_filter = "info,[=bad".to_owned();
        let err = init(&cfg).expect_err("filter must be rejected");
        assert!(matches!(err, TelemetryError::InvalidFilter { .. }));
    }

    #[test]
    fn http_metrics_are_rendered() {
        // A local recorder keeps this test independent of global state.
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            describe_metrics();
            record_http_request(
                "GET",
                "/api/v1/public/health",
                200,
                Duration::from_millis(3),
            );
        });
        let text = handle.render();
        assert!(text.contains("http_requests_total{method=\"GET\",route=\"/api/v1/public/health\",status=\"200\"} 1"), "{text}");
        assert!(text.contains("http_request_duration_seconds"), "{text}");
    }
}
