// crates/backtest/src/fill_model.rs
//
// Pluggable fill simulation models for backtesting.
//
// Default: FixedSlippage (backward compat with existing 1bp slippage)
// New:     SquareRootImpact (institutional-grade market impact model)
//
// The square-root model is the industry standard used by Almgren-Chriss,
// JPMorgan, and Goldman Sachs for execution cost estimation.
//
// Reference: Almgren & Chriss (2000) "Optimal execution of portfolio transactions"

use super::engine::Bar;

/// Result of a fill simulation.
#[derive(Debug, Clone)]
pub struct FillResult {
    /// The price at which the order is filled (after impact + slippage).
    pub fill_price: f64,
    /// Simulated latency in microseconds.
    pub latency_us: f64,
}

/// Trait for pluggable fill models.
/// Implementations determine how orders are filled in backtesting —
/// the fill price accounts for market impact, spread, and latency.
pub trait FillModel: Send {
    /// Simulate a fill for the given order.
    ///
    /// `order_qty`: signed quantity (positive = buy, negative = sell)
    /// `reference_price`: the price at which execution is attempted (open or close)
    /// `bar`: the current market bar (provides volume, spread info)
    ///
    /// Returns the adjusted fill price and simulated latency.
    fn simulate_fill(
        &self,
        order_qty: f64,
        reference_price: f64,
        bar: &Bar,
    ) -> FillResult;
}

// ─── Fixed Slippage (backward compat) ────────────────────────────

/// Original fixed-rate slippage model.
/// Applies a constant percentage of price as slippage.
/// This is unrealistic but simple — preserved for backward compatibility.
pub struct FixedSlippage {
    /// Slippage as fraction of price (e.g., 0.0001 = 1bp).
    pub rate: f64,
}

impl FixedSlippage {
    pub fn new(rate: f64) -> Self {
        Self { rate }
    }

    /// Default: 1 basis point (matches BacktestConfig::default()).
    pub fn default_1bp() -> Self {
        Self { rate: 0.0001 }
    }
}

impl FillModel for FixedSlippage {
    fn simulate_fill(
        &self,
        order_qty: f64,
        reference_price: f64,
        _bar: &Bar,
    ) -> FillResult {
        let slippage = if order_qty > 0.0 {
            reference_price * self.rate
        } else {
            -reference_price * self.rate
        };

        FillResult {
            fill_price: reference_price + slippage,
            latency_us: 0.0, // instant fills in simple model
        }
    }
}

// ─── Square-Root Impact Model ────────────────────────────────────

/// Institutional-grade market impact model.
///
/// Impact formula:
///   impact = σ × η × sign(q) × |q / ADV|^0.5
///
/// Where:
///   σ   = bar-level volatility estimate (high-low range / mid)
///   η   = impact coefficient (calibrated, typically 0.1–0.5)
///   q   = order quantity
///   ADV = average daily volume (approximated from bar volume)
///
/// Latency is drawn from a deterministic model based on configuration
/// (not randomized to keep backtests reproducible).
///
/// Reference: Almgren & Chriss (2000), Torre & Ferrari (1997)
pub struct SquareRootImpact {
    /// Impact coefficient η. Higher = more impact per unit participation.
    /// Typical values: 0.1 (liquid large-caps) to 0.5 (illiquid small-caps).
    pub impact_coefficient: f64,

    /// Temporary impact decay factor. Fraction of impact that is temporary
    /// (reverts after execution). Range [0, 1]. Default: 0.5.
    pub temporary_fraction: f64,

    /// Simulated fill latency in microseconds (deterministic for reproducibility).
    pub latency_us: f64,

    /// Minimum impact in basis points (fee floor — can't execute for free).
    pub min_impact_bps: f64,
}

impl SquareRootImpact {
    /// Create with default institutional parameters.
    pub fn new(impact_coefficient: f64) -> Self {
        Self {
            impact_coefficient,
            temporary_fraction: 0.5,
            latency_us: 500.0,   // 500μs baseline
            min_impact_bps: 0.5, // 0.5bp minimum
        }
    }

    /// Liquid large-cap preset (SPY, AAPL, etc.)
    pub fn liquid() -> Self {
        Self::new(0.1)
    }

    /// Mid-cap preset
    pub fn mid_cap() -> Self {
        Self::new(0.25)
    }

    /// Illiquid / small-cap preset
    pub fn illiquid() -> Self {
        Self::new(0.5)
    }

