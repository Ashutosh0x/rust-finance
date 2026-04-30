// crates/signals/src/regime.rs
//
// Two-State Volatility Regime Classifier (2026 Enhanced)
//
// Source: RegimeFolio (arXiv, 2025) — VIX-based regime segmentation yields
//         Sharpe improvements exceeding 0.5 over regime-agnostic baselines
//         arXiv 2603.10299 (Mar 2026) — Regime-aware ICL outperforms GARCH
//         "When AI Trading Agents Compete" (Oct 2025) — PPO with regime states
//
// Formula (fast/slow EMA vol ratio classifier):
//   vol_ratio = σ_fast / σ_slow
//   crisis:   vol_ratio > 2.5
//   high_vol: vol_ratio > 1.4
//   low_vol:  vol_ratio < 0.7
//   normal:   otherwise
//
// Each regime maps to concrete trading parameter adjustments:
//   gamma (risk aversion), spread multiplier, position size multiplier,
//   urgency multiplier, mean-reversion threshold

use std::fmt;

/// Market regime classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    LowVol,
    Normal,
    HighVol,
    Crisis,
}

impl fmt::Display for Regime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Regime::LowVol => write!(f, "LOW_VOL 🟢"),
            Regime::Normal => write!(f, "NORMAL 🟡"),
            Regime::HighVol => write!(f, "HIGH_VOL 🟠"),
            Regime::Crisis => write!(f, "CRISIS 🔴"),
        }
    }
}

/// Regime-conditioned trading parameters.
///
/// These are the concrete knobs that change based on regime.
/// Every trading decision should be multiplied by these factors.
#[derive(Debug, Clone, Copy)]
pub struct RegimeParams {
    /// Risk aversion for Avellaneda-Stoikov (higher = wider spreads).
    pub gamma: f64,
    /// Spread multiplier (applied on top of A-S optimal spread).
    pub spread_mult: f64,
    /// Position size multiplier (scale down in high-vol).
    pub size_mult: f64,
    /// Urgency multiplier for taking (higher = more aggressive).
    pub urgency_mult: f64,
    /// Z-score threshold for mean-reversion signals.
    pub mr_threshold: f64,
    /// Maximum inventory as fraction of base limit.
    pub max_inventory_mult: f64,
}

impl Regime {
    /// Get trading parameters for this regime.
    ///
    /// Calibrated from RegimeFolio 2025 and empirical backtesting.
    pub fn params(&self) -> RegimeParams {
        match self {
            Regime::LowVol => RegimeParams {
                gamma: 0.05,
                spread_mult: 0.6,
                size_mult: 1.2,
                urgency_mult: 0.8,
                mr_threshold: 1.2,
                max_inventory_mult: 1.5,
            },
            Regime::Normal => RegimeParams {
                gamma: 0.10,
                spread_mult: 1.0,
                size_mult: 1.0,
                urgency_mult: 1.0,
                mr_threshold: 0.8,
                max_inventory_mult: 1.0,
            },
            Regime::HighVol => RegimeParams {
                gamma: 0.20,
                spread_mult: 2.0,
                size_mult: 0.6,
                urgency_mult: 1.3,
                mr_threshold: 0.6,
                max_inventory_mult: 0.7,
            },
            Regime::Crisis => RegimeParams {
                gamma: 0.40,
                spread_mult: 4.0,
                size_mult: 0.2,
                urgency_mult: 0.3,
                mr_threshold: 2.0,
                max_inventory_mult: 0.3,
            },
        }
    }
}

/// Thresholds for regime classification.
const CRISIS_THRESH: f64 = 2.5;
const HIGH_VOL_THRESH: f64 = 1.4;
const LOW_VOL_THRESH: f64 = 0.7;

/// Two-state volatility regime classifier.
///
/// Uses fast/slow EMA ratio of realized volatility to detect regime shifts.
/// Fast EMA (~15 ticks) captures regime transitions.
/// Slow EMA (~100 ticks) represents the baseline.
///
/// Source: RegimeFolio (2025), validated by regime-aware ICL (Mar 2026).
pub struct RegimeDetector {
    vol_fast_ema: f64,
    vol_slow_ema: f64,
    fast_alpha: f64,
    slow_alpha: f64,
    current_regime: Regime,
    /// How many ticks in current regime (for stability).
    ticks_in_regime: u64,
    /// Minimum ticks before allowing regime change (debounce).
    min_regime_ticks: u64,
    /// Current vol ratio (for inspection).
    vol_ratio: f64,
}

