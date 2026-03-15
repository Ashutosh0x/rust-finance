// crates/oms/src/position.rs
//
// Position manager computing net qty, real/unrealised P&L,
// VWAP cost basis, and position drawdown tracking.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub qty: f64,
    pub vwap_cost: f64,
    pub realised_pnl: f64,
    pub unrealised_pnl: f64,
    pub market_value: f64,
    /// Peak equity for this specific position to track drawdown.
    pub peak_value: f64,
    pub max_drawdown: f64,
}

impl Position {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            qty: 0.0,
            vwap_cost: 0.0,
            realised_pnl: 0.0,
            unrealised_pnl: 0.0,
            market_value: 0.0,
            peak_value: 0.0,
            max_drawdown: 0.0,
        }
    }

    /// Update position tracking after a fill.
    /// Qty is positive for buys, negative for sells.
    pub fn apply_fill(&mut self, fill_qty: f64, fill_price: f64, commission: f64) {
        let is_buy = fill_qty > 0.0;
        let is_long = self.qty > 0.0;
        let is_short = self.qty < 0.0;

        self.realised_pnl -= commission;

        if self.qty == 0.0 {
            // New position (long or short)
            self.qty = fill_qty;
            self.vwap_cost = fill_price;
        } else if (is_long && is_buy) || (is_short && !is_buy) {
            // Adding to existing position (same direction)
            let new_qty = self.qty + fill_qty;
            // VWAP = (Old_Notional + New_Notional) / Total_Qty
            self.vwap_cost =
                (self.vwap_cost * self.qty.abs() + fill_price * fill_qty.abs()) / new_qty.abs();
            self.qty = new_qty;
        } else {
            // Reducing position (closing or flipping direction)
            if fill_qty.abs() <= self.qty.abs() {
                // Partial or full close
                let closed_qty = fill_qty.abs();
                let direction_multiplier = if is_long { 1.0 } else { -1.0 };
                let pnl = (fill_price - self.vwap_cost) * closed_qty * direction_multiplier;
                self.realised_pnl += pnl;
                self.qty += fill_qty;
                if self.qty.abs() < 1e-9 {
                    self.qty = 0.0;
                    self.vwap_cost = 0.0;
                }
            } else {
                // Position flip (e.g., Long 10, Sell 15 -> Short 5)
                let closed_qty = self.qty.abs();
                let remaining_qty = fill_qty.abs() - closed_qty;
                let direction_multiplier = if is_long { 1.0 } else { -1.0 };

                // Realise P&L on the closed portion
                let pnl = (fill_price - self.vwap_cost) * closed_qty * direction_multiplier;
                self.realised_pnl += pnl;

                // Establish new position on the remaining
                self.qty = if is_long { -remaining_qty } else { remaining_qty };
                self.vwap_cost = fill_price;
            }
        }
    }

    /// Mark the position to market against the latest price.
    pub fn mark_to_market(&mut self, current_price: f64) {
        if self.qty == 0.0 {
            self.unrealised_pnl = 0.0;
            self.market_value = 0.0;
            return;
        }

        self.market_value = current_price * self.qty.abs();

        let direction_multiplier = if self.qty > 0.0 { 1.0 } else { -1.0 };
        self.unrealised_pnl =
            (current_price - self.vwap_cost) * self.qty.abs() * direction_multiplier;

        let current_equity = self.market_value + self.unrealised_pnl;

        if current_equity > self.peak_value {
            self.peak_value = current_equity;
        } else if self.peak_value > 0.0 {
            let drawdown = (self.peak_value - current_equity) / self.peak_value;
            if drawdown > self.max_drawdown {
                self.max_drawdown = drawdown;
            }
        }
    }
}

pub struct PositionManager {
    positions: HashMap<String, Position>,
}

impl PositionManager {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
        }
    }

    pub fn apply_fill(&mut self, symbol: &str, fill_qty: f64, fill_price: f64, commission: f64) {
        let pos = self
            .positions
            .entry(symbol.to_string())
            .or_insert_with(|| Position::new(symbol.to_string()));
        pos.apply_fill(fill_qty, fill_price, commission);
    }

    pub fn mark_to_market(&mut self, symbol: &str, current_price: f64) {
        if let Some(pos) = self.positions.get_mut(symbol) {
            pos.mark_to_market(current_price);
        }
    }

    pub fn get_position(&self, symbol: &str) -> Option<&Position> {
        self.positions.get(symbol)
    }

    pub fn all_positions(&self) -> Vec<&Position> {
        self.positions.values().collect()
    }
}
