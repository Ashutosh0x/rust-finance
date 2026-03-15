// crates/backtest/src/strategy.rs
//
// Strategy trait — implement this to plug any algo into the backtest engine.
// Includes a SimpleMovingAverageCrossover as a reference implementation.

use std::collections::{HashMap, VecDeque};
use crate::engine::Bar;

/// A signal emitted by a strategy.
#[derive(Debug, Clone)]
pub struct StrategySignal {
    pub symbol: String,
    /// Positive = buy, negative = sell. Size in units.
    pub qty: f64,
    pub reason: &'static str,
}

/// The strategy trait — implement `on_bar` to define trading logic.
pub trait Strategy {
    fn on_bar(
        &mut self,
        bar: &Bar,
        positions: &HashMap<String, f64>,
        cash: f64,
    ) -> Vec<StrategySignal>;
}

// ── SMA Crossover Strategy ────────────────────────────────────────────────────

/// Classic dual-SMA crossover: buy when fast crosses above slow, sell when below.
pub struct SimpleMovingAverageCrossover {
    fast_period: usize,
    slow_period: usize,
    prices: HashMap<String, VecDeque<f64>>,
    position_size: f64, // fixed position size in units
}

impl SimpleMovingAverageCrossover {
    pub fn new(fast_period: usize, slow_period: usize) -> Self {
        Self {
            fast_period,
            slow_period,
            prices: HashMap::new(),
            position_size: 10.0,
        }
    }

    fn sma(prices: &VecDeque<f64>, period: usize) -> Option<f64> {
        if prices.len() < period {
            return None;
        }
        let sum: f64 = prices.iter().rev().take(period).sum();
        Some(sum / period as f64)
    }
}

impl Strategy for SimpleMovingAverageCrossover {
    fn on_bar(
        &mut self,
        bar: &Bar,
        positions: &HashMap<String, f64>,
        _cash: f64,
    ) -> Vec<StrategySignal> {
        let buf = self
            .prices
            .entry(bar.symbol.clone())
            .or_insert_with(VecDeque::new);

        buf.push_back(bar.close);
        if buf.len() > self.slow_period + 1 {
            buf.pop_front();
        }

        let fast_now = Self::sma(buf, self.fast_period);
        // Fast SMA one bar ago — need a second buffer snapshot
        let fast_prev = if buf.len() > 1 {
            let prev_buf: VecDeque<f64> = buf.iter().rev().skip(1).take(self.fast_period).cloned().collect();
            if prev_buf.len() == self.fast_period {
                Some(prev_buf.iter().sum::<f64>() / self.fast_period as f64)
            } else {
                None
            }
        } else {
            None
        };
        let slow_now = Self::sma(buf, self.slow_period);
        let current_pos = positions.get(&bar.symbol).copied().unwrap_or(0.0);

        let mut signals = Vec::new();

        match (fast_now, fast_prev, slow_now) {
            (Some(fast), Some(prev_fast), Some(slow)) => {
                // Golden cross: fast crossed above slow
                if fast > slow && prev_fast <= slow && current_pos <= 0.0 {
                    // Close short if any, then go long
                    if current_pos < 0.0 {
                        signals.push(StrategySignal {
                            symbol: bar.symbol.clone(),
                            qty: -current_pos,
                            reason: "close_short",
                        });
                    }
                    signals.push(StrategySignal {
                        symbol: bar.symbol.clone(),
                        qty: self.position_size,
                        reason: "golden_cross_long",
                    });
                }
                // Death cross: fast crossed below slow
                else if fast < slow && prev_fast >= slow && current_pos >= 0.0 {
                    if current_pos > 0.0 {
                        signals.push(StrategySignal {
                            symbol: bar.symbol.clone(),
                            qty: -current_pos,
                            reason: "close_long",
                        });
                    }
                    signals.push(StrategySignal {
                        symbol: bar.symbol.clone(),
                        qty: -self.position_size,
                        reason: "death_cross_short",
                    });
                }
            }
            _ => {}
        }

        signals
    }
}

// ── Mean Reversion Strategy ───────────────────────────────────────────────────

/// Simple z-score mean reversion: buy oversold, sell overbought.
pub struct ZScoreMeanReversion {
    window: usize,
    entry_z: f64,
    exit_z: f64,
    prices: HashMap<String, VecDeque<f64>>,
    position_size: f64,
}

impl ZScoreMeanReversion {
    pub fn new(window: usize, entry_z: f64, exit_z: f64) -> Self {
        Self {
            window,
            entry_z,
            exit_z,
            prices: HashMap::new(),
            position_size: 10.0,
        }
    }
}

impl Strategy for ZScoreMeanReversion {
    fn on_bar(
        &mut self,
        bar: &Bar,
        positions: &HashMap<String, f64>,
        _cash: f64,
    ) -> Vec<StrategySignal> {
        let buf = self
            .prices
            .entry(bar.symbol.clone())
            .or_insert_with(VecDeque::new);

        buf.push_back(bar.close);
        if buf.len() > self.window {
            buf.pop_front();
        }

        if buf.len() < self.window {
            return vec![];
        }

        let mean: f64 = buf.iter().sum::<f64>() / buf.len() as f64;
        let variance: f64 = buf.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / buf.len() as f64;
        let std_dev = variance.sqrt();
        if std_dev < 1e-9 {
            return vec![];
        }

        let z = (bar.close - mean) / std_dev;
        let current_pos = positions.get(&bar.symbol).copied().unwrap_or(0.0);
        let mut signals = Vec::new();

        if z < -self.entry_z && current_pos == 0.0 {
            signals.push(StrategySignal {
                symbol: bar.symbol.clone(),
                qty: self.position_size,
                reason: "oversold_entry",
            });
        } else if z > self.entry_z && current_pos == 0.0 {
            signals.push(StrategySignal {
                symbol: bar.symbol.clone(),
                qty: -self.position_size,
                reason: "overbought_entry",
            });
        } else if z.abs() < self.exit_z && current_pos != 0.0 {
            signals.push(StrategySignal {
                symbol: bar.symbol.clone(),
                qty: -current_pos,
                reason: "mean_reversion_exit",
            });
        }

        signals
    }
}
