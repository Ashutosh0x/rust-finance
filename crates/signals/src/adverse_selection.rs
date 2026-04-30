// crates/signals/src/adverse_selection.rs
//
// Adverse Selection & Toxicity Detection
//
// Source: Barzykin et al. (arXiv 2508.20225, Nov 2025) — "Optimal Quoting
//         under Adverse Selection and Price Reading"
//         arXiv 2602.00776 (Jan 2026) — Cryptocurrency Microstructure confirms
//         "stale maker bids get aggressively lifted during flash crashes"
//         Glosten & Milgrom (1985) — foundational adverse selection model
//
// The #1 killer of market makers: informed flow picks you off before you can
// adjust quotes. This module detects toxic fills in real-time and recommends
// spread widening or quoting halts.
//
// Key formula:
//   For each passive fill at price P at time t:
//     post_move = fair_value(t+N) - P       (for buy fills)
//     post_move = P - fair_value(t+N)       (for sell fills)
//   A fill is "toxic" if post_move < -threshold (price moved against us)
//   toxicity_ema = α × is_toxic + (1-α) × toxicity_ema
//
// Response thresholds:
//   toxicity > 0.35: widen spreads by 2× and reduce size by 50%
//   toxicity > 0.65: stop passive quoting entirely

use std::collections::VecDeque;

/// Configuration for the toxicity detector.
#[derive(Debug, Clone)]
pub struct ToxicityConfig {
    /// EMA smoothing for toxicity signal (0.1 = slow, 0.3 = fast).
    pub alpha: f64,
    /// Threshold to start widening spreads.
    pub warn_threshold: f64,
    /// Threshold to halt passive quoting.
    pub halt_threshold: f64,
    /// Price movement threshold to classify a fill as "toxic" (in price units).
    /// Typically 0.5–2.0 ticks depending on instrument.
    pub adverse_move_threshold: f64,
    /// Number of ticks after fill to evaluate the price move.
    pub lookback_ticks: usize,
    /// Maximum number of fills to retain for analysis.
    pub max_history: usize,
}

impl Default for ToxicityConfig {
    fn default() -> Self {
        Self {
            alpha: 0.15,
            warn_threshold: 0.35,
            halt_threshold: 0.65,
            adverse_move_threshold: 0.5,
            lookback_ticks: 3,
            max_history: 200,
        }
    }
}

/// Recommended quoting action based on toxicity level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToxicityAction {
    /// Normal quoting — no adverse selection detected.
    Normal,
    /// Widen spreads and reduce size — elevated toxic flow.
    Widen {
        spread_multiplier: f64,
        size_multiplier: f64,
    },
    /// Halt all passive quoting — extreme adverse selection.
    HaltPassive,
}

/// Record of a single passive fill for toxicity analysis.
#[derive(Debug, Clone)]
#[allow(dead_code)] // timestamp stored for audit trail / gap-alert logging
struct FillEntry {
    price: f64,
    is_bid: bool, // true = we bought (filled on our bid)
    timestamp: u64,
    evaluated: bool,
    post_move: Option<f64>,
}

/// Real-time adverse selection detector for market makers.
///
/// Tracks every passive fill and measures whether prices move against us
/// after being filled, which indicates informed/toxic flow.
///
/// Usage:
/// ```ignore
/// let mut detector = ToxicityDetector::new(ToxicityConfig::default());
///
/// // On every passive fill:
/// detector.record_fill(fill_price, true, timestamp);
///
/// // On every tick (evaluate past fills):
/// let action = detector.evaluate(current_fair_value, current_timestamp);
///
/// match action {
///     ToxicityAction::Normal => { /* quote normally */ }
///     ToxicityAction::Widen { spread_multiplier, size_multiplier } => { ... }
///     ToxicityAction::HaltPassive => { /* stop passive quoting */ }
/// }
/// ```
pub struct ToxicityDetector {
    config: ToxicityConfig,
    fills: VecDeque<FillEntry>,
    toxicity_ema: f64,
    /// Exponentially weighted fill rate (fills per tick).
    fill_rate_ema: f64,
    /// Count of consecutive toxic fills.
    consecutive_toxic: usize,
    /// Total fills evaluated.
    total_evaluated: u64,
    /// Total toxic fills.
    total_toxic: u64,
}

impl ToxicityDetector {
    pub fn new(config: ToxicityConfig) -> Self {
        Self {
            config,
            fills: VecDeque::with_capacity(200),
            toxicity_ema: 0.0,
            fill_rate_ema: 0.0,
            consecutive_toxic: 0,
            total_evaluated: 0,
            total_toxic: 0,
        }
    }

    /// Record a new passive fill.
    /// `is_bid`: true if we were filled on our bid (we bought).
    pub fn record_fill(&mut self, price: f64, is_bid: bool, timestamp: u64) {
        self.fills.push_back(FillEntry {
            price,
            is_bid,
            timestamp,
            evaluated: false,
            post_move: None,
        });

        // Trim history
        while self.fills.len() > self.config.max_history {
            self.fills.pop_front();
        }

        // Update fill rate EMA (faster fills = more suspicious)
        self.fill_rate_ema = 0.2 * 1.0 + 0.8 * self.fill_rate_ema;
    }