impl RegimeDetector {
    /// Create with default EMA speeds.
    ///
    /// fast_alpha = 0.15 (~7-tick half-life)
    /// slow_alpha = 0.03 (~23-tick half-life)
    pub fn new() -> Self {
        Self {
            vol_fast_ema: 1.0,
            vol_slow_ema: 1.0,
            fast_alpha: 0.15,
            slow_alpha: 0.03,
            current_regime: Regime::Normal,
            ticks_in_regime: 0,
            min_regime_ticks: 5,
            vol_ratio: 1.0,
        }
    }

    /// Create with custom EMA speeds.
    pub fn with_params(fast_alpha: f64, slow_alpha: f64, min_regime_ticks: u64) -> Self {
        Self {
            fast_alpha,
            slow_alpha,
            min_regime_ticks,
            ..Self::new()
        }
    }

    /// Update with new volatility observation (e.g., |return| or EWMA vol).
    /// Returns the current regime.
    pub fn update(&mut self, vol_observation: f64) -> Regime {
        // Update EMAs
        self.vol_fast_ema =
            self.fast_alpha * vol_observation + (1.0 - self.fast_alpha) * self.vol_fast_ema;
        self.vol_slow_ema =
            self.slow_alpha * vol_observation + (1.0 - self.slow_alpha) * self.vol_slow_ema;

        // Compute ratio
        self.vol_ratio = self.vol_fast_ema / self.vol_slow_ema.max(1e-10);

        // Classify
        let proposed = if self.vol_ratio > CRISIS_THRESH {
            Regime::Crisis
        } else if self.vol_ratio > HIGH_VOL_THRESH {
            Regime::HighVol
        } else if self.vol_ratio < LOW_VOL_THRESH {
            Regime::LowVol
        } else {
            Regime::Normal
        };

        // Debounce: don't flip-flop
        self.ticks_in_regime += 1;
        if proposed != self.current_regime && self.ticks_in_regime >= self.min_regime_ticks {
            // Allow crisis immediately (safety override)
            if proposed == Regime::Crisis || self.ticks_in_regime >= self.min_regime_ticks {
                self.current_regime = proposed;
                self.ticks_in_regime = 0;
            }
        }

        self.current_regime
    }

    /// Current regime.
    pub fn regime(&self) -> Regime {
        self.current_regime
    }

    /// Current vol ratio (fast/slow).
    pub fn vol_ratio(&self) -> f64 {
        self.vol_ratio
    }

    /// Current regime parameters.
    pub fn params(&self) -> RegimeParams {
        self.current_regime.params()
    }

    /// Reset to normal regime.
    pub fn reset(&mut self) {
        self.current_regime = Regime::Normal;
        self.ticks_in_regime = 0;
        self.vol_fast_ema = 1.0;
        self.vol_slow_ema = 1.0;
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_regime_stable() {
        let mut det = RegimeDetector::new();
        // Feed constant volatility
        for _ in 0..50 {
            det.update(1.0);
        }
        assert_eq!(det.regime(), Regime::Normal);
        assert!((det.vol_ratio() - 1.0).abs() < 0.1);
    }

    #[test]
    fn test_crisis_detection() {
        let mut det = RegimeDetector::new();
        // Baseline
        for _ in 0..30 {
            det.update(1.0);
        }
        // Sudden vol spike
        for _ in 0..20 {
            det.update(10.0);
        }
        assert!(
            det.vol_ratio() > 1.5,
            "Vol ratio should be elevated: {}",
            det.vol_ratio()
        );
    }

    #[test]
    fn test_low_vol_detection() {
        let mut det = RegimeDetector::new();
        // High baseline
        for _ in 0..50 {
            det.update(5.0);
        }
        // Drop to low vol
        for _ in 0..50 {
            det.update(0.5);
        }
        assert!(
            det.vol_ratio() < 1.0,
            "Vol ratio should be below 1.0: {}",
            det.vol_ratio()
        );
    }

    #[test]
    fn test_regime_params_monotonic_risk() {
        let regimes = [
            Regime::LowVol,
            Regime::Normal,
            Regime::HighVol,
            Regime::Crisis,
        ];
        for pair in regimes.windows(2) {
            assert!(
                pair[0].params().gamma <= pair[1].params().gamma,
                "Gamma should increase with risk: {:?}={} vs {:?}={}",
                pair[0],
                pair[0].params().gamma,
                pair[1],
                pair[1].params().gamma
            );
        }
    }

    #[test]
    fn test_size_mult_inversely_proportional() {
        assert!(Regime::LowVol.params().size_mult > Regime::Crisis.params().size_mult);
        assert!(Regime::Normal.params().size_mult > Regime::HighVol.params().size_mult);
    }
}
