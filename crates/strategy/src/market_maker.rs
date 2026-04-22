// crates/strategy/src/market_maker.rs
//
// Avellaneda-Stoikov Optimal Market Making Strategy
//
// The foundational model for institutional market making (IMC, Citadel, Jane Street).
// Computes the reservation price (inventory-adjusted fair value) and optimal spread.
//
// Reference: Avellaneda & Stoikov (2008) "High-frequency trading in a limit order book"
//
// Key equations:
//   reservation_price = s - q · γ · σ² · (T - t)
//   optimal_spread   = γ · σ² · (T - t) + (2/γ) · ln(1 + γ/κ)
//
// Where:
//   s = mid price
//   q = current inventory (positive = long, negative = short)
//   γ = risk aversion parameter (higher = tighter inventory control)
//   σ = volatility estimate (annualized)
//   T - t = time remaining in the session
//   κ = order arrival rate (estimated from tick data)

use std::collections::VecDeque;

/// Configuration for the Avellaneda-Stoikov market maker.
#[derive(Debug, Clone)]
pub struct MarketMakerConfig {
    /// Risk aversion parameter. Higher → more aggressive inventory management.
    /// Typical values: 0.01 (relaxed) to 1.0 (aggressive).
    pub gamma: f64,

    /// Order arrival intensity parameter (κ).
    /// Estimated from historical fill rates. Higher = tighter spreads.
    pub kappa: f64,

    /// Session duration in seconds (e.g., 23400 for 6.5hr equity session).
    pub session_duration_secs: f64,

    /// Maximum inventory (absolute) before emergency liquidation.
    pub max_inventory: f64,

    /// Minimum spread as fraction of mid price (fee floor).
    pub min_spread_bps: f64,

    /// Maximum spread as fraction of mid price (don't quote too wide).
    pub max_spread_bps: f64,

    /// Order size (in units of base asset).
    pub order_size: f64,

    /// VPIN threshold above which we widen spreads (adverse selection protection).
    pub vpin_widen_threshold: f64,

    /// Inventory skew factor: how much to asymmetrize quotes per unit inventory.
    pub inventory_skew_factor: f64,
}

impl Default for MarketMakerConfig {
    fn default() -> Self {
        Self {
            gamma: 0.1,
            kappa: 1.5,
            session_duration_secs: 23400.0, // 6.5 hours
            max_inventory: 100.0,
            min_spread_bps: 5.0,   // 5 bps minimum (must cover fees)
            max_spread_bps: 100.0, // 100 bps maximum
            order_size: 1.0,
            vpin_widen_threshold: 0.7,
            inventory_skew_factor: 0.5,
        }
    }
}

/// A two-sided quote produced by the market maker.
#[derive(Debug, Clone)]
pub struct Quote {
    pub bid_price: f64,
    pub ask_price: f64,
    pub bid_size: f64,
    pub ask_size: f64,
    pub reservation_price: f64,
    pub optimal_spread: f64,
    pub inventory: f64,
    pub reason: String,
}

/// Quote action: what the market maker wants to do.
#[derive(Debug)]
pub enum MakerAction {
    /// Post new two-sided quote.
    UpdateQuotes(Quote),
    /// Cancel all quotes (risk event).
    CancelAll(String),
    /// Emergency: flatten inventory immediately.
    Flatten(String),
}

/// The Avellaneda-Stoikov market maker engine.
pub struct AvellanedaStoikov {
    config: MarketMakerConfig,

    // State
    inventory: f64,
    session_elapsed_secs: f64,

    // Volatility estimation (EWMA)
    return_window: VecDeque<f64>,
    prev_mid: f64,
    ewma_variance: f64,
    ewma_lambda: f64, // decay factor for EWMA vol

    // Order arrival rate estimation
    trade_count_window: VecDeque<f64>, // timestamps of trades in last N seconds
    arrival_rate: f64,

    // Toxicity
    vpin_value: f64,

    initialized: bool,
}

impl AvellanedaStoikov {
    pub fn new(config: MarketMakerConfig) -> Self {
        Self {
            config,
            inventory: 0.0,
            session_elapsed_secs: 0.0,
            return_window: VecDeque::with_capacity(500),
            prev_mid: 0.0,
            ewma_variance: 0.0,
            ewma_lambda: 0.94, // RiskMetrics standard
            trade_count_window: VecDeque::with_capacity(1000),
            arrival_rate: 1.5,
            vpin_value: 0.0,
            initialized: false,
        }
    }