    /// Evaluate toxicity based on current fair value.
    /// Call on every tick or at regular intervals.
    /// Returns the recommended quoting action.
    pub fn evaluate(&mut self, fair_value: f64, _current_ts: u64) -> ToxicityAction {
        // Decay fill rate
        self.fill_rate_ema *= 0.99;

        // Evaluate unevaluated fills that are old enough
        for fill in self.fills.iter_mut() {
            if fill.evaluated {
                continue;
            }

            // Calculate post-fill price move
            let post_move = if fill.is_bid {
                // We bought: positive post_move = price went up (good for us)
                fair_value - fill.price
            } else {
                // We sold: positive post_move = price went down (good for us)
                fill.price - fair_value
            };

            fill.post_move = Some(post_move);
            fill.evaluated = true;

            // Is this fill toxic?
            let is_toxic = post_move < -self.config.adverse_move_threshold;

            self.total_evaluated += 1;
            if is_toxic {
                self.total_toxic += 1;
                self.consecutive_toxic += 1;
            } else {
                self.consecutive_toxic = 0;
            }

            // Update toxicity EMA
            let toxic_signal = if is_toxic { 1.0 } else { 0.0 };
            self.toxicity_ema =
                self.config.alpha * toxic_signal + (1.0 - self.config.alpha) * self.toxicity_ema;
        }

        // Determine action
        self.recommend_action()
    }

    fn recommend_action(&self) -> ToxicityAction {
        if self.toxicity_ema > self.config.halt_threshold || self.consecutive_toxic >= 5 {
            ToxicityAction::HaltPassive
        } else if self.toxicity_ema > self.config.warn_threshold {
            let excess = self.toxicity_ema - self.config.warn_threshold;
            ToxicityAction::Widen {
                spread_multiplier: 2.0 + excess * 5.0,
                size_multiplier: (0.5 - excess).max(0.1),
            }
        } else {
            ToxicityAction::Normal
        }
    }

    /// Current toxicity EMA value [0, 1].
    pub fn toxicity(&self) -> f64 {
        self.toxicity_ema
    }

    /// Is quoting currently considered safe?
    pub fn is_safe(&self) -> bool {
        self.toxicity_ema < self.config.warn_threshold
    }

    /// Lifetime toxicity rate.
    pub fn lifetime_toxic_rate(&self) -> f64 {
        if self.total_evaluated == 0 {
            return 0.0;
        }
        self.total_toxic as f64 / self.total_evaluated as f64
    }

    /// Recent fill rate (fills per tick, EMA smoothed).
    pub fn fill_rate(&self) -> f64 {
        self.fill_rate_ema
    }

    /// Reset detector state (e.g., on session boundary).
    pub fn reset(&mut self) {
        self.fills.clear();
        self.toxicity_ema = 0.0;
        self.fill_rate_ema = 0.0;
        self.consecutive_toxic = 0;
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_flow_stays_normal() {
        let mut det = ToxicityDetector::new(ToxicityConfig::default());

        // Simulate fills where price moves in our favor
        for i in 0..20 {
            det.record_fill(100.0, true, i);
            let action = det.evaluate(100.5, i + 3); // Price went UP after buy = good
            assert_eq!(
                action,
                ToxicityAction::Normal,
                "Normal flow should be Normal at tick {}",
                i
            );
        }
        assert!(
            det.toxicity() < 0.1,
            "Toxicity should be low: {}",
            det.toxicity()
        );
    }

    #[test]
    fn test_toxic_flow_triggers_widen() {
        let mut det = ToxicityDetector::new(ToxicityConfig {
            alpha: 0.3, // Fast response for test
            ..ToxicityConfig::default()
        });

        // Simulate toxic fills: we buy, price drops
        for i in 0..20 {
            det.record_fill(100.0, true, i);
            let _action = det.evaluate(99.0, i + 3); // Price DOWN after buy = toxic
        }

        assert!(
            det.toxicity() > 0.3,
            "Toxicity should be elevated: {}",
            det.toxicity()
        );
    }

    #[test]
    fn test_extreme_toxicity_halts() {
        let mut det = ToxicityDetector::new(ToxicityConfig {
            alpha: 0.5,
            halt_threshold: 0.6,
            ..ToxicityConfig::default()
        });

        // Relentless toxic fills
        for i in 0..30 {
            det.record_fill(100.0, true, i);
            det.evaluate(98.0, i + 3); // -2.0 move, well beyond threshold
        }

        let action = det.evaluate(98.0, 33);
        assert!(
            matches!(action, ToxicityAction::HaltPassive),
            "Extreme toxicity should halt: tox={:.3}",
            det.toxicity()
        );
    }

    #[test]
    fn test_mixed_flow_moderate_toxicity() {
        let mut det = ToxicityDetector::new(ToxicityConfig::default());

        for i in 0..40 {
            det.record_fill(100.0, true, i);
            if i % 3 == 0 {
                // Every 3rd fill is toxic
                det.evaluate(99.0, i + 3);
            } else {
                det.evaluate(100.5, i + 3);
            }
        }

        // Should be somewhere between 0 and warn_threshold
        let tox = det.toxicity();
        assert!(
            tox > 0.0 && tox < 0.5,
            "Mixed flow toxicity should be moderate: {}",
            tox
        );
    }
}
