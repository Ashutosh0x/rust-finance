// crates/backtest/src/engine.rs
//
// Backtesting engine — replays historical OHLCV bars through a strategy,
// simulates fills against bid/ask, and computes performance metrics.
//
// v0.3: Pluggable fill model (SquareRootImpact replaces fixed slippage).

use crate::fill_model::{FillModel, FixedSlippage};
use crate::strategy::{Strategy, StrategySignal};
use serde::{Deserialize, Serialize};

/// A single historical OHLCV + spread bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bar {
    pub timestamp: i64, // Unix ms
    pub symbol: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub bid: f64,
    pub ask: f64,
}

/// Simulated fill result.
#[derive(Debug, Clone)]
pub struct SimulatedFill {
    pub timestamp: i64,
    pub symbol: String,
    pub qty: f64, // positive = buy, negative = sell
    pub price: f64,
    pub commission: f64,
    pub slippage: f64,
}

/// Backtest configuration.
#[derive(Clone)]
pub struct BacktestConfig {
    pub initial_cash: f64,
    /// Commission as fraction of notional.
    pub commission_rate: f64,
    /// Legacy slippage rate — used by FixedSlippage if no fill_model is set.
    pub slippage_rate: f64,
    /// Fill on next bar's open (True) vs current bar's close (False).
    pub fill_on_next_open: bool,
    /// Allow short selling.
    pub allow_short: bool,
}

impl std::fmt::Debug for BacktestConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BacktestConfig")
            .field("initial_cash", &self.initial_cash)
            .field("commission_rate", &self.commission_rate)
            .field("slippage_rate", &self.slippage_rate)
            .field("fill_on_next_open", &self.fill_on_next_open)
            .field("allow_short", &self.allow_short)
            .finish()
    }
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            initial_cash: 100_000.0,
            commission_rate: 0.0005, // 5 bps
            slippage_rate: 0.0001,   // 1 bp (legacy)
            fill_on_next_open: true,
            allow_short: true,
        }
    }
}

/// Performance metrics computed after a backtest run.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestMetrics {
    pub total_return: f64,
    pub cagr: f64,
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub max_drawdown: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub total_trades: usize,
    pub avg_trade_pnl: f64,
    pub final_equity: f64,
    pub equity_curve: Vec<(i64, f64)>,
}

/// Main backtesting engine.
pub struct BacktestEngine {
    cfg: BacktestConfig,
    fill_model: Box<dyn FillModel>,
    cash: f64,
    positions: std::collections::HashMap<String, f64>, // symbol → qty
    cost_basis: std::collections::HashMap<String, f64>,
    equity_curve: Vec<(i64, f64)>,
    fills: Vec<SimulatedFill>,
    pending_signals: Vec<StrategySignal>,
    last_prices: std::collections::HashMap<String, f64>,
    current_bar: Option<Bar>,
}

impl BacktestEngine {
    pub fn new(cfg: BacktestConfig) -> Self {
        let cash = cfg.initial_cash;
        let fill_model = Box::new(FixedSlippage::new(cfg.slippage_rate));
        Self {
            cfg,
            fill_model,
            cash,
            positions: Default::default(),
            cost_basis: Default::default(),
            equity_curve: Vec::new(),
            fills: Vec::new(),
            pending_signals: Vec::new(),
            last_prices: Default::default(),
            current_bar: None,
        }
    }

    /// Create with a custom fill model (e.g. SquareRootImpact).
    pub fn with_fill_model(cfg: BacktestConfig, fill_model: Box<dyn FillModel>) -> Self {
        let cash = cfg.initial_cash;
        Self {
            cfg,
            fill_model,
            cash,
            positions: Default::default(),
            cost_basis: Default::default(),
            equity_curve: Vec::new(),
            fills: Vec::new(),
            pending_signals: Vec::new(),
            last_prices: Default::default(),
            current_bar: None,
        }
    }

