use serde::{Deserialize, Serialize};
use std::time::SystemTime;
pub mod dashboard;
pub mod events;
pub mod time;
pub mod models;
pub mod config;
pub mod env_writer;
pub mod key_validator;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SwapEvent {
    pub tx_sig: String,
    pub timestamp: SystemTime,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: u128,
    pub amount_out: u128,
    pub pool: String,
    pub slot: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenInfo {
    pub mint: String,
    pub symbol: String,
    pub decimals: u8,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Action {
    Buy { token: String, size: f64, confidence: f32 },
    Sell { token: String, size: f64, confidence: f32 },
    Hold,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TradeResult {
    pub tx_sig: String,
    pub success: bool,
    pub error: Option<String>,
}

pub type Result<T> = std::result::Result<T, anyhow::Error>;
