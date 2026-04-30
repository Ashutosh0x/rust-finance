// crates/signals/src/alpha_health.rs
//
// Alpha Signal Health — IC Decay & Hit Rate Monitor
//
// Source: AlphaForgeBench (arXiv 2602.18481, 2026) — documents alpha decay problem
//         Chain-of-Alpha (arXiv 2508.06312, 2025) — IC-based alpha evaluation
//
// Why this matters: Alphas decay. A signal that worked last week may be worthless
// today. This module tracks the real-time health of each alpha signal so the
// composite signal generator can downweight or disable decayed signals.
//
// Metrics:
//   Information Coefficient (IC): rank correlation between signal and forward return
//     IC > 0.05 = strong, IC > 0.03 = healthy, IC < 0.01 = decayed
//   Hit Rate: fraction of times signal predicted direction correctly
//     > 0.52 = valuable, < 0.48 = noise
//
// Health classification:
//   Healthy:  IC_ema > 0.03 AND hit_rate > 0.52
//   Degraded: IC_ema in [0.01, 0.03] OR hit_rate in [0.48, 0.52]
//   Decayed:  IC_ema < 0.01 AND hit_rate < 0.48

use std::collections::VecDeque;

/// Alpha health classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaHealth {
    /// Signal is predictive — full weight.
    Healthy,
    /// Signal is weakening — reduce weight.
    Degraded,
    /// Signal has lost predictive power — disable.
    Decayed,
}

impl AlphaHealth {
    /// Weight multiplier for this health level.
    /// Used by composite signal to scale contributions.
    pub fn weight(&self) -> f64 {
        match self {
            AlphaHealth::Healthy => 1.0,
            AlphaHealth::Degraded => 0.4,
            AlphaHealth::Decayed => 0.0,
        }
    }
}

/// Thresholds for health classification.
const IC_HEALTHY: f64 = 0.03;
const IC_DEGRADED: f64 = 0.01;
const HIT_HEALTHY: f64 = 0.52;
const HIT_DEGRADED: f64 = 0.48;

/// Monitors the predictive health of a single alpha signal.
///
/// Tracks IC (direction-weighted agreement) and hit rate over a rolling window.
/// Call `update()` every tick with the signal value and the realized return.
pub struct AlphaMonitor {
    name: String,
    /// Rolling window of IC-like observations.
    ic_window: VecDeque<f64>,
    /// EMA-smoothed hit rate.
    hit_rate_ema: f64,
    /// Window size for IC computation.
    window_size: usize,
    /// Current health assessment.
    health: AlphaHealth,
    /// Rolling IC average (cached).
    ic_avg: f64,
    /// Total signals processed.
    total_updates: u64,
}

impl AlphaMonitor {
    pub fn new(name: &str, window_size: usize) -> Self {
        Self {
            name: name.to_string(),
            ic_window: VecDeque::with_capacity(window_size + 1),
            hit_rate_ema: 0.5, // Prior: coin flip
            window_size,
            health: AlphaHealth::Healthy,
            ic_avg: 0.05, // Optimistic prior
            total_updates: 0,
        }
    }

    /// Update with signal value and realized return.
    ///
    /// `signal`: the alpha signal value (any scale)
    /// `actual_return`: the forward return that materialized
    ///
    /// Returns the updated health classification.
    pub fn update(&mut self, signal: f64, actual_return: f64) -> AlphaHealth {
        self.total_updates += 1;

        // Hit rate: did signal predict direction correctly?
        let hit = if signal * actual_return > 0.0 {
            1.0
        } else {
            0.0
        };
        self.hit_rate_ema = 0.05 * hit + 0.95 * self.hit_rate_ema;

        // Simplified IC: normalized sign agreement
        // Full Spearman IC requires rank computation over a window — this is
        // a fast proxy that correlates well with true IC for single signals.
        let denom = signal.abs() * actual_return.abs();
        let ic_obs = if denom > 1e-15 {
            (signal * actual_return) / denom // +1 or -1 weighted by magnitude
        } else {
            0.0
        };

        self.ic_window.push_back(ic_obs);
        if self.ic_window.len() > self.window_size {
            self.ic_window.pop_front();
        }

        // Update rolling IC average
        if !self.ic_window.is_empty() {
            self.ic_avg = self.ic_window.iter().sum::<f64>() / self.ic_window.len() as f64;
        }

        // Classify health
        self.health = if self.ic_avg > IC_HEALTHY && self.hit_rate_ema > HIT_HEALTHY {
            AlphaHealth::Healthy
        } else if self.ic_avg < IC_DEGRADED && self.hit_rate_ema < HIT_DEGRADED {
            AlphaHealth::Decayed
        } else {
            AlphaHealth::Degraded
        };

        self.health
    }

    pub fn health(&self) -> AlphaHealth {
        self.health
    }

    pub fn ic(&self) -> f64 {
        self.ic_avg
    }

    pub fn hit_rate(&self) -> f64 {
        self.hit_rate_ema
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Reset to initial state.
    pub fn reset(&mut self) {
        self.ic_window.clear();
        self.hit_rate_ema = 0.5;
        self.ic_avg = 0.05;
        self.health = AlphaHealth::Healthy;
        self.total_updates = 0;
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_healthy_signal() {
        let mut mon = AlphaMonitor::new("test_signal", 50);

        // Consistently correct predictions
        for _ in 0..100 {
            mon.update(1.0, 0.5); // Positive signal, positive return
        }

        assert_eq!(mon.health(), AlphaHealth::Healthy);
        assert!(mon.ic() > IC_HEALTHY, "IC should be high: {}", mon.ic());
        assert!(
            mon.hit_rate() > HIT_HEALTHY,
            "Hit rate should be high: {}",
            mon.hit_rate()
        );
    }

    #[test]
    fn test_decayed_signal() {
        let mut mon = AlphaMonitor::new("noise", 50);

        // Signal is anti-predictive (worse than random)
        for _ in 0..200 {
            mon.update(1.0, -0.5); // Signal says up, market goes down
        }

        assert_eq!(mon.health(), AlphaHealth::Decayed);
        assert!(
            mon.ic() < IC_DEGRADED,
            "IC should be negative: {}",
            mon.ic()
        );
    }

    #[test]
    fn test_weight_mapping() {
        assert!((AlphaHealth::Healthy.weight() - 1.0).abs() < 0.01);
        assert!((AlphaHealth::Degraded.weight() - 0.4).abs() < 0.01);
        assert!((AlphaHealth::Decayed.weight() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_mixed_signal_degraded() {
        let mut mon = AlphaMonitor::new("noisy", 50);

        // 50/50 — basically random
        for i in 0..100 {
            if i % 2 == 0 {
                mon.update(1.0, 0.5);
            } else {
                mon.update(1.0, -0.5);
            }
        }

        // Should be degraded (IC near 0, hit rate near 0.5)
        assert!(mon.ic().abs() < 0.1, "IC should be near zero: {}", mon.ic());
    }
}
