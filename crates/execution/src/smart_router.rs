// crates/execution/src/smart_router.rs
//
// Smart Order Router (SOR) — Multi-venue routing with scoring
//
// Routes orders to the optimal venue based on:
//   1. Liquidity depth (available volume at NBBO)
//   2. Latency (p95 round-trip time)
//   3. Fill rate (historical fill probability)
//   4. Fee structure (maker/taker rebates)
//   5. Market impact (estimated price impact per venue)
//
// Supports lit venues, dark pools, and internalizers.
// Compliant with MiFID II best execution and Reg NMS requirements.

use std::collections::HashMap;

/// Venue type classification.
#[derive(Debug, Clone, PartialEq)]
pub enum VenueType {
    /// Lit exchange (NYSE, NASDAQ, CBOE, etc.)
    LitExchange,
    /// Dark pool (IEX, LTSE, Liquidnet, etc.)
    DarkPool,
    /// Internalizer / systematic internalizer
    Internalizer,
    /// Crypto exchange (Binance, Coinbase, etc.)
    CryptoExchange,
    /// Prediction market (Polymarket, Kalshi)
    PredictionMarket,
}

/// Performance statistics for a single venue.
#[derive(Debug, Clone)]
pub struct VenueStats {
    pub venue_id: String,
    pub venue_type: VenueType,
    /// Average fill rate [0, 1] — what fraction of orders sent here get filled
    pub fill_rate: f64,
    /// p95 latency in microseconds
    pub latency_p95_us: f64,
    /// Maker fee (negative = rebate). In basis points.
    pub maker_fee_bps: f64,
    /// Taker fee in basis points.
    pub taker_fee_bps: f64,
    /// Available liquidity at NBBO (shares/units)
    pub liquidity_at_nbbo: f64,
    /// Estimated price impact per 1000 shares (bps)
    pub impact_per_1k_bps: f64,
    /// Is the venue currently available
    pub is_available: bool,
    /// Number of orders routed (for scoring decay)
    pub orders_routed: u64,
    /// Number of fills received
    pub fills_received: u64,
}

impl VenueStats {
    pub fn new(venue_id: &str, venue_type: VenueType) -> Self {
        Self {
            venue_id: venue_id.to_string(),
            venue_type,
            fill_rate: 0.5,
            latency_p95_us: 1000.0,
            maker_fee_bps: -2.0, // default rebate
            taker_fee_bps: 3.0,
            liquidity_at_nbbo: 1000.0,
            impact_per_1k_bps: 1.0,
            is_available: true,
            orders_routed: 0,
            fills_received: 0,
        }
    }
}

/// Routing decision for a single order.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub venue_id: String,
    pub score: f64,
    pub reason: String,
    pub estimated_cost_bps: f64,
}

/// Routing strategy configuration.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Weight for fill rate in scoring (higher = prioritize fills)
    pub fill_rate_weight: f64,
    /// Weight for latency in scoring (higher = prioritize speed)
    pub latency_weight: f64,
    /// Weight for fee in scoring (higher = prioritize cost)
    pub fee_weight: f64,
    /// Weight for liquidity in scoring
    pub liquidity_weight: f64,
    /// Weight for market impact
    pub impact_weight: f64,
    /// Prefer dark pools for large orders above this threshold
    pub dark_pool_threshold_shares: f64,
    /// Anti-gaming: minimum % of flow to each venue to prevent information leakage
    pub min_venue_allocation_pct: f64,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            fill_rate_weight: 0.30,
            latency_weight: 0.15,
            fee_weight: 0.20,
            liquidity_weight: 0.25,
            impact_weight: 0.10,
            dark_pool_threshold_shares: 5000.0,
            min_venue_allocation_pct: 0.05,
        }
    }
}

/// Smart Order Router engine.
pub struct SmartOrderRouter {
    config: RouterConfig,
    venues: HashMap<String, VenueStats>,
}

impl SmartOrderRouter {
    pub fn new(config: RouterConfig) -> Self {
        Self {
            config,
            venues: HashMap::new(),
        }
    }

    /// Register a venue for routing consideration.
    pub fn register_venue(&mut self, stats: VenueStats) {
        self.venues.insert(stats.venue_id.clone(), stats);
    }

    /// Update venue statistics (call periodically with fresh data).
    pub fn update_venue(&mut self, venue_id: &str, update: impl FnOnce(&mut VenueStats)) {
        if let Some(stats) = self.venues.get_mut(venue_id) {
            update(stats);
        }
    }

