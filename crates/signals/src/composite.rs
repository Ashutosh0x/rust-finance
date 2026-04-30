// crates/signals/src/composite.rs
//
// Composite Signal Generator — The 2026 Master Formula
//
// Synthesizes all alpha modules into a single trading signal score.
//
// Source: Chain-of-Alpha (arXiv 2508.06312, 2025) — IC-proportional weighting
//         ComSIA (arXiv 2601.19504, Jan 2026) — hybrid technical+sentiment
//         RegimeFolio (2025) — regime-conditioned allocation
//
// Architecture:
//   1. Each signal (OFI, microprice, mean-reversion, momentum) produces a
//      normalized value in [-1, +1].
//   2. Each signal's weight is proportional to its estimated IC (information
//      coefficient), gated by its AlphaHealth status.
//   3. The composite score is regime-adjusted and toxicity-discounted.
//   4. Final output: (signal_score, conviction) where conviction ∈ [0, 1]
//      drives position sizing via Kelly.
//
// Formula:
//   score = Σᵢ (wᵢ × health_wᵢ × signalᵢ) / Σᵢ (wᵢ × health_wᵢ)
//   conviction = min(1.0, |score| × 2)
//
//   Where wᵢ = max(0, IC_i) — IC-weighted (Chain-of-Alpha 2025)
//   health_wᵢ from AlphaMonitor (1.0/0.4/0.0 for healthy/degraded/decayed)

use super::alpha_health::AlphaHealth;

/// A single signal contribution to the composite.
#[derive(Debug, Clone)]
pub struct SignalInput {
    /// Signal name (for logging/attribution).
    pub name: String,
    /// Signal value, normalized to [-1, +1].
    pub value: f64,
    /// Estimated information coefficient (from backtesting or online tracking).
    pub ic: f64,
    /// Current alpha health status.
    pub health: AlphaHealth,
}

/// Output of the composite signal generator.
#[derive(Debug, Clone, Copy)]
pub struct CompositeOutput {
    /// Combined signal score in [-1, +1].
    /// Positive = buy pressure, negative = sell pressure.
    pub score: f64,
    /// Conviction level in [0, 1].
    /// Drives position sizing (higher = larger positions).
    pub conviction: f64,
    /// Number of active (non-decayed) signals contributing.
    pub active_signals: usize,
    /// Total effective weight (sum of IC × health_weight).
    pub total_weight: f64,
}

/// Composite signal generator combining multiple alpha sources.
///
/// Usage:
/// ```ignore
/// let mut gen = CompositeSignal::new();
///
/// let output = gen.compute(&[
///     SignalInput { name: "OFI".into(), value: ofi, ic: 0.06, health: AlphaHealth::Healthy },
///     SignalInput { name: "MR".into(), value: -z_score/3.0, ic: 0.05, health: health_mr },
///     SignalInput { name: "MACD".into(), value: macd_norm, ic: 0.04, health: health_macd },
/// ], regime, toxicity);
/// ```
pub struct CompositeSignal {
    /// Regime scaling factors.
    regime_scaling: [f64; 4], // LowVol, Normal, HighVol, Crisis
}

impl CompositeSignal {
    pub fn new() -> Self {
        Self {
            // From RegimeFolio 2025: scale signal strength by regime
            regime_scaling: [0.7, 1.0, 0.8, 0.1],
        }
    }

