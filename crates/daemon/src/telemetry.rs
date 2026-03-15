// crates/daemon/src/telemetry.rs
//
// OpenTelemetry distributed tracing + structured logging initialisation.
// Instruments the critical path with spans:
// market_event → ai_analysis → risk_check → order_submit → rpc_send.

use opentelemetry::{global, trace::TracerProvider};
use opentelemetry_otlp::WithExportConfig;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialise the full observability stack:
/// - JSON structured logging to stdout
/// - OpenTelemetry traces exported to OTLP (Jaeger/Tempo/etc.)
/// - Prometheus metrics (see crates/metrics)
pub fn init(service_name: &'static str, otlp_endpoint: Option<&str>) {
    // ── OTLP Trace Exporter ─────────────────────────────────────────────────
    let tracer = if let Some(endpoint) = otlp_endpoint {
        let exporter = opentelemetry_otlp::new_exporter()
            .http()
            .with_endpoint(endpoint);

        let provider = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(exporter)
            .with_trace_config(opentelemetry_sdk::trace::Config::default().with_resource(
                opentelemetry_sdk::Resource::new(vec![
                    opentelemetry::KeyValue::new("service.name", service_name),
                    opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                ]),
            ))
            .install_batch(opentelemetry_sdk::runtime::Tokio)
            .expect("OTLP pipeline");

        Some(provider.tracer(service_name))
    } else {
        None
    };

    // ── Tracing Subscriber ──────────────────────────────────────────────────
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,rustforge=debug"));

    let json_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .with_target(true)
        .with_thread_names(true);

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(json_layer);

    if let Some(t) = tracer {
        let otel_layer = tracing_opentelemetry::layer().with_tracer(t);
        registry.with(otel_layer).init();
    } else {
        registry.init();
    }

    tracing::info!(service = service_name, "Telemetry initialised");
}

/// Flush all pending telemetry on shutdown.
pub fn shutdown() {
    global::shutdown_tracer_provider();
}

// ── Span helpers ──────────────────────────────────────────────────────────────

/// Wraps the full trade decision pipeline in a parent span.
/// Usage:
/// ```rust,ignore
/// let span = trade_pipeline_span("AAPL");
/// let _guard = span.enter();
/// // ... execute pipeline
/// ```
pub fn trade_pipeline_span(symbol: &str) -> tracing::Span {
    tracing::info_span!(
        "trade_pipeline",
        symbol = symbol,
        otel.name = "trade.pipeline",
    )
}

pub fn ai_analysis_span(analyst: &str, symbol: &str) -> tracing::Span {
    tracing::info_span!(
        "ai_analysis",
        analyst = analyst,
        symbol = symbol,
        otel.name = "ai.analysis",
    )
}

pub fn risk_check_span(symbol: &str) -> tracing::Span {
    tracing::info_span!("risk_check", symbol = symbol, otel.name = "risk.check",)
}

pub fn order_submit_span(symbol: &str, side: &str) -> tracing::Span {
    tracing::info_span!(
        "order_submit",
        symbol = symbol,
        side = side,
        otel.name = "order.submit",
    )
}

pub fn rpc_send_span(node: &str, method: &str) -> tracing::Span {
    tracing::info_span!(
        "rpc_send",
        rpc.node = node,
        rpc.method = method,
        otel.name = "rpc.send",
    )
}

// ── Grafana Tempo / Jaeger docker-compose addition ────────────────────────────
// Add to infra/docker/docker-compose.yml:
//
//  jaeger:
//    image: jaegertracing/all-in-one:1.57
//    container_name: rustforge_jaeger
//    ports:
//      - "16686:16686"   # Jaeger UI
//      - "4318:4318"     # OTLP HTTP receiver
//    networks:
//      - rustforge_net
//
// Then set in .env:
//   OTLP_ENDPOINT=http://localhost:4318/v1/traces