    fn estimate_volatility(bar: &Bar) -> f64 {
        let mid = (bar.high + bar.low) / 2.0;
        if mid > 0.0 {
            (bar.high - bar.low) / mid
        } else {
            0.01 // fallback 1%
        }
    }
}

impl FillModel for SquareRootImpact {
    fn simulate_fill(
        &self,
        order_qty: f64,
        reference_price: f64,
        bar: &Bar,
    ) -> FillResult {
        let abs_qty = order_qty.abs();
        let sign = if order_qty > 0.0 { 1.0 } else { -1.0 };

        // Participation rate: fraction of bar volume
        let adv = bar.volume.max(1.0);
        let participation = abs_qty / adv;

        // Volatility from bar range
        let sigma = Self::estimate_volatility(bar);

        // Square-root impact: σ × η × sqrt(participation)
        let impact_frac = sigma * self.impact_coefficient * participation.sqrt();

        // Apply minimum impact floor
        let min_impact_frac = self.min_impact_bps / 10_000.0;
        let total_impact = impact_frac.max(min_impact_frac);

        // Permanent + temporary impact decomposition
        // For backtest purposes, we apply the full impact to the fill price.
        // In reality, temporary impact would revert — but for P&L accounting
        // at fill time, we charge the full amount.
        let fill_price = reference_price * (1.0 + sign * total_impact);

        FillResult {
            fill_price,
            latency_us: self.latency_us,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bar(price: f64, volume: f64) -> Bar {
        Bar {
            timestamp: 0,
            symbol: "TEST".into(),
            open: price,
            high: price * 1.01,
            low: price * 0.99,
            close: price,
            volume,
            bid: price - 0.05,
            ask: price + 0.05,
        }
    }

    #[test]
    fn test_fixed_slippage_buy() {
        let model = FixedSlippage::default_1bp();
        let bar = make_bar(100.0, 1_000_000.0);
        let result = model.simulate_fill(100.0, 100.0, &bar);
        assert!(result.fill_price > 100.0, "Buy should have positive slippage");
        assert!((result.fill_price - 100.01).abs() < 0.001, "1bp of 100 = 0.01");
    }

    #[test]
    fn test_fixed_slippage_sell() {
        let model = FixedSlippage::default_1bp();
        let bar = make_bar(100.0, 1_000_000.0);
        let result = model.simulate_fill(-100.0, 100.0, &bar);
        assert!(result.fill_price < 100.0, "Sell should have negative slippage");
    }

    #[test]
    fn test_sqrt_impact_larger_order_more_impact() {
        let model = SquareRootImpact::liquid();
        let bar = make_bar(100.0, 1_000_000.0);

        let small = model.simulate_fill(100.0, 100.0, &bar);
        let large = model.simulate_fill(100_000.0, 100.0, &bar);

        assert!(
            large.fill_price > small.fill_price,
            "Larger order should have more impact: small={:.6}, large={:.6}",
            small.fill_price,
            large.fill_price
        );
    }

    #[test]
    fn test_sqrt_impact_illiquid_more_impact() {
        let liquid = SquareRootImpact::liquid();
        let illiquid = SquareRootImpact::illiquid();
        let bar = make_bar(100.0, 1_000_000.0);

        let fill_liquid = liquid.simulate_fill(10_000.0, 100.0, &bar);
        let fill_illiquid = illiquid.simulate_fill(10_000.0, 100.0, &bar);

        assert!(
            fill_illiquid.fill_price > fill_liquid.fill_price,
            "Illiquid should have more impact"
        );
    }

    #[test]
    fn test_sqrt_impact_sell_pushes_price_down() {
        let model = SquareRootImpact::mid_cap();
        let bar = make_bar(100.0, 1_000_000.0);
        let result = model.simulate_fill(-10_000.0, 100.0, &bar);
        assert!(
            result.fill_price < 100.0,
            "Sell should push fill price below reference"
        );
    }

    #[test]
    fn test_sqrt_impact_low_volume_more_impact() {
        let model = SquareRootImpact::mid_cap();
        let high_vol = make_bar(100.0, 10_000_000.0);
        let low_vol = make_bar(100.0, 100_000.0);

        let fill_high = model.simulate_fill(1_000.0, 100.0, &high_vol);
        let fill_low = model.simulate_fill(1_000.0, 100.0, &low_vol);

        assert!(
            fill_low.fill_price > fill_high.fill_price,
            "Lower volume should produce more impact"
        );
    }
}