    /// Run a strategy over a sequence of bars.
    pub fn run<S: Strategy>(&mut self, bars: &[Bar], strategy: &mut S) -> BacktestMetrics {
        for (_i, bar) in bars.iter().enumerate() {
            self.current_bar = Some(bar.clone());

            // Execute any pending signals from the previous bar
            if self.cfg.fill_on_next_open {
                let pending = std::mem::take(&mut self.pending_signals);
                for signal in pending {
                    self.execute_signal(&signal, bar.open, bar.timestamp);
                }
            }

            self.last_prices.insert(bar.symbol.clone(), bar.close);

            // Feed bar to strategy
            let signals = strategy.on_bar(bar, &self.positions, self.cash);
            for signal in signals {
                if self.cfg.fill_on_next_open {
                    self.pending_signals.push(signal);
                } else {
                    self.execute_signal(&signal, bar.close, bar.timestamp);
                }
            }

            // Mark-to-market equity
            let equity = self.mark_to_market(bar);
            self.equity_curve.push((bar.timestamp, equity));
        }

        self.compute_metrics()
    }

    fn execute_signal(&mut self, signal: &StrategySignal, price: f64, timestamp: i64) {
        // Use the fill model for price impact simulation
        let bar = self.current_bar.clone().unwrap_or_else(|| Bar {
            timestamp,
            symbol: signal.symbol.clone(),
            open: price,
            high: price * 1.005,
            low: price * 0.995,
            close: price,
            volume: 1_000_000.0,
            bid: price - 0.01,
            ask: price + 0.01,
        });
        let fill_result = self.fill_model.simulate_fill(signal.qty, price, &bar);
        let fill_price = fill_result.fill_price;
        let _slippage = fill_price - price;
        let notional = fill_price * signal.qty.abs();
        let commission = notional * self.cfg.commission_rate;

        // Check if we have enough cash for buys
        if signal.qty > 0.0 && self.cash < notional + commission {
            return; // Skip — insufficient funds
        }

        // Check if short selling is allowed
        let current_qty = self.positions.get(&signal.symbol).copied().unwrap_or(0.0);
        if signal.qty < 0.0 && current_qty + signal.qty < 0.0 && !self.cfg.allow_short {
            return;
        }

        // Update cash
        self.cash -= signal.qty * fill_price + commission;

        // Update position
        let new_qty = current_qty + signal.qty;
        if new_qty.abs() < 1e-9 {
            self.positions.remove(&signal.symbol);
            self.cost_basis.remove(&signal.symbol);
        } else {
            self.positions.insert(signal.symbol.clone(), new_qty);
            // Update VWAP cost basis
            let prev_cost = self
                .cost_basis
                .get(&signal.symbol)
                .copied()
                .unwrap_or(fill_price);
            if (current_qty > 0.0) == (signal.qty > 0.0) {
                let vwap =
                    (prev_cost * current_qty.abs() + fill_price * signal.qty.abs()) / new_qty.abs();
                self.cost_basis.insert(signal.symbol.clone(), vwap);
            } else {
                self.cost_basis.insert(signal.symbol.clone(), fill_price);
            }
        }

        self.fills.push(SimulatedFill {
            timestamp,
            symbol: signal.symbol.clone(),
            qty: signal.qty,
            price: fill_price,
            commission,
            slippage: (fill_price - price).abs() * signal.qty.abs(),
        });
    }

    fn mark_to_market(&self, bar: &Bar) -> f64 {
        let pos_value: f64 =
            self.positions
                .iter()
                .map(|(sym, qty)| {
                    if sym == &bar.symbol {
                        qty * bar.close
                    } else {
                        let last_price =
                            self.last_prices.get(sym).copied().unwrap_or_else(|| {
                                self.cost_basis.get(sym).copied().unwrap_or(0.0)
                            });
                        qty * last_price
                    }
                })
                .sum();
        self.cash + pos_value
    }