    /// Compute composite signal from multiple alpha inputs.
    ///
    /// `regime_idx`: 0=LowVol, 1=Normal, 2=HighVol, 3=Crisis
    /// `toxicity`: current toxicity EMA [0, 1]
    pub fn compute(
        &self,
        signals: &[SignalInput],
        regime_idx: usize,
        toxicity: f64,
    ) -> CompositeOutput {
        if signals.is_empty() {
            return CompositeOutput {
                score: 0.0,
                conviction: 0.0,
                active_signals: 0,
                total_weight: 0.0,
            };
        }

        let mut weighted_sum = 0.0;
        let mut weight_total = 0.0;
        let mut active_count = 0;

        for signal in signals {
            let health_w = signal.health.weight();
            if health_w < 1e-10 {
                continue; // Decayed signal — skip entirely
            }

            let ic_w = signal.ic.max(0.0); // Only positive IC contributes
            if ic_w < 1e-10 {
                continue;
            }

            let effective_w = ic_w * health_w;
            weighted_sum += effective_w * signal.value;
            weight_total += effective_w;
            active_count += 1;
        }

        if weight_total < 1e-10 {
            return CompositeOutput {
                score: 0.0,
                conviction: 0.0,
                active_signals: 0,
                total_weight: 0.0,
            };
        }

        let raw_score = weighted_sum / weight_total;

        // Regime scaling
        let regime_mult = self.regime_scaling.get(regime_idx).copied().unwrap_or(1.0);

        // Toxicity discount
        let toxicity_mult = (1.0 - toxicity * 1.5).max(0.0);

        let final_score = (raw_score * regime_mult * toxicity_mult).clamp(-1.0, 1.0);
        let conviction = (final_score.abs() * 2.0).min(1.0);

        CompositeOutput {
            score: final_score,
            conviction,
            active_signals: active_count,
            total_weight: weight_total,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_signal(name: &str, value: f64, ic: f64, health: AlphaHealth) -> SignalInput {
        SignalInput {
            name: name.to_string(),
            value,
            ic,
            health,
        }
    }

    #[test]
    fn test_single_bullish_signal() {
        let gen = CompositeSignal::new();
        let output = gen.compute(
            &[make_signal("OFI", 0.8, 0.06, AlphaHealth::Healthy)],
            1, // Normal regime
            0.0,
        );
        assert!(
            output.score > 0.0,
            "Bullish OFI → positive score: {}",
            output.score
        );
        assert!(output.conviction > 0.0);
        assert_eq!(output.active_signals, 1);
    }

    #[test]
    fn test_conflicting_signals_cancel() {
        let gen = CompositeSignal::new();
        let output = gen.compute(
            &[
                make_signal("OFI", 1.0, 0.05, AlphaHealth::Healthy), // Buy
                make_signal("MR", -1.0, 0.05, AlphaHealth::Healthy), // Sell
            ],
            1,
            0.0,
        );
        assert!(
            output.score.abs() < 0.01,
            "Conflicting → near zero: {}",
            output.score
        );
    }

    #[test]
    fn test_ic_weighting() {
        let gen = CompositeSignal::new();
        let output = gen.compute(
            &[
                make_signal("strong", 1.0, 0.10, AlphaHealth::Healthy),
                make_signal("weak", -1.0, 0.02, AlphaHealth::Healthy),
            ],
            1,
            0.0,
        );
        // Strong signal (IC=0.10) should dominate weak (IC=0.02)
        assert!(
            output.score > 0.0,
            "High-IC signal should dominate: {}",
            output.score
        );
    }

    #[test]
    fn test_decayed_signal_excluded() {
        let gen = CompositeSignal::new();
        let output = gen.compute(
            &[
                make_signal("good", 0.5, 0.05, AlphaHealth::Healthy),
                make_signal("dead", -1.0, 0.10, AlphaHealth::Decayed), // High IC but decayed
            ],
            1,
            0.0,
        );
        assert!(
            output.score > 0.0,
            "Decayed signal should not contribute: {}",
            output.score
        );
        assert_eq!(output.active_signals, 1);
    }

    #[test]
    fn test_crisis_regime_suppresses() {
        let gen = CompositeSignal::new();
        let normal = gen.compute(
            &[make_signal("OFI", 0.8, 0.06, AlphaHealth::Healthy)],
            1, // Normal
            0.0,
        );
        let crisis = gen.compute(
            &[make_signal("OFI", 0.8, 0.06, AlphaHealth::Healthy)],
            3, // Crisis
            0.0,
        );
        assert!(
            crisis.score.abs() < normal.score.abs(),
            "Crisis should suppress signal: normal={}, crisis={}",
            normal.score,
            crisis.score
        );
    }

    #[test]
    fn test_toxicity_reduces_conviction() {
        let gen = CompositeSignal::new();
        let clean = gen.compute(
            &[make_signal("OFI", 0.8, 0.06, AlphaHealth::Healthy)],
            1,
            0.0,
        );
        let toxic = gen.compute(
            &[make_signal("OFI", 0.8, 0.06, AlphaHealth::Healthy)],
            1,
            0.5,
        );
        assert!(
            toxic.conviction < clean.conviction,
            "Toxicity should reduce conviction: clean={}, toxic={}",
            clean.conviction,
            toxic.conviction
        );
    }

    #[test]
    fn test_empty_signals() {
        let gen = CompositeSignal::new();
        let output = gen.compute(&[], 1, 0.0);
        assert_eq!(output.score, 0.0);
        assert_eq!(output.active_signals, 0);
    }
}
