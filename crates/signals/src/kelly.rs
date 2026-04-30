// crates/signals/src/kelly.rs
//
// Kelly Criterion + Bayesian Shrinkage Position Sizing
//
// Source: Kelly (1956), Thorp (2008) — foundational optimal growth theory
//         PolySwarm (arXiv 2604.03888, Apr 2026) — uses quarter-Kelly explicitly
//         EdgeTools (Aug 2025) — Bayesian shrinkage implementation
//         "Sizing the Risk" (arXiv 2508.16598, 2025) — Kelly + VIX hybrid
//
// Key insight from research consensus:
//   Quarter-Kelly (f* × 0.25) captures 56% of max geometric growth rate
//   with 75% less portfolio volatility. This is the 2026 industry standard.
//
// Final position sizing formula (2026 synthesis):
//   size = kelly_base × conviction × regime_size_mult × (1 - toxicity × 2)

/// Kelly fraction for discrete outcomes (win/loss model).
///
/// f* = (p × b - q) / b
/// where p = win probability, b = win/loss ratio, q = 1 - p
///
/// `fraction`: Kelly multiplier (0.25 = quarter-Kelly per PolySwarm 2026).
///
/// Returns the optimal fraction of capital to risk [0, 1].
#[inline]
pub fn kelly_fraction(win_prob: f64, win_loss_ratio: f64, fraction: f64) -> f64 {
    if win_loss_ratio <= 0.0 || win_prob <= 0.0 || win_prob >= 1.0 {
        return 0.0;
    }
    let q = 1.0 - win_prob;
    let full_kelly = (win_prob * win_loss_ratio - q) / win_loss_ratio;
    full_kelly.max(0.0) * fraction
}

/// Continuous Kelly for market making / spread capture (Thorp/Markowitz form).
///
/// f* = μ / (γ × σ²)
/// where μ = expected PnL per unit, σ = PnL volatility, γ = risk aversion.
///
/// `fraction`: Kelly multiplier (0.25 recommended).
#[inline]
pub fn kelly_continuous(mu: f64, sigma: f64, gamma: f64, fraction: f64) -> f64 {
    if sigma <= 1e-10 || gamma <= 0.0 {
        return 0.0;
    }
    let full_kelly = mu / (gamma * sigma * sigma);
    full_kelly.max(0.0) * fraction
}

/// Kelly with Bayesian shrinkage for small sample sizes.
///
/// Source: EdgeTools Aug 2025, implementing Michaud (1989).
///
/// With few trades, shrinks toward zero (don't over-bet on limited data).
/// With many trades, converges to standard fractional Kelly.
///
/// f_shrunk = f* × fraction × (n / (n + n_prior))
///
/// `n_trades`: number of historical trades
/// `n_prior`: prior strength (30 = moderate, 100 = conservative)
pub fn kelly_bayesian(
    win_prob: f64,
    win_loss_ratio: f64,
    n_trades: usize,
    n_prior: usize,
    fraction: f64,
) -> f64 {
    let base_f = kelly_fraction(win_prob, win_loss_ratio, 1.0);
    let shrinkage = n_trades as f64 / (n_trades + n_prior).max(1) as f64;
    (base_f * fraction * shrinkage).max(0.0)
}

/// Final conviction-scaled position quantity.
///
/// Combines Kelly base with:
/// - Signal conviction (strength of the trading signal)
/// - Toxicity discount (reduce size under adverse selection)
/// - Regime size multiplier (from regime detector)
///
/// This is the 2026 master sizing formula:
///   qty = base_kelly × limit × conviction × regime_mult × (1 - toxicity × 2)
pub fn conviction_sized_qty(
    base_kelly: f64,
    position_limit: f64,
    conviction: f64,
    toxicity: f64,
    regime_size_mult: f64,
) -> f64 {
    let raw_qty = base_kelly * position_limit * conviction * regime_size_mult;
    let toxicity_discount = (1.0 - toxicity * 2.0).max(0.0);
    let final_qty = raw_qty * toxicity_discount;
    final_qty.max(0.0).min(position_limit)
}

