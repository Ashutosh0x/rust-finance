use common::events::OrderSide;
use compact_str::CompactString;

/// Lightweight snapshot of a resting order for self-match prevention checks.
#[derive(Debug, Clone)]
pub struct OpenOrderSnapshot {
    pub symbol: CompactString,
    pub side: OrderSide,
    pub price: f64,
}

pub struct EngineState {
    pub total_equity: f64,
    pub daily_pnl: f64,
    pub current_drawdown_pct: f64,
    pub open_order_count: usize,
    pub daily_trade_count: usize,
    /// Current resting orders — used by SelfMatchPrevention interceptor.
    pub open_orders: Vec<OpenOrderSnapshot>,
}

impl EngineState {
    pub fn new(equity: f64) -> Self {
        Self {
            total_equity: equity,
            daily_pnl: 0.0,
            current_drawdown_pct: 0.0,
            open_order_count: 0,
            daily_trade_count: 0,
            open_orders: Vec::new(),
        }
    }

    pub fn update_drawdown(&mut self, peak_equity: f64) {
        if peak_equity > 0.0 {
            self.current_drawdown_pct = (peak_equity - self.total_equity) / peak_equity;
        }
    }
}
