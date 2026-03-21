// crates/polymarket/src/lib.rs

pub mod config;
pub mod auth;
pub mod signing;
pub mod clob;
pub mod data;
pub mod gamma;
pub mod websocket;
pub mod copy_trading;

// Re-export key types
pub use clob::{ClobClient, Side, OrderType, BookLevel, OrderBookResponse};
pub use config::PolymarketConfig;
pub use signing::Order;
pub use auth::ApiCredentials;
pub use gamma::{
    GammaClient, GammaEvent, GammaMarket, GammaTag, GammaCategory,
    GammaSeries, GammaCollection, GammaComment, PublicProfile,
    EventQuery, MarketQuery, Token,
};
pub use data::{DataClient, UserPosition, UserProfile, LeaderboardEntry};

