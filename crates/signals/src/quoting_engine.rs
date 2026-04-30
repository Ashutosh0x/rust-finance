// crates/signals/src/quoting_engine.rs
//
// 2026 Enhanced Avellaneda-Stoikov Quoting Engine
//
// Integrates ALL 2025-2026 research into a single quoting decision:
//   - Avellaneda-Stoikov reservation pricing (2008)
//   - Multi-level microprice fair value (Oxford MLOFI 2019/2026)
//   - Regime-conditioned gamma and spread (RegimeFolio 2025)
//   - Adverse selection detection and gating (Barzykin 2025, Crypto 2026)
//   - VPIN toxicity integration (Easley et al. 2012)
//   - OFI-based quote skewing (Cont et al. 2014)
//   - Kelly-optimal position sizing (PolySwarm 2026)
//
// Core formulas:
// NOTE: positive OFI shifts both bid and ask upward in the implementation.
//   Reservation price: r = fv - q × γ × σ² × τ
//   Optimal half-spread: δ = (γ × σ² × τ)/2 + (1/γ) × ln(1 + γ/κ)
//   Final bid: r - δ × spread_mult × tox_mult - λ_ofi × OFI
//   Final ask: r + δ × spread_mult × tox_mult - λ_ofi × OFI

/// Configuration for the enhanced quoting engine.
#[derive(Debug, Clone)]
pub struct QuotingConfig {
    /// Base risk aversion γ (overridden by regime).
    pub base_gamma: f64,
    /// Order arrival intensity κ estimate.
    pub kappa: f64,
    /// OFI sensitivity λ_ofi (how much OFI shifts quotes).
    pub lambda_ofi: f64,
    /// Minimum half-spread floor (never quote tighter than this).
    pub min_half_spread: f64,
    /// Maximum position limit.
    pub position_limit: f64,
    /// Session duration in ticks (for τ calculation).
    pub session_ticks: u64,
}

impl Default for QuotingConfig {
    fn default() -> Self {
        Self {
            base_gamma: 0.10,
            kappa: 1.5,
            lambda_ofi: 0.5,
            min_half_spread: 1.0,
            position_limit: 100.0,
            session_ticks: 1_000_000,
        }
    }
}

/// Output of the quoting engine.
#[derive(Debug, Clone, Copy)]
pub struct QuotingDecision {
    /// Fair value estimate (multi-level microprice).
    pub fair_value: f64,
    /// Reservation price (inventory-skewed fair value).
    pub reservation_price: f64,
    /// Bid quote.
    pub bid: f64,
    /// Ask quote.
    pub ask: f64,
    /// Optimal half-spread before adjustments.
    pub raw_half_spread: f64,
    /// Final half-spread after regime + toxicity adjustments.
    pub adjusted_half_spread: f64,
    /// Bid size (Kelly-scaled).
    pub bid_size: f64,
    /// Ask size (Kelly-scaled).
    pub ask_size: f64,
    /// Whether quoting is active (false = halt due to toxicity/crisis).
    pub active: bool,
    /// Reason for halt (if active == false).
    pub halt_reason: &'static str,
}

/// The 2026 quoting engine combining all research modules.
pub struct QuotingEngine {
    config: QuotingConfig,
}

impl QuotingEngine {
    pub fn new(config: QuotingConfig) -> Self {
        Self { config }
    }