    fn compute_metrics(&self) -> BacktestMetrics {
        let initial = self.cfg.initial_cash;
        let final_equity = self.equity_curve.last().map(|(_, v)| *v).unwrap_or(initial);
        let total_return = (final_equity - initial) / initial;

        // CAGR
        let n_bars = self.equity_curve.len();
        let years = n_bars as f64 / 252.0; // assume daily bars
        let cagr = if years > 0.0 {
            (final_equity / initial).powf(1.0 / years) - 1.0
        } else {
            0.0
        };

        // Daily returns
        let returns: Vec<f64> = self
            .equity_curve
            .windows(2)
            .map(|w| (w[1].1 - w[0].1) / w[0].1)
            .collect();

        // Sharpe ratio (annualised, risk-free = 0)
        let mean_ret = returns.iter().sum::<f64>() / returns.len().max(1) as f64;
        let variance = returns.iter().map(|r| (r - mean_ret).powi(2)).sum::<f64>()
            / returns.len().max(1) as f64;
        let std_dev = variance.sqrt();
        let sharpe = if std_dev > 0.0 {
            mean_ret / std_dev * (252.0_f64).sqrt()
        } else {
            0.0
        };

        // Sortino ratio (downside deviation only)
        let downside: Vec<f64> = returns.iter().filter(|&&r| r < 0.0).copied().collect();
        let downside_variance =
            downside.iter().map(|r| r.powi(2)).sum::<f64>() / downside.len().max(1) as f64;
        let downside_std = downside_variance.sqrt();
        let sortino = if downside_std > 0.0 {
            mean_ret / downside_std * (252.0_f64).sqrt()
        } else {
            0.0
        };

        // Max drawdown
        let mut peak = initial;
        let mut max_dd = 0.0_f64;
        for (_, equity) in &self.equity_curve {
            if *equity > peak {
                peak = *equity;
            }
            let dd = (peak - equity) / peak;
            if dd > max_dd {
                max_dd = dd;
            }
        }

        // Win rate & profit factor
        let (wins, losses): (Vec<f64>, Vec<f64>) = {
            let mut w = Vec::new();
            let mut l = Vec::new();
            for fill in &self.fills {
                let cost = self
                    .cost_basis
                    .get(&fill.symbol)
                    .copied()
                    .unwrap_or(fill.price);
                let pnl = (fill.price - cost) * fill.qty - fill.commission;
                if pnl > 0.0 {
                    w.push(pnl);
                } else {
                    l.push(pnl.abs());
                }
            }
            (w, l)
        };

        let total_trades = self.fills.len();
        let win_rate = if total_trades > 0 {
            wins.len() as f64 / total_trades as f64
        } else {
            0.0
        };
        let gross_profit: f64 = wins.iter().sum();
        let gross_loss: f64 = losses.iter().sum();
        let profit_factor = if gross_loss > 0.0 {
            gross_profit / gross_loss
        } else {
            f64::INFINITY
        };

        let avg_trade_pnl = if total_trades > 0 {
            (gross_profit - gross_loss) / total_trades as f64
        } else {
            0.0
        };

        BacktestMetrics {
            total_return,
            cagr,
            sharpe_ratio: sharpe,
            sortino_ratio: sortino,
            max_drawdown: max_dd,
            win_rate,
            profit_factor,
            total_trades,
            avg_trade_pnl,
            final_equity,
            equity_curve: self.equity_curve.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::SimpleMovingAverageCrossover;

    fn make_trending_bars(n: usize, trend: f64) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let price = 100.0 + i as f64 * trend;
                Bar {
                    timestamp: i as i64 * 86_400_000,
                    symbol: "TEST".into(),
                    open: price,
                    high: price * 1.01,
                    low: price * 0.99,
                    close: price,
                    volume: 1_000_000.0,
                    bid: price - 0.05,
                    ask: price + 0.05,
                }
            })
            .collect()
    }

    #[test]
    fn test_backtest_completes() {
        let bars = make_trending_bars(100, 0.5);
        let mut engine = BacktestEngine::new(BacktestConfig::default());
        let mut strategy = SimpleMovingAverageCrossover::new(5, 20);
        let metrics = engine.run(&bars, &mut strategy);
        assert!(metrics.equity_curve.len() == 100);
    }

    #[test]
    fn test_uptrend_produces_positive_return() {
        let bars = make_trending_bars(200, 1.0);
        let mut engine = BacktestEngine::new(BacktestConfig::default());
        let mut strategy = SimpleMovingAverageCrossover::new(5, 20);
        let metrics = engine.run(&bars, &mut strategy);
        // Strong uptrend → positive total return
        assert!(metrics.total_return > 0.0 || metrics.total_trades == 0);
    }
}
