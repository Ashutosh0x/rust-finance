use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardState {
    pub sol_balance: f64,
    pub sol_price_usd: f64,
    pub uptime_secs: u64,
    pub total_txs: u64,
    pub pnl_percent: f64,
    pub pnl_sol: f64,
    pub total_trades: u64,

    pub logs: VecDeque<String>,
    pub recent_trades: VecDeque<TradeEntry>,
    pub parsed_txs: VecDeque<TxEntry>,
    pub tracked_tokens: Vec<TokenEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeEntry {
    pub token: String,
    pub entry: f64,
    pub exit: f64,
    pub pnl_percent: f64,
    pub pnl_sol: f64,
    pub pnl_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxEntry {
    pub sig: String,
    pub token: String,
    pub tx_type: String,
    pub sol_amount: f64,
    pub reserves: String,
    pub age_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenEntry {
    pub token: String,
    pub mcap: String,
    pub frames: String,
    pub balance: f64,
    pub pnl: f64,
    pub pos: String,
    pub model_score: String,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            sol_balance: 99.90,
            sol_price_usd: 100.0,
            uptime_secs: 0,
            total_txs: 0,
            pnl_percent: 0.0,
            pnl_sol: 0.0,
            total_trades: 0,
            logs: VecDeque::with_capacity(50),
            recent_trades: VecDeque::with_capacity(10),
            parsed_txs: VecDeque::with_capacity(15),
            tracked_tokens: Vec::new(),
        }
    }
}

impl DashboardState {
    pub fn push_log(&mut self, msg: String) {
        if self.logs.len() >= 50 {
            self.logs.pop_front();
        }
        self.logs.push_back(msg);
    }
}
