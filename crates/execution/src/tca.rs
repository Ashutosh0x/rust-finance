// crates/execution/src/tca.rs
// Transaction Cost Analysis — measure explicit + implicit execution costs
// TT won 2026 FOW International Award for this category

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// A single fill record with all data needed for TCA
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillRecord {
    pub order_id: String,
    pub symbol: String,
    pub strategy: String,
    /// Price at the moment the order decision was made (pre-signal)
    pub decision_price: f64,
    /// VWAP at time of order submission
    pub arrival_vwap: f64,
    /// Actual fill price
    pub fill_price: f64,
    /// Order side: 1.0 for buy, -1.0 for sell
    pub side_sign: f64,
    pub quantity: u64,
    /// Commission paid in USD
    pub commission_usd: f64,
    /// Timestamp of decision
    pub decision_ts: u64,
    /// Timestamp of fill
    pub fill_ts: u64,
    /// Day's VWAP at time of fill (for VWAP benchmark)
    pub day_vwap: f64,
    /// TWAP over execution window
    pub interval_twap: f64,
}

/// TCA metrics for a single fill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcaMetrics {
    pub order_id: String,
    pub symbol: String,
    pub strategy: String,
    /// Explicit cost: commissions as bps of notional
    pub commission_bps: f64,
    /// Implementation shortfall vs decision price (bps) — THE gold standard metric
    pub implementation_shortfall_bps: f64,
    /// Slippage vs arrival VWAP (bps)
    pub arrival_slippage_bps: f64,
    /// Slippage vs day VWAP (bps)
    pub vwap_slippage_bps: f64,
    /// Slippage vs TWAP (bps)
    pub twap_slippage_bps: f64,
    /// Market impact: price change from decision to fill (bps)
    pub market_impact_bps: f64,
    /// Execution latency in milliseconds
    pub latency_ms: u64,
    /// Total cost (explicit + implicit) in bps
    pub total_cost_bps: f64,
}

impl TcaMetrics {
    pub fn from_fill(fill: &FillRecord) -> Self {
        let notional = fill.fill_price * fill.quantity as f64;

        let commission_bps = if notional > 0.0 {
            (fill.commission_usd / notional) * 10_000.0
        } else { 0.0 };

        // Implementation Shortfall = (fill_price - decision_price) × side_sign
        let is_bps = ((fill.fill_price - fill.decision_price) / fill.decision_price)
            * fill.side_sign * 10_000.0;

        let arrival_bps = ((fill.fill_price - fill.arrival_vwap) / fill.arrival_vwap)
            * fill.side_sign * 10_000.0;

        let vwap_bps = ((fill.fill_price - fill.day_vwap) / fill.day_vwap)
            * fill.side_sign * 10_000.0;

        let twap_bps = ((fill.fill_price - fill.interval_twap) / fill.interval_twap)
            * fill.side_sign * 10_000.0;

        let impact_bps = ((fill.fill_price - fill.decision_price) / fill.decision_price)
            * 10_000.0;

        let latency_ms = fill.fill_ts.saturating_sub(fill.decision_ts) / 1_000;

        Self {
            order_id: fill.order_id.clone(),
            symbol: fill.symbol.clone(),
            strategy: fill.strategy.clone(),
            commission_bps,
            implementation_shortfall_bps: is_bps,
            arrival_slippage_bps: arrival_bps,
            vwap_slippage_bps: vwap_bps,
            twap_slippage_bps: twap_bps,
            market_impact_bps: impact_bps,
            latency_ms,
            total_cost_bps: commission_bps + is_bps.abs(),
        }
    }
}

