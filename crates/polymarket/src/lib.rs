#![forbid(unsafe_code)]
// crates/polymarket/src/lib.rs

pub mod arbitrage;
pub mod auth;
pub mod clob;
pub mod config;
pub mod copy_trading;
pub mod data;
pub mod gamma;
pub mod signing;
pub mod websocket;

// Re-export key types
pub use auth::ApiCredentials;
pub use clob::{BookLevel, ClobClient, OrderBookResponse, OrderType, Side};
pub use config::PolymarketConfig;
pub use data::{DataClient, LeaderboardEntry, UserPosition, UserProfile};
pub use gamma::{
    EventQuery, GammaCategory, GammaClient, GammaCollection, GammaComment, GammaEvent, GammaMarket,
    GammaSeries, GammaTag, MarketQuery, PublicProfile, Token,
};
pub use signing::Order;
