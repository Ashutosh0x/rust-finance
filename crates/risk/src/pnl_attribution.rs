// crates/risk/src/pnl_attribution.rs
// Break down today's P&L by strategy, symbol, signal source, and Greek exposure
// Essential for understanding what's actually making (or losing) money

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnlAttribution {
    pub total_realized_pnl: f64,
    pub total_unrealized_pnl: f64,
    pub total_pnl: f64,
    pub total_commission: f64,
    pub total_slippage: f64,
    pub net_pnl: f64,

    /// P&L broken down by strategy name
    pub by_strategy: HashMap<String, StrategyPnl>,
    /// P&L broken down by symbol
    pub by_symbol: HashMap<String, SymbolPnl>,
    /// P&L broken down by signal source (Dexter, MiroFish, manual, etc.)
    pub by_signal_source: HashMap<String, f64>,
    /// P&L broken down by hour of day (0-23)
    pub by_hour: HashMap<u8, f64>,
    /// Rolling 5-minute P&L (for momentum detection)
    pub rolling_5m: f64,
    /// Greeks exposure (if options are traded)
    pub greeks: PortfolioGreeks,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrategyPnl {
    pub strategy: String,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    pub commission: f64,
    pub trade_count: usize,
    pub win_count: usize,
    pub loss_count: usize,
    pub largest_win: f64,
    pub largest_loss: f64,
    pub current_drawdown: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolPnl {
    pub symbol: String,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    pub total_volume_usd: f64,
    pub net_position: f64,
    pub avg_entry_price: f64,
    pub current_price: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortfolioGreeks {
    /// Net delta in USD (1.0 = $1 P&L per $1 move in underlying)
    pub net_delta: f64,
    /// Net gamma in USD (rate of delta change)
    pub net_gamma: f64,
    /// Net theta in USD per day (time decay)
    pub net_theta: f64,
    /// Net vega in USD per 1% IV change
    pub net_vega: f64,
    /// Rho: sensitivity to interest rates
    pub net_rho: f64,
}

/// A single P&L event — feed these in to build attribution
#[derive(Debug, Clone)]
pub struct PnlEvent {
    pub ts: u64,
    pub symbol: String,
    pub strategy: String,
    pub signal_source: String, // "dexter", "mirofish", "manual", "bracket_tp", etc.
    pub realized_pnl: f64,
    pub commission: f64,
    pub slippage: f64,
    pub is_win: bool,
}

pub struct PnlAttributor {
    events: Vec<PnlEvent>,
    unrealized: HashMap<String, f64>, // symbol → current unrealized P&L
    greeks: PortfolioGreeks,
    session_start_ts: u64,
}

impl PnlAttributor {
    pub fn new(session_start_ts: u64) -> Self {
        Self {
            events: Vec::new(),
            unrealized: HashMap::new(),
            greeks: PortfolioGreeks::default(),
            session_start_ts,
        }
    }

    pub fn record_trade(&mut self, event: PnlEvent) {
        tracing::debug!(
            symbol = %event.symbol,
            strategy = %event.strategy,
            pnl = event.realized_pnl,
            "P&L event recorded"
        );
        self.events.push(event);
    }

    pub fn update_unrealized(&mut self, symbol: &str, unrealized_pnl: f64) {
        self.unrealized.insert(symbol.to_string(), unrealized_pnl);
    }

    pub fn update_greeks(&mut self, greeks: PortfolioGreeks) {
        self.greeks = greeks;
    }

    pub fn compute(&self, now_ts: u64) -> PnlAttribution {
        let total_realized: f64 = self.events.iter().map(|e| e.realized_pnl).sum();
        let total_unrealized: f64 = self.unrealized.values().sum();
        let total_commission: f64 = self.events.iter().map(|e| e.commission).sum();
        let total_slippage: f64 = self.events.iter().map(|e| e.slippage).sum();

        // By strategy
        let mut by_strategy: HashMap<String, StrategyPnl> = HashMap::new();
        for event in &self.events {
            let entry = by_strategy
                .entry(event.strategy.clone())
                .or_insert_with(|| StrategyPnl {
                    strategy: event.strategy.clone(),
                    ..Default::default()
                });
            entry.realized_pnl += event.realized_pnl;
            entry.commission += event.commission;
            entry.trade_count += 1;
            if event.is_win {
                entry.win_count += 1;
                if event.realized_pnl > entry.largest_win {
                    entry.largest_win = event.realized_pnl;
                }
            } else {
                entry.loss_count += 1;
                if event.realized_pnl < entry.largest_loss {
                    entry.largest_loss = event.realized_pnl;
                }
            }
        }
        // Add unrealized to strategy P&L
        for (sym, &unreal) in &self.unrealized {
            // Best effort: attribute unrealized to last strategy that traded this symbol
            if let Some(last_event) = self.events.iter().rev().find(|e| &e.symbol == sym) {
                if let Some(entry) = by_strategy.get_mut(&last_event.strategy) {
                    entry.unrealized_pnl += unreal;
                }
            }
        }

        // By symbol
        let mut by_symbol: HashMap<String, SymbolPnl> = HashMap::new();
        for event in &self.events {
            let entry = by_symbol
                .entry(event.symbol.clone())
                .or_insert_with(|| SymbolPnl {
                    symbol: event.symbol.clone(),
                    ..Default::default()
                });
            entry.realized_pnl += event.realized_pnl;
        }
        for (sym, &unreal) in &self.unrealized {
            by_symbol
                .entry(sym.clone())
                .or_insert_with(|| SymbolPnl {
                    symbol: sym.clone(),
                    ..Default::default()
                })
                .unrealized_pnl = unreal;
        }

        // By signal source
        let mut by_signal_source: HashMap<String, f64> = HashMap::new();
        for event in &self.events {
            *by_signal_source
                .entry(event.signal_source.clone())
                .or_insert(0.0) += event.realized_pnl;
        }

        // By hour
        let mut by_hour: HashMap<u8, f64> = HashMap::new();
        for event in &self.events {
            let hour = ((event.ts / 1_000_000) % 86400 / 3600) as u8;
            *by_hour.entry(hour).or_insert(0.0) += event.realized_pnl;
        }

        // Rolling 5-minute P&L
        let cutoff_5m = now_ts.saturating_sub(5 * 60 * 1_000_000);
        let rolling_5m: f64 = self
            .events
            .iter()
            .filter(|e| e.ts >= cutoff_5m)
            .map(|e| e.realized_pnl)
            .sum();

        PnlAttribution {
            total_realized_pnl: total_realized,
            total_unrealized_pnl: total_unrealized,
            total_pnl: total_realized + total_unrealized,
            total_commission,
            total_slippage,
            net_pnl: total_realized + total_unrealized - total_commission - total_slippage,
            by_strategy,
            by_symbol,
            by_signal_source,
            by_hour,
            rolling_5m,
            greeks: self.greeks.clone(),
        }
    }

    pub fn reset_session(&mut self, new_session_ts: u64) {
        self.events.clear();
        self.unrealized.clear();
        self.session_start_ts = new_session_ts;
    }
}
