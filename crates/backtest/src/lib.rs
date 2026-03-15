// crates/backtest/src/lib.rs
//
// Root module for backtesting logic.

pub mod engine;
pub mod strategy;

pub use engine::{BacktestEngine, BacktestConfig, BacktestMetrics, Bar};
pub use strategy::{Strategy, StrategySignal, SimpleMovingAverageCrossover, ZScoreMeanReversion};