/// Aggregated TCA report across multiple fills
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TcaReport {
    pub fill_count: usize,
    pub total_notional_usd: f64,
    pub avg_implementation_shortfall_bps: f64,
    pub avg_arrival_slippage_bps: f64,
    pub avg_vwap_slippage_bps: f64,
    pub avg_commission_bps: f64,
    pub avg_total_cost_bps: f64,
    pub avg_latency_ms: f64,
    /// Best execution hour (0-23) by lowest IS cost
    pub best_execution_hour: Option<u8>,
    /// Worst execution hour
    pub worst_execution_hour: Option<u8>,
    /// Per-strategy breakdown
    pub by_strategy: HashMap<String, StrategyTca>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct StrategyTca {
    pub fill_count: usize,
    pub avg_is_bps: f64,
    pub avg_latency_ms: f64,
}

pub struct TcaEngine {
    fills: Vec<FillRecord>,
    metrics: Vec<TcaMetrics>,
}

impl TcaEngine {
    pub fn new() -> Self {
        Self { fills: Vec::new(), metrics: Vec::new() }
    }

    pub fn record_fill(&mut self, fill: FillRecord) -> TcaMetrics {
        let metrics = TcaMetrics::from_fill(&fill);
        tracing::info!(
            order_id = %fill.order_id,
            symbol = %fill.symbol,
            strategy = %fill.strategy,
            is_bps = metrics.implementation_shortfall_bps,
            total_cost_bps = metrics.total_cost_bps,
            latency_ms = metrics.latency_ms,
            "TCA recorded"
        );
        self.fills.push(fill);
        self.metrics.push(metrics.clone());
        metrics
    }

    pub fn generate_report(&self) -> TcaReport {
        if self.metrics.is_empty() {
            return TcaReport::default();
        }
        let n = self.metrics.len() as f64;

        let avg_is    = self.metrics.iter().map(|m| m.implementation_shortfall_bps).sum::<f64>() / n;
        let avg_arr   = self.metrics.iter().map(|m| m.arrival_slippage_bps).sum::<f64>() / n;
        let avg_vwap  = self.metrics.iter().map(|m| m.vwap_slippage_bps).sum::<f64>() / n;
        let avg_comm  = self.metrics.iter().map(|m| m.commission_bps).sum::<f64>() / n;
        let avg_total = self.metrics.iter().map(|m| m.total_cost_bps).sum::<f64>() / n;
        let avg_lat   = self.metrics.iter().map(|m| m.latency_ms as f64).sum::<f64>() / n;
        let total_not = self.fills.iter().map(|f| f.fill_price * f.quantity as f64).sum::<f64>();

        // Per-strategy aggregation
        let mut by_strategy: HashMap<String, (usize, f64, f64)> = HashMap::new();
        for m in &self.metrics {
            let e = by_strategy.entry(m.strategy.clone()).or_default();
            e.0 += 1;
            e.1 += m.implementation_shortfall_bps;
            e.2 += m.latency_ms as f64;
        }
        let by_strategy = by_strategy.into_iter().map(|(k, (cnt, is_sum, lat_sum))| {
            let n = cnt as f64;
            (k, StrategyTca { fill_count: cnt, avg_is_bps: is_sum / n, avg_latency_ms: lat_sum / n })
        }).collect();

        // Best / worst execution hour
        let mut hour_costs: HashMap<u8, Vec<f64>> = HashMap::new();
        for (fill, m) in self.fills.iter().zip(self.metrics.iter()) {
            let hour = ((fill.fill_ts / 1_000_000) % 86400 / 3600) as u8;
            hour_costs.entry(hour).or_default().push(m.implementation_shortfall_bps);
        }
        let hour_avgs: Vec<(u8, f64)> = hour_costs.iter()
            .map(|(h, costs)| (*h, costs.iter().sum::<f64>() / costs.len() as f64))
            .collect();
        let best_hour  = hour_avgs.iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|(h, _)| *h);
        let worst_hour = hour_avgs.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|(h, _)| *h);

        TcaReport {
            fill_count: self.metrics.len(),
            total_notional_usd: total_not,
            avg_implementation_shortfall_bps: avg_is,
            avg_arrival_slippage_bps: avg_arr,
            avg_vwap_slippage_bps: avg_vwap,
            avg_commission_bps: avg_comm,
            avg_total_cost_bps: avg_total,
            avg_latency_ms: avg_lat,
            best_execution_hour: best_hour,
            worst_execution_hour: worst_hour,
            by_strategy,
        }
    }

    pub fn metrics_for_strategy(&self, strategy: &str) -> Vec<&TcaMetrics> {
        self.metrics.iter().filter(|m| m.strategy == strategy).collect()
    }
}