    /// Compute optimal bid/ask quotes.
    ///
    /// # Arguments
    /// * `fair_value` - Multi-level microprice or Bayesian FV
    /// * `inventory` - Current inventory position (positive = long)
    /// * `sigma` - Current realized volatility (EWMA or GARCH-based)
    /// * `tau` - Remaining session fraction [0, 1]
    /// * `ofi` - Normalized order flow imbalance [-1, +1]
    /// * `vpin` - Volume-synchronized probability of informed trading [0, 1]
    /// * `regime_gamma` - Risk aversion from regime detector
    /// * `regime_spread_mult` - Spread multiplier from regime detector
    /// * `regime_size_mult` - Size multiplier from regime detector
    /// * `toxicity` - Adverse selection toxicity EMA [0, 1]
    /// * `base_size` - Base order size (Kelly-optimal)
    pub fn compute(
        &self,
        fair_value: f64,
        inventory: f64,
        sigma: f64,
        tau: f64,
        ofi: f64,
        vpin: f64,
        regime_gamma: f64,
        regime_spread_mult: f64,
        regime_size_mult: f64,
        toxicity: f64,
        base_size: f64,
    ) -> QuotingDecision {
        let c = &self.config;

        // ── Safety checks ──
        // Halt on extreme toxicity (Barzykin 2025)
        if toxicity > 0.65 {
            return self.halt_decision(fair_value, "adverse_selection_halt");
        }

        // Halt on VPIN > 0.85 (Easley et al. 2012 extreme threshold)
        if vpin > 0.85 {
            return self.halt_decision(fair_value, "vpin_extreme");
        }

        // ── Regime-adjusted gamma ──
        // Use regime_gamma which is already calibrated per regime
        let gamma = regime_gamma.max(0.01);
        let sigma_sq = sigma.powi(2).max(1e-12);
        let tau_safe = tau.max(0.001); // Prevent division by zero at session end

        // ── Normalized inventory ──
        let q = inventory / c.position_limit.max(1.0);

        // ── Reservation price (Avellaneda-Stoikov 2008) ──
        // r = fair_value - q × γ × σ² × τ
        let reservation = fair_value - q * gamma * sigma_sq * tau_safe;

        // ── Optimal half-spread ──
        // δ = (γ × σ² × τ)/2 + (1/γ) × ln(1 + γ/κ)
        let spread_component_time = (gamma * sigma_sq * tau_safe) / 2.0;
        let spread_component_arrival = (1.0 / gamma) * (1.0 + gamma / c.kappa.max(0.1)).ln();
        let raw_half_spread =
            (spread_component_time + spread_component_arrival).max(c.min_half_spread);

        // ── Spread adjustments ──
        // Regime multiplier (RegimeFolio 2025)
        let mut spread_mult = regime_spread_mult;

        // VPIN toxicity widening (Easley et al. 2012)
        // When VPIN is high, widen spreads to compensate for adverse selection
        if vpin > 0.5 {
            spread_mult *= 1.0 + (vpin - 0.5) * 2.0; // Up to 2× wider at VPIN=1.0
        }

        // Toxicity widening (Barzykin 2025)
        if toxicity > 0.35 {
            spread_mult *= 2.0 + (toxicity - 0.35) * 5.0;
        }

        let adjusted_half = raw_half_spread * spread_mult;

        // ── OFI skew (Cont et al. 2014) ──
        // Positive OFI (buy pressure) → raise both quotes
        let ofi_adj = c.lambda_ofi * ofi;

        // ── Final quotes ──
        let bid = reservation - adjusted_half + ofi_adj;
        let ask = reservation + adjusted_half + ofi_adj;

        // ── Position sizing ──
        // Kelly-based with regime and toxicity scaling
        let toxicity_discount = (1.0 - toxicity * 2.0).max(0.0);
        let bid_size = (base_size * regime_size_mult * toxicity_discount).max(0.0);
        let ask_size = (base_size * regime_size_mult * toxicity_discount).max(0.0);

        // Skew sizes by inventory: reduce size on the side that would increase risk
        let (bid_size, ask_size) = if inventory > 0.0 {
            // Long → reduce bid size, increase ask size
            let inv_ratio = (inventory / c.position_limit).min(1.0);
            (
                bid_size * (1.0 - inv_ratio * 0.5),
                ask_size * (1.0 + inv_ratio * 0.3),
            )
        } else if inventory < 0.0 {
            // Short → increase bid size, reduce ask size
            let inv_ratio = (-inventory / c.position_limit).min(1.0);
            (
                bid_size * (1.0 + inv_ratio * 0.3),
                ask_size * (1.0 - inv_ratio * 0.5),
            )
        } else {
            (bid_size, ask_size)
        };

        QuotingDecision {
            fair_value,
            reservation_price: reservation,
            bid,
            ask,
            raw_half_spread,
            adjusted_half_spread: adjusted_half,
            bid_size,
            ask_size,
            active: true,
            halt_reason: "",
        }
    }