    /// Feed a new mid-price observation and produce a quoting decision.
    pub fn on_tick(
        &mut self,
        mid_price: f64,
        elapsed_secs: f64,
        vpin: Option<f64>,
    ) -> MakerAction {
        self.session_elapsed_secs = elapsed_secs;

        if let Some(v) = vpin {
            self.vpin_value = v;
        }

        // Initialize
        if !self.initialized {
            self.prev_mid = mid_price;
            self.initialized = true;
            return MakerAction::UpdateQuotes(self.compute_quote(mid_price));
        }

        // Update EWMA volatility
        let ret = (mid_price / self.prev_mid).ln();
        self.return_window.push_back(ret);
        if self.return_window.len() > 500 {
            self.return_window.pop_front();
        }
        self.ewma_variance =
            self.ewma_lambda * self.ewma_variance + (1.0 - self.ewma_lambda) * ret * ret;
        self.prev_mid = mid_price;

        // Emergency: max inventory breach
        if self.inventory.abs() > self.config.max_inventory {
            return MakerAction::Flatten(format!(
                "Inventory {} exceeds max {}",
                self.inventory, self.config.max_inventory
            ));
        }

        // High toxicity: widen or pull quotes
        if self.vpin_value > 0.85 {
            return MakerAction::CancelAll(format!(
                "VPIN {:.2} > 0.85 — toxic flow detected, pulling quotes",
                self.vpin_value
            ));
        }

        MakerAction::UpdateQuotes(self.compute_quote(mid_price))
    }

    /// Record a fill event.
    pub fn on_fill(&mut self, qty: f64, is_our_bid: bool) {
        if is_our_bid {
            self.inventory += qty; // We bought
        } else {
            self.inventory -= qty; // We sold
        }
    }

    /// Record a market trade (for arrival rate estimation).
    pub fn on_market_trade(&mut self, timestamp_secs: f64) {
        self.trade_count_window.push_back(timestamp_secs);

        // Keep only last 60 seconds
        while let Some(&t) = self.trade_count_window.front() {
            if timestamp_secs - t > 60.0 {
                self.trade_count_window.pop_front();
            } else {
                break;
            }
        }

        // Arrival rate = trades per second
        self.arrival_rate = self.trade_count_window.len() as f64 / 60.0;
    }

    fn compute_quote(&self, mid: f64) -> Quote {
        let gamma = self.config.gamma;
        let sigma = self.annualized_vol();
        let sigma_sq = sigma * sigma;
        let tau = self.time_remaining();
        let q = self.inventory;
        let kappa = self.arrival_rate.max(0.1); // Use live estimate, floor at 0.1

        // ── Reservation price ────────────────────────────────────
        // r = s - q · γ · σ² · τ
        // When long (q > 0): reservation price drops below mid → sell bias
        // When short (q < 0): reservation price rises above mid → buy bias
        let reservation = mid - q * gamma * sigma_sq * tau;

        // ── Optimal spread ───────────────────────────────────────
        // δ = γσ²τ + (2/γ) · ln(1 + γ/κ)
        let spread = gamma * sigma_sq * tau + (2.0 / gamma) * (1.0 + gamma / kappa).ln();

        // Apply VPIN-based spread widening
        let toxicity_multiplier = if self.vpin_value > self.config.vpin_widen_threshold {
            1.0 + (self.vpin_value - self.config.vpin_widen_threshold) * 3.0
        } else {
            1.0
        };

        let adjusted_spread = spread * toxicity_multiplier;

        // Clamp spread to bounds
        let half_spread = (adjusted_spread / 2.0)
            .max(mid * self.config.min_spread_bps / 10_000.0)
            .min(mid * self.config.max_spread_bps / 10_000.0);

        // ── Inventory skew ───────────────────────────────────────
        // When long: tighten ask (make it easier to sell), widen bid
        // When short: tighten bid (make it easier to buy), widen ask
        let skew = q * self.config.inventory_skew_factor * sigma_sq * tau;

        let bid = reservation - half_spread - skew;
        let ask = reservation + half_spread - skew;

        // ── Size adjustment based on inventory ───────────────────
        let base_size = self.config.order_size;
        let inventory_ratio = q.abs() / self.config.max_inventory.max(1.0);
        let bid_size = if q > 0.0 {
            base_size * (1.0 - inventory_ratio * 0.5) // Reduce bid when long
        } else {
            base_size
        };
        let ask_size = if q < 0.0 {
            base_size * (1.0 - inventory_ratio * 0.5) // Reduce ask when short
        } else {
            base_size
        };

        Quote {
            bid_price: bid,
            ask_price: ask,
            bid_size: bid_size.max(0.01),
            ask_size: ask_size.max(0.01),
            reservation_price: reservation,
            optimal_spread: adjusted_spread,
            inventory: q,
            reason: format!(
                "AS: mid={:.2} res={:.4} spread={:.4}bps q={:.1} vpin={:.2} σ={:.4}",
                mid,
                reservation,
                adjusted_spread / mid * 10_000.0,
                q,
                self.vpin_value,
                sigma
            ),
        }
    }

    fn annualized_vol(&self) -> f64 {
        // EWMA variance is per-tick; annualize assuming ~23400 seconds per day
        // and ~252 trading days. Ticks per day depends on data frequency.
        let daily_var = self.ewma_variance * 23400.0; // crude: 1-sec ticks
        let annual_var = daily_var * 252.0;
        annual_var.sqrt().max(0.01) // Floor at 1% annual vol
    }

