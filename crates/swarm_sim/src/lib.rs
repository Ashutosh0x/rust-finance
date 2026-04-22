#![forbid(unsafe_code)]
// ============================================================
// swarm_sim — Financial Market Swarm Simulation Engine
// Part of RustForge Terminal (rust-finance)
//
// Architecture:
//   AgentPool  ──step()──►  ActionLog  ──aggregate()──►  SwarmSignal
//       ▲                                                      │
//   MarketState ◄──────── price_impact() ◄────────────────────┘
// ============================================================

pub mod agent;
pub mod config;
pub mod digital_twin;
pub mod engine;
pub mod interview;
pub mod market;
pub mod persistence;
pub mod scenario;
pub mod signal;

pub use agent::{Agent, AgentId, AgentState, TraderType};
pub use config::SwarmConfig;
pub use engine::{SwarmEngine, SwarmStep};
pub use interview::{InterviewEngine, TradeReason};
pub use market::{MarketState, OrderBook, PriceLevel};
pub use scenario::{MarketScenario, ScenarioEngine};
pub use signal::{Conviction, SignalDirection, SwarmSignal};

pub fn default_engine(config: SwarmConfig, initial_state: MarketState) -> engine::SwarmEngine {
    engine::SwarmEngine::new(config, initial_state)
}