    fn halt_decision(&self, fair_value: f64, reason: &'static str) -> QuotingDecision {
        QuotingDecision {
            fair_value,
            reservation_price: fair_value,
            bid: fair_value - 1000.0, // Far away — effectively no quote
            ask: fair_value + 1000.0,
            raw_half_spread: 0.0,
            adjusted_half_spread: 0.0,
            bid_size: 0.0,
            ask_size: 0.0,
            active: false,
            halt_reason: reason,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_engine() -> QuotingEngine {
        QuotingEngine::new(QuotingConfig::default())
    }

    #[test]
    fn test_basic_quotes_bracket_fv() {
        let engine = default_engine();
        let q = engine.compute(
            100.0, // fair value
            0.0,   // no inventory
            0.01,  // vol
            0.5,   // half session remaining
            0.0,   // no OFI
            0.0,   // no VPIN
            0.10,  // normal gamma
            1.0,   // normal spread mult
            1.0,   // normal size mult
            0.0,   // no toxicity
            10.0,  // base size
        );

        assert!(q.active);
        assert!(q.bid < 100.0, "Bid should be below FV: {}", q.bid);
        assert!(q.ask > 100.0, "Ask should be above FV: {}", q.ask);
        assert!(q.ask > q.bid, "Ask > Bid: ask={}, bid={}", q.ask, q.bid);
    }

    #[test]
    fn test_inventory_skews_reservation() {
        let engine = default_engine();

        let long = engine.compute(100.0, 50.0, 0.01, 0.5, 0.0, 0.0, 0.10, 1.0, 1.0, 0.0, 10.0);
        let flat = engine.compute(100.0, 0.0, 0.01, 0.5, 0.0, 0.0, 0.10, 1.0, 1.0, 0.0, 10.0);
        let short = engine.compute(100.0, -50.0, 0.01, 0.5, 0.0, 0.0, 0.10, 1.0, 1.0, 0.0, 10.0);

        // Long inventory → reservation below FV (want to sell)
        assert!(
            long.reservation_price < flat.reservation_price,
            "Long should skew down: long={}, flat={}",
            long.reservation_price,
            flat.reservation_price
        );
        // Short inventory → reservation above FV (want to buy)
        assert!(
            short.reservation_price > flat.reservation_price,
            "Short should skew up: short={}, flat={}",
            short.reservation_price,
            flat.reservation_price
        );
    }

    #[test]
    fn test_high_vol_widens_spread() {
        // Use zero floor so vol differences aren't clamped
        let engine = QuotingEngine::new(QuotingConfig {
            min_half_spread: 0.0,
            ..QuotingConfig::default()
        });

        let low_vol = engine.compute(100.0, 0.0, 0.005, 0.5, 0.0, 0.0, 0.10, 1.0, 1.0, 0.0, 10.0);
        let high_vol = engine.compute(100.0, 0.0, 0.05, 0.5, 0.0, 0.0, 0.10, 1.0, 1.0, 0.0, 10.0);

        let spread_low = low_vol.ask - low_vol.bid;
        let spread_high = high_vol.ask - high_vol.bid;

        assert!(
            spread_high > spread_low,
            "High vol should widen: low={}, high={}",
            spread_low,
            spread_high
        );
    }

    #[test]
    fn test_toxicity_halts_at_threshold() {
        let engine = default_engine();
        let q = engine.compute(100.0, 0.0, 0.01, 0.5, 0.0, 0.0, 0.10, 1.0, 1.0, 0.70, 10.0);

        assert!(!q.active, "Should halt at toxicity > 0.65");
        assert_eq!(q.halt_reason, "adverse_selection_halt");
        assert_eq!(q.bid_size, 0.0);
        assert_eq!(q.ask_size, 0.0);
    }

    #[test]
    fn test_vpin_widens_spread() {
        let engine = default_engine();

        let safe = engine.compute(100.0, 0.0, 0.01, 0.5, 0.0, 0.1, 0.10, 1.0, 1.0, 0.0, 10.0);
        let toxic = engine.compute(100.0, 0.0, 0.01, 0.5, 0.0, 0.7, 0.10, 1.0, 1.0, 0.0, 10.0);

        let spread_safe = safe.ask - safe.bid;
        let spread_toxic = toxic.ask - toxic.bid;

        assert!(
            spread_toxic > spread_safe,
            "High VPIN should widen: safe={}, toxic={}",
            spread_safe,
            spread_toxic
        );
    }

    #[test]
    fn test_ofi_skews_quotes() {
        let engine = default_engine();

        let neutral = engine.compute(100.0, 0.0, 0.01, 0.5, 0.0, 0.0, 0.10, 1.0, 1.0, 0.0, 10.0);
        let buy_pressure =
            engine.compute(100.0, 0.0, 0.01, 0.5, 0.8, 0.0, 0.10, 1.0, 1.0, 0.0, 10.0);

        // Positive OFI → both quotes shift down (OFI adjustment is subtracted)
        assert!(
            buy_pressure.bid > neutral.bid,
            "Buy pressure should shift bid: neutral={}, ofi={}",
            neutral.bid,
            buy_pressure.bid
        );
    }

    #[test]
    fn test_regime_crisis_reduces_size() {
        let engine = default_engine();

        let normal = engine.compute(100.0, 0.0, 0.01, 0.5, 0.0, 0.0, 0.10, 1.0, 1.0, 0.0, 10.0);
        let crisis = engine.compute(100.0, 0.0, 0.01, 0.5, 0.0, 0.0, 0.40, 4.0, 0.2, 0.0, 10.0);

        assert!(
            crisis.bid_size < normal.bid_size,
            "Crisis should reduce size: normal={}, crisis={}",
            normal.bid_size,
            crisis.bid_size
        );
    }
}