    fn time_remaining(&self) -> f64 {
        let remaining = self.config.session_duration_secs - self.session_elapsed_secs;
        (remaining / self.config.session_duration_secs).max(0.001)
    }

    /// Current inventory position.
    pub fn inventory(&self) -> f64 {
        self.inventory
    }

    /// Current EWMA volatility estimate (annualized).
    pub fn volatility(&self) -> f64 {
        self.annualized_vol()
    }

    /// Current estimated order arrival rate (trades/sec).
    pub fn arrival_rate(&self) -> f64 {
        self.arrival_rate
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reservation_price_long_inventory() {
        let mut mm = AvellanedaStoikov::new(MarketMakerConfig::default());

        // Initialize
        mm.on_tick(100.0, 0.0, Some(0.3));
        mm.on_fill(10.0, true); // Now long 10 units

        if let MakerAction::UpdateQuotes(q) = mm.on_tick(100.0, 100.0, Some(0.3)) {
            // Long inventory → reservation < mid → sell bias
            assert!(
                q.reservation_price <= 100.0,
                "Long inventory reservation should be <= mid: {}",
                q.reservation_price
            );
            // Ask should be tighter than bid (want to sell)
            let bid_dist = q.reservation_price - q.bid_price;
            let ask_dist = q.ask_price - q.reservation_price;
            assert!(
                bid_dist > 0.0 && ask_dist > 0.0,
                "Both sides should be positive distance from reservation"
            );
        }
    }

    #[test]
    fn test_reservation_price_short_inventory() {
        let mut mm = AvellanedaStoikov::new(MarketMakerConfig::default());

        mm.on_tick(100.0, 0.0, Some(0.3));
        mm.on_fill(10.0, false); // Now short 10 units

        if let MakerAction::UpdateQuotes(q) = mm.on_tick(100.0, 100.0, Some(0.3)) {
            // Short inventory → reservation > mid → buy bias
            assert!(
                q.reservation_price >= 100.0,
                "Short inventory reservation should be >= mid: {}",
                q.reservation_price
            );
        }
    }

    #[test]
    fn test_zero_inventory_symmetric() {
        let mut mm = AvellanedaStoikov::new(MarketMakerConfig::default());

        mm.on_tick(100.0, 0.0, Some(0.3));

        if let MakerAction::UpdateQuotes(q) = mm.on_tick(100.0, 100.0, Some(0.3)) {
            // Zero inventory → reservation == mid
            assert!(
                (q.reservation_price - 100.0).abs() < 0.01,
                "Zero inventory reservation should be near mid: {}",
                q.reservation_price
            );
            // Spread should be symmetric
            let bid_dist = q.reservation_price - q.bid_price;
            let ask_dist = q.ask_price - q.reservation_price;
            assert!(
                (bid_dist - ask_dist).abs() < 0.01,
                "Spread should be symmetric: bid_dist={}, ask_dist={}",
                bid_dist,
                ask_dist
            );
        }
    }

    #[test]
    fn test_high_vpin_widens_spread() {
        let mut mm = AvellanedaStoikov::new(MarketMakerConfig::default());

        // Normal VPIN
        mm.on_tick(100.0, 0.0, Some(0.3));
        let normal_spread = if let MakerAction::UpdateQuotes(q) = mm.on_tick(100.0, 100.0, Some(0.3)) {
            q.optimal_spread
        } else {
            panic!("Expected quote");
        };

        // High VPIN
        let mut mm2 = AvellanedaStoikov::new(MarketMakerConfig::default());
        mm2.on_tick(100.0, 0.0, Some(0.8));
        let wide_spread = if let MakerAction::UpdateQuotes(q) = mm2.on_tick(100.0, 100.0, Some(0.8)) {
            q.optimal_spread
        } else {
            panic!("Expected quote");
        };

        assert!(
            wide_spread > normal_spread,
            "High VPIN should widen spread: normal={}, wide={}",
            normal_spread,
            wide_spread
        );
    }

    #[test]
    fn test_max_inventory_flattens() {
        let config = MarketMakerConfig {
            max_inventory: 50.0,
            ..Default::default()
        };
        let mut mm = AvellanedaStoikov::new(config);

        mm.on_tick(100.0, 0.0, Some(0.3));
        // Simulate filling 60 units (above max 50)
        mm.on_fill(60.0, true);

        let action = mm.on_tick(100.0, 100.0, Some(0.3));
        assert!(
            matches!(action, MakerAction::Flatten(_)),
            "Should flatten at max inventory"
        );
    }

    #[test]
    fn test_extreme_vpin_cancels() {
        let mut mm = AvellanedaStoikov::new(MarketMakerConfig::default());

        mm.on_tick(100.0, 0.0, Some(0.3));

        let action = mm.on_tick(100.0, 100.0, Some(0.90));
        assert!(
            matches!(action, MakerAction::CancelAll(_)),
            "Should cancel all at extreme VPIN"
        );
    }
}