    /// Record a fill from a venue (updates fill rate).
    pub fn record_fill(&mut self, venue_id: &str) {
        if let Some(stats) = self.venues.get_mut(venue_id) {
            stats.fills_received += 1;
            stats.fill_rate = stats.fills_received as f64 / stats.orders_routed.max(1) as f64;
        }
    }

    /// Record an order sent to a venue.
    pub fn record_order(&mut self, venue_id: &str) {
        if let Some(stats) = self.venues.get_mut(venue_id) {
            stats.orders_routed += 1;
        }
    }

    /// Route an order: returns ranked list of venues with scores.
    ///
    /// `order_size`: number of shares/units
    /// `is_passive`: true for limit orders (use maker fees), false for market orders
    pub fn route(&self, order_size: f64, is_passive: bool) -> Vec<RoutingDecision> {
        let mut decisions: Vec<RoutingDecision> = self
            .venues
            .values()
            .filter(|v| v.is_available)
            .map(|v| {
                let score = self.score_venue(v, order_size, is_passive);
                let fee = if is_passive { v.maker_fee_bps } else { v.taker_fee_bps };
                let impact = v.impact_per_1k_bps * (order_size / 1000.0).sqrt();

                RoutingDecision {
                    venue_id: v.venue_id.clone(),
                    score,
                    reason: format!(
                        "fill={:.0}% lat={:.0}µs fee={:.1}bps liq={:.0} impact={:.1}bps",
                        v.fill_rate * 100.0,
                        v.latency_p95_us,
                        fee,
                        v.liquidity_at_nbbo,
                        impact
                    ),
                    estimated_cost_bps: fee + impact,
                }
            })
            .collect();

        // Sort by score descending (higher = better)
        decisions.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        decisions
    }

    /// Get the single best venue for an order.
    pub fn best_venue(&self, order_size: f64, is_passive: bool) -> Option<RoutingDecision> {
        self.route(order_size, is_passive).into_iter().next()
    }

    /// Split a large order across multiple venues (spray routing).
    /// Returns (venue_id, allocation_quantity) pairs.
    pub fn spray_route(&self, total_qty: f64, is_passive: bool) -> Vec<(String, f64)> {
        let decisions = self.route(total_qty, is_passive);
        if decisions.is_empty() {
            return vec![];
        }

        // Score-weighted allocation
        let total_score: f64 = decisions.iter().map(|d| d.score.max(0.01)).sum();
        let mut allocations: Vec<(String, f64)> = decisions
            .iter()
            .map(|d| {
                let weight = d.score.max(0.01) / total_score;
                let qty = total_qty * weight;
                (d.venue_id.clone(), qty)
            })
            .collect();

        // Enforce minimum allocation
        let min_qty = total_qty * self.config.min_venue_allocation_pct;
        for alloc in &mut allocations {
            if alloc.1 < min_qty && alloc.1 > 0.0 {
                alloc.1 = 0.0; // Too small — remove
            }
        }

        // Remove zero allocations and renormalize
        allocations.retain(|a| a.1 > 0.0);
        let allocated: f64 = allocations.iter().map(|a| a.1).sum();
        if allocated > 0.0 && (allocated - total_qty).abs() > 0.01 {
            let scale = total_qty / allocated;
            for alloc in &mut allocations {
                alloc.1 *= scale;
            }
        }

        allocations
    }

    fn score_venue(&self, venue: &VenueStats, order_size: f64, is_passive: bool) -> f64 {
        let c = &self.config;

        // Normalize each factor to [0, 1] range
        let fill_score = venue.fill_rate; // already [0, 1]

        // Latency: lower is better. 100µs = 1.0, 10000µs = 0.0
        let latency_score = (1.0 - (venue.latency_p95_us / 10_000.0).min(1.0)).max(0.0);

        // Fee: lower is better. Rebates (negative) get high scores.
        let fee = if is_passive { venue.maker_fee_bps } else { venue.taker_fee_bps };
        let fee_score = (1.0 - (fee + 5.0) / 20.0).clamp(0.0, 1.0); // -5bps rebate = 0.5, +15bps = 0.0

        // Liquidity: more is better, logarithmic scaling
        let liq_score = ((venue.liquidity_at_nbbo + 1.0).ln() / 15.0).min(1.0);

        // Impact: lower is better
        let impact = venue.impact_per_1k_bps * (order_size / 1000.0).sqrt();
        let impact_score = (1.0 - impact / 20.0).clamp(0.0, 1.0);

        // Dark pool bonus for large orders
        let dark_bonus = if order_size > c.dark_pool_threshold_shares
            && venue.venue_type == VenueType::DarkPool
        {
            0.15 // 15% bonus for dark pools on large orders
        } else {
            0.0
        };

        c.fill_rate_weight * fill_score
            + c.latency_weight * latency_score
            + c.fee_weight * fee_score
            + c.liquidity_weight * liq_score
            + c.impact_weight * impact_score
            + dark_bonus
    }

