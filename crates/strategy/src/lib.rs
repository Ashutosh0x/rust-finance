#![forbid(unsafe_code)]
// crates/strategy/src/lib.rs
// v2 Strategy trait + concrete implementations
pub mod market_maker;

use common::events::{BotEvent, Envelope, MarketEvent};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::info;

// ─── v2 Strategy Trait ───────────────────────────────────────────

/// A trade signal produced by a strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeSignal {
    pub symbol: String,
    pub direction: Direction,
    pub quantity: f64,
    pub confidence: f64,
    pub strategy_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Direction {
    Buy,
    Sell,
    Hold,
}

/// v2 strategy trait — processes typed MarketEvent envelopes
pub trait PluggableStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn on_market_event(&mut self, event: &Envelope<MarketEvent>) -> Option<TradeSignal>;
    fn on_ai_signal(&mut self, signal: &BotEvent);
    fn reset(&mut self);
}

// ─── Momentum Strategy ───────────────────────────────────────────

/// Simple momentum: if price is above SMA → BUY, below → SELL
/// Uses VecDeque for O(1) push/pop rolling window (not Vec::remove(0) which is O(n)).
pub struct MomentumStrategy {
    window: VecDeque<f64>,
    running_sum: f64,
    period: usize,
    threshold: f64,
    last_ai_confidence: f64,
}

impl MomentumStrategy {
    pub fn new(period: usize, threshold: f64) -> Self {
        Self {
            window: VecDeque::with_capacity(period + 1),
            running_sum: 0.0,
            period,
            threshold,
            last_ai_confidence: 1.0,
        }
    }

    #[inline]
    fn sma(&self) -> f64 {
        if self.window.is_empty() {
            return 0.0;
        }
        self.running_sum / self.window.len() as f64
    }
}

impl PluggableStrategy for MomentumStrategy {
    fn name(&self) -> &str {
        "Momentum"
    }

    fn on_market_event(&mut self, event: &Envelope<MarketEvent>) -> Option<TradeSignal> {
        let (symbol, price) = match &event.payload {
            MarketEvent::Trade(t) => (t.symbol.to_string(), t.price),
            _ => return None,
        };

        // O(1) rolling window update
        self.window.push_back(price);
        self.running_sum += price;
        if self.window.len() > self.period {
            if let Some(old) = self.window.pop_front() {
                self.running_sum -= old;
            }
        }
        if self.window.len() < self.period {
            return None; // Not enough data
        }

        // AI confidence gate
        if self.last_ai_confidence < 0.65 {
            info!(
                confidence = self.last_ai_confidence,
                "Momentum: AI veto active"
            );
            return None;
        }

        let sma = self.sma();
        let deviation = (price - sma) / sma;

        if deviation > self.threshold {
            Some(TradeSignal {
                symbol: symbol.to_string(),
                direction: Direction::Buy,
                quantity: 1.0,
                confidence: (deviation / self.threshold).min(1.0),
                strategy_id: "momentum".into(),
            })
        } else if deviation < -self.threshold {
            Some(TradeSignal {
                symbol: symbol.to_string(),
                direction: Direction::Sell,
                quantity: 1.0,
                confidence: (-deviation / self.threshold).min(1.0),
                strategy_id: "momentum".into(),
            })
        } else {
            None
        }
    }

    fn on_ai_signal(&mut self, event: &BotEvent) {
        if let BotEvent::AISignal {
            confidence, symbol, ..
        } = event
        {
            info!(symbol, confidence, "Momentum: ingesting AI signal");
            self.last_ai_confidence = *confidence;
        }
    }

    fn reset(&mut self) {
        self.window.clear();
        self.running_sum = 0.0;
        self.last_ai_confidence = 1.0;
    }
}

// ─── Mean Reversion Strategy ─────────────────────────────────────

/// Mean reversion: if price deviates >N std from mean → fade the move.
/// Uses VecDeque for O(1) push/pop, running_sum for O(1) mean,
/// and running_sum_sq for O(1) variance (Welford's online algorithm).
pub struct MeanReversionStrategy {
    window: VecDeque<f64>,
    running_sum: f64,
    running_sum_sq: f64,
    period: usize,
    z_score_threshold: f64,
    last_ai_confidence: f64,
}