/// Win rate and win/loss ratio tracker for Kelly input.
pub struct KellyTracker {
    wins: u64,
    losses: u64,
    total_win_pnl: f64,
    total_loss_pnl: f64,
}

impl KellyTracker {
    pub fn new() -> Self {
        Self {
            wins: 0,
            losses: 0,
            total_win_pnl: 0.0,
            total_loss_pnl: 0.0,
        }
    }

    /// Record a trade result.
    pub fn record(&mut self, pnl: f64) {
        if pnl > 0.0 {
            self.wins += 1;
            self.total_win_pnl += pnl;
        } else if pnl < 0.0 {
            self.losses += 1;
            self.total_loss_pnl += pnl.abs();
        }
    }

    /// Win probability.
    pub fn win_prob(&self) -> f64 {
        let total = self.wins + self.losses;
        if total == 0 {
            return 0.5; // Prior
        }
        self.wins as f64 / total as f64
    }

    /// Average win / average loss ratio.
    pub fn win_loss_ratio(&self) -> f64 {
        let avg_win = if self.wins > 0 {
            self.total_win_pnl / self.wins as f64
        } else {
            1.0
        };
        let avg_loss = if self.losses > 0 {
            self.total_loss_pnl / self.losses as f64
        } else {
            1.0
        };
        if avg_loss < 1e-10 {
            return 1.0;
        }
        avg_win / avg_loss
    }

    /// Total number of trades.
    pub fn n_trades(&self) -> usize {
        (self.wins + self.losses) as usize
    }

    /// Compute optimal Kelly fraction with Bayesian shrinkage.
    pub fn optimal_fraction(&self, kelly_mult: f64) -> f64 {
        kelly_bayesian(self.win_prob(), self.win_loss_ratio(), self.n_trades(), 30, kelly_mult)
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kelly_basic() {
        // 60% win rate, 1:1 payoff → f* = (0.6×1 - 0.4)/1 = 0.2
        let f = kelly_fraction(0.6, 1.0, 1.0);
        assert!((f - 0.2).abs() < 0.001, "Full Kelly: {}", f);
    }

    #[test]
    fn test_quarter_kelly() {
        let f = kelly_fraction(0.6, 1.0, 0.25);
        assert!((f - 0.05).abs() < 0.001, "Quarter Kelly: {}", f);
    }

    #[test]
    fn test_kelly_negative_edge() {
        // 40% win rate, 1:1 → negative edge → f* = 0
        let f = kelly_fraction(0.4, 1.0, 0.25);
        assert!(f <= 0.0, "Negative edge should be 0: {}", f);
    }

    #[test]
    fn test_bayesian_shrinkage() {
        let few_trades = kelly_bayesian(0.6, 1.5, 5, 30, 0.25);
        let many_trades = kelly_bayesian(0.6, 1.5, 200, 30, 0.25);
        assert!(many_trades > few_trades,
            "More trades should mean larger bet: few={}, many={}", few_trades, many_trades);
    }

    #[test]
    fn test_conviction_sizing_scales() {
        let full_conv = conviction_sized_qty(0.1, 100.0, 1.0, 0.0, 1.0);
        let half_conv = conviction_sized_qty(0.1, 100.0, 0.5, 0.0, 1.0);
        assert!((full_conv / half_conv - 2.0).abs() < 0.01,
            "Double conviction = double size");
    }

    #[test]
    fn test_toxicity_reduces_size() {
        let safe = conviction_sized_qty(0.1, 100.0, 1.0, 0.0, 1.0);
        let toxic = conviction_sized_qty(0.1, 100.0, 1.0, 0.4, 1.0);
        assert!(toxic < safe, "Toxicity should reduce size: safe={}, toxic={}", safe, toxic);
    }

    #[test]
    fn test_kelly_tracker() {
        let mut tracker = KellyTracker::new();
        for _ in 0..60 {
            tracker.record(1.5); // 60 wins of $1.50
        }
        for _ in 0..40 {
            tracker.record(-1.0); // 40 losses of $1.00
        }
        assert!((tracker.win_prob() - 0.6).abs() < 0.01);
        assert!((tracker.win_loss_ratio() - 1.5).abs() < 0.01);
        assert!(tracker.optimal_fraction(0.25) > 0.0);
    }
}