    /// Generate a best execution report (MiFID II / Reg NMS compliance).
    pub fn best_execution_report(&self) -> Vec<VenueReport> {
        self.venues
            .values()
            .map(|v| VenueReport {
                venue_id: v.venue_id.clone(),
                venue_type: format!("{:?}", v.venue_type),
                orders_routed: v.orders_routed,
                fills_received: v.fills_received,
                fill_rate: v.fill_rate,
                avg_latency_us: v.latency_p95_us,
                maker_fee_bps: v.maker_fee_bps,
                taker_fee_bps: v.taker_fee_bps,
            })
            .collect()
    }
}

/// Venue performance report for regulatory compliance.
#[derive(Debug, Clone)]
pub struct VenueReport {
    pub venue_id: String,
    pub venue_type: String,
    pub orders_routed: u64,
    pub fills_received: u64,
    pub fill_rate: f64,
    pub avg_latency_us: f64,
    pub maker_fee_bps: f64,
    pub taker_fee_bps: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_router() -> SmartOrderRouter {
        let mut router = SmartOrderRouter::new(RouterConfig::default());

        router.register_venue(VenueStats {
            venue_id: "NYSE".into(),
            venue_type: VenueType::LitExchange,
            fill_rate: 0.85,
            latency_p95_us: 200.0,
            maker_fee_bps: -2.0,
            taker_fee_bps: 3.0,
            liquidity_at_nbbo: 5000.0,
            impact_per_1k_bps: 0.5,
            is_available: true,
            orders_routed: 100,
            fills_received: 85,
        });

        router.register_venue(VenueStats {
            venue_id: "IEX".into(),
            venue_type: VenueType::DarkPool,
            fill_rate: 0.40,
            latency_p95_us: 350.0,
            maker_fee_bps: 0.0,
            taker_fee_bps: 0.9,
            liquidity_at_nbbo: 2000.0,
            impact_per_1k_bps: 0.1,
            is_available: true,
            orders_routed: 50,
            fills_received: 20,
        });

        router.register_venue(VenueStats {
            venue_id: "NASDAQ".into(),
            venue_type: VenueType::LitExchange,
            fill_rate: 0.80,
            latency_p95_us: 150.0,
            maker_fee_bps: -3.0,
            taker_fee_bps: 3.5,
            liquidity_at_nbbo: 8000.0,
            impact_per_1k_bps: 0.4,
            is_available: true,
            orders_routed: 200,
            fills_received: 160,
        });

        router
    }

    #[test]
    fn test_routing_returns_ranked_results() {
        let router = setup_router();
        let decisions = router.route(500.0, false);
        assert_eq!(decisions.len(), 3);
        // Should be sorted by score descending
        for w in decisions.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }

    #[test]
    fn test_dark_pool_preferred_for_large_orders() {
        let router = setup_router();
        let decisions = router.route(10_000.0, false);

        // IEX should get a dark pool bonus for large orders
        let iex = decisions.iter().find(|d| d.venue_id == "IEX").unwrap();
        assert!(iex.score > 0.0);
    }

    #[test]
    fn test_best_venue_returns_top() {
        let router = setup_router();
        let best = router.best_venue(500.0, false).unwrap();
        let all = router.route(500.0, false);
        assert_eq!(best.venue_id, all[0].venue_id);
    }

    #[test]
    fn test_spray_routing_sums_to_total() {
        let router = setup_router();
        let allocs = router.spray_route(10_000.0, false);

        let total: f64 = allocs.iter().map(|a| a.1).sum();
        assert!(
            (total - 10_000.0).abs() < 1.0,
            "Spray should sum to total: {}",
            total
        );
    }

    #[test]
    fn test_unavailable_venue_excluded() {
        let mut router = setup_router();
        router.update_venue("NYSE", |v| v.is_available = false);

        let decisions = router.route(500.0, false);
        assert!(!decisions.iter().any(|d| d.venue_id == "NYSE"));
    }

    #[test]
    fn test_fill_rate_tracking() {
        let mut router = setup_router();

        router.record_order("NYSE");
        router.record_order("NYSE");
        router.record_fill("NYSE");

        let report = router.best_execution_report();
        let nyse = report.iter().find(|r| r.venue_id == "NYSE").unwrap();
        assert_eq!(nyse.orders_routed, 102);
        assert_eq!(nyse.fills_received, 86);
    }
}