impl MeanReversionStrategy {
    pub fn new(period: usize, z_score_threshold: f64) -> Self {
        Self {
            window: VecDeque::with_capacity(period + 1),
            running_sum: 0.0,
            running_sum_sq: 0.0,
            period,
            z_score_threshold,
            last_ai_confidence: 1.0,
        }
    }

    fn mean_and_std(&self) -> (f64, f64) {
        if self.window.is_empty() {
            return (0.0, 0.0);
        }
        let n = self.window.len() as f64;
        let mean = self.running_sum / n;
        // Var(X) = E[X²] - E[X]² — O(1) using running sums
        let variance = (self.running_sum_sq / n) - mean * mean;
        (mean, variance.max(0.0).sqrt())
    }
}

impl PluggableStrategy for MeanReversionStrategy {
    fn name(&self) -> &str {
        "MeanReversion"
    }

    fn on_market_event(&mut self, event: &Envelope<MarketEvent>) -> Option<TradeSignal> {
        let (symbol, price) = match &event.payload {
            MarketEvent::Trade(t) => (t.symbol.to_string(), t.price),
            _ => return None,
        };

        // O(1) rolling window update
        self.window.push_back(price);
        self.running_sum += price;
        self.running_sum_sq += price * price;
        if self.window.len() > self.period {
            if let Some(old) = self.window.pop_front() {
                self.running_sum -= old;
                self.running_sum_sq -= old * old;
            }
        }
        if self.window.len() < self.period {
            return None;
        }

        if self.last_ai_confidence < 0.65 {
            return None;
        }

        let (mean, std) = self.mean_and_std();
        if std < 1e-10 {
            return None;
        }

        let z_score = (price - mean) / std;

        if z_score > self.z_score_threshold {
            // Price too high relative to mean — sell (fade the rally)
            Some(TradeSignal {
                symbol: symbol.to_string(),
                direction: Direction::Sell,
                quantity: 1.0,
                confidence: (z_score / self.z_score_threshold).min(1.0) * 0.8,
                strategy_id: "mean_reversion".into(),
            })
        } else if z_score < -self.z_score_threshold {
            // Price too low — buy (fade the dip)
            Some(TradeSignal {
                symbol: symbol.to_string(),
                direction: Direction::Buy,
                quantity: 1.0,
                confidence: (-z_score / self.z_score_threshold).min(1.0) * 0.8,
                strategy_id: "mean_reversion".into(),
            })
        } else {
            None
        }
    }

    fn on_ai_signal(&mut self, event: &BotEvent) {
        if let BotEvent::AISignal {
            confidence, symbol, ..
        } = event
        {
            info!(symbol, confidence, "MeanReversion: ingesting AI signal");
            self.last_ai_confidence = *confidence;
        }
    }

    fn reset(&mut self) {
        self.window.clear();
        self.running_sum = 0.0;
        self.running_sum_sq = 0.0;
        self.last_ai_confidence = 1.0;
    }
}

// ─── v1 Compat ───────────────────────────────────────────────────

/// Legacy v1 strategy trait (backward compat with daemon)
pub trait Strategy: Send {
    fn on_event(&mut self, event: &common::SwapEvent) -> common::Action;
    fn on_ai_signal_v1(&mut self, signal: &BotEvent);
}

/// Legacy simple strategy (kept for daemon backward compat)
pub struct SimpleStrategy {
    threshold: u128,
    last_ai_confidence: f64,
}

impl SimpleStrategy {
    pub fn new(threshold: u128) -> Self {
        Self {
            threshold,
            last_ai_confidence: 1.0,
        }
    }
}

impl Strategy for SimpleStrategy {
    fn on_event(&mut self, event: &common::SwapEvent) -> common::Action {
        if self.last_ai_confidence < 0.65 {
            info!(
                "Strategy Veto: AI Confidence {} below 0.65",
                self.last_ai_confidence
            );
            return common::Action::Hold;
        }

        if event.amount_in > self.threshold {
            common::Action::Buy {
                token: event.token_out.clone(),
                size: 0.1,
                confidence: 0.9,
            }
        } else {
            common::Action::Hold
        }
    }

    fn on_ai_signal_v1(&mut self, event: &BotEvent) {
        if let BotEvent::AISignal {
            confidence, symbol, ..
        } = event
        {
            info!(
                "Strategy engine ingesting AI Signal for {}. Confidence: {}",
                symbol, confidence
            );
            self.last_ai_confidence = *confidence;
        }
    }
}
