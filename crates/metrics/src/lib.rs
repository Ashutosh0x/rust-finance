// crates/metrics/src/lib.rs
//
// Prometheus metrics registry for RustForge.
// Instruments the critical trading path: order latency, fill rate,
// AI signal timing, RPC latency, risk events, and system health.

use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;
use tracing::info;

/// Install the global Prometheus recorder and start the HTTP scrape endpoint.
/// Call once at daemon startup before any other component initialises.
pub fn install(scrape_addr: SocketAddr) {
    PrometheusBuilder::new()
        .with_http_listener(scrape_addr)
        .install()
        .expect("Failed to install Prometheus metrics recorder");

    info!(%scrape_addr, "Prometheus metrics endpoint started");
}

// ── Market Data ───────────────────────────────────────────────────────────────

pub fn record_market_event(source: &str) {
    counter!("rustforge_market_events_total", "source" => source.to_owned()).increment(1);
}

pub fn record_ws_reconnect(source: &str) {
    counter!("rustforge_ws_reconnects_total", "source" => source.to_owned()).increment(1);
}

pub fn set_ws_connected(source: &str, connected: bool) {
    gauge!("rustforge_ws_connected", "source" => source.to_owned())
        .set(if connected { 1.0 } else { 0.0 });
}

// ── Orders & Fills ────────────────────────────────────────────────────────────

pub fn record_order_submitted(symbol: &str, side: &str) {
    counter!(
        "rustforge_orders_submitted_total",
        "symbol" => symbol.to_owned(),
        "side" => side.to_owned()
    ).increment(1);
}

pub fn record_order_filled(symbol: &str, side: &str) {
    counter!(
        "rustforge_orders_filled_total",
        "symbol" => symbol.to_owned(),
        "side" => side.to_owned()
    ).increment(1);
}

pub fn record_order_rejected(symbol: &str, reason: &str) {
    counter!(
        "rustforge_orders_rejected_total",
        "symbol" => symbol.to_owned(),
        "reason" => reason.to_owned()
    ).increment(1);
}

/// Record end-to-end order latency: signal received → order submitted (µs).
pub fn record_order_latency_us(us: f64) {
    histogram!("rustforge_order_latency_us").record(us);
}

pub fn set_open_orders(count: f64) {
    gauge!("rustforge_open_orders").set(count);
}

// ── AI Signals ────────────────────────────────────────────────────────────────

pub fn record_ai_signal(analyst: &str, action: &str, symbol: &str) {
    counter!(
        "rustforge_ai_signals_total",
        "analyst" => analyst.to_owned(),
        "action" => action.to_owned(),
        "symbol" => symbol.to_owned()
    ).increment(1);
}

/// Time taken for one AI analyst call (ms).
pub fn record_ai_latency_ms(analyst: &str, ms: f64) {
    histogram!(
        "rustforge_ai_latency_ms",
        "analyst" => analyst.to_owned()
    ).record(ms);
}

pub fn record_compaction_event() {
    counter!("rustforge_compaction_events_total").increment(1);
}

pub fn set_ai_context_tokens(analyst: &str, tokens: f64) {
    gauge!(
        "rustforge_ai_context_tokens",
        "analyst" => analyst.to_owned()
    ).set(tokens);
}

// ── RPC Relay ─────────────────────────────────────────────────────────────────

pub fn record_rpc_latency_us(node: &str, us: f64) {
    histogram!(
        "rustforge_rpc_latency_us",
        "node" => node.to_owned()
    ).record(us);
}

pub fn record_rpc_failure(node: &str) {
    counter!("rustforge_rpc_failures_total", "node" => node.to_owned()).increment(1);
}

pub fn set_rpc_node_healthy(node: &str, healthy: bool) {
    gauge!("rustforge_rpc_node_healthy", "node" => node.to_owned())
        .set(if healthy { 1.0 } else { 0.0 });
}

// ── Risk & Kill Switch ────────────────────────────────────────────────────────

pub fn record_risk_breach(breach_type: &str) {
    counter!(
        "rustforge_risk_breaches_total",
        "type" => breach_type.to_owned()
    ).increment(1);
}

pub fn set_kill_switch_active(active: bool) {
    gauge!("rustforge_kill_switch_active").set(if active { 1.0 } else { 0.0 });
}

pub fn set_portfolio_drawdown(drawdown: f64) {
    gauge!("rustforge_portfolio_drawdown").set(drawdown);
}

pub fn set_portfolio_value(value: f64) {
    gauge!("rustforge_portfolio_value_usd").set(value);
}

pub fn set_var_95(var: f64) {
    gauge!("rustforge_portfolio_var_95").set(var);
}

pub fn set_garch_vol(symbol: &str, vol: f64) {
    gauge!("rustforge_garch_annualised_vol", "symbol" => symbol.to_owned()).set(vol);
}

// ── Circuit Breaker ───────────────────────────────────────────────────────────

pub fn record_circuit_breaker_open(service: &str) {
    counter!(
        "rustforge_circuit_breaker_opens_total",
        "service" => service.to_owned()
    ).increment(1);
}

pub fn set_circuit_breaker_state(service: &str, state: &str) {
    // state: "closed" = 0, "half_open" = 1, "open" = 2
    let val = match state {
        "closed" => 0.0,
        "half_open" => 1.0,
        "open" => 2.0,
        _ => -1.0,
    };
    gauge!(
        "rustforge_circuit_breaker_state",
        "service" => service.to_owned()
    ).set(val);
}

// ── Persistence ───────────────────────────────────────────────────────────────

pub fn record_db_write(table: &str) {
    counter!("rustforge_db_writes_total", "table" => table.to_owned()).increment(1);
}

pub fn record_db_write_latency_ms(table: &str, ms: f64) {
    histogram!(
        "rustforge_db_write_latency_ms",
        "table" => table.to_owned()
    ).record(ms);
}

pub fn set_db_queue_depth(depth: f64) {
    gauge!("rustforge_db_queue_depth").set(depth);
}

// ── Event Bus ─────────────────────────────────────────────────────────────────

pub fn record_event_bus_publish(event_type: &str) {
    counter!(
        "rustforge_event_bus_publishes_total",
        "type" => event_type.to_owned()
    ).increment(1);
}

pub fn set_event_bus_lag(lag: f64) {
    gauge!("rustforge_event_bus_lag_ms").set(lag);
}

// ── MiroFish Swarm ────────────────────────────────────────────────────────────

pub fn record_swarm_run(symbol: &str, dominant_action: &str) {
    counter!(
        "rustforge_swarm_runs_total",
        "symbol" => symbol.to_owned(),
        "action" => dominant_action.to_owned()
    ).increment(1);
}

pub fn record_swarm_latency_ms(ms: f64) {
    histogram!("rustforge_swarm_latency_ms").record(ms);
}
