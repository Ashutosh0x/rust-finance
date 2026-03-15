// crates/daemon/src/strategy_registry.rs
//
// Strategy plugin system.
// Strategies are registered by name and hot-swappable at runtime.
// Each strategy receives market events and emits trade signals.
// The daemon routes signals through SEBI + risk checks before execution.

use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{info, warn};

// ── Signal ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TradeSignal {
    pub strategy_id: String,
    pub symbol: String,
    /// Positive = buy, negative = sell, zero = exit / hold.
    pub qty: f64,
    pub limit_price: Option<f64>,
    pub reason: String,
    pub confidence: f64,
}

// ── Market Context ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MarketContext {
    pub symbol: String,
    pub price: f64,
    pub bid: f64,
    pub ask: f64,
    pub volume: f64,
    pub timestamp_ms: i64,
    /// Rolling close prices, newest last.
    pub price_history: Vec<f64>,
    /// Current open position qty for this symbol.
    pub current_position: f64,
    /// Available cash.
    pub cash: f64,
}

// ── Strategy Trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait PluggableStrategy: Send + Sync {
    /// Unique identifier for this strategy instance.
    fn id(&self) -> &str;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Symbols this strategy subscribes to.
    fn subscribed_symbols(&self) -> Vec<String>;

    /// Called on each market event. Return `None` to pass.
    async fn on_market_event(&mut self, ctx: &MarketContext) -> Option<TradeSignal>;

    /// Called when a fill is received for a signal emitted by this strategy.
    async fn on_fill(&mut self, symbol: &str, qty: f64, price: f64) {}

    /// Called on periodic heartbeat (every N seconds) — useful for time-based exits.
    async fn on_heartbeat(&mut self) -> Vec<TradeSignal> {
        vec![]
    }

    /// Whether this strategy is currently enabled.
    fn is_enabled(&self) -> bool;

    /// Enable/disable without removing from registry.
    fn set_enabled(&mut self, enabled: bool);
}

// ── Strategy Registry ─────────────────────────────────────────────────────────

type BoxedStrategy = Box<dyn PluggableStrategy>;

pub struct StrategyRegistry {
    strategies: Arc<RwLock<HashMap<String, BoxedStrategy>>>,
}

impl StrategyRegistry {
    pub fn new() -> Self {
        Self {
            strategies: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new strategy. Replaces any existing strategy with the same ID.
    pub async fn register(&self, strategy: BoxedStrategy) {
        let id = strategy.id().to_string();
        let name = strategy.name().to_string();
        self.strategies.write().await.insert(id.clone(), strategy);
        info!(id, name, "Strategy registered");
    }

    /// Remove a strategy by ID.
    pub async fn deregister(&self, id: &str) {
        if self.strategies.write().await.remove(id).is_some() {
            info!(id, "Strategy deregistered");
        }
    }

    /// Enable or disable a strategy by ID.
    pub async fn set_enabled(&self, id: &str, enabled: bool) {
        let mut strats = self.strategies.write().await;
        if let Some(s) = strats.get_mut(id) {
            s.set_enabled(enabled);
            info!(id, enabled, "Strategy state changed");
        } else {
            warn!(id, "Strategy not found for enable/disable");
        }
    }

    /// Dispatch a market event to all enabled strategies subscribed to the symbol.
    /// Returns all generated signals.
    pub async fn dispatch_market_event(&self, ctx: &MarketContext) -> Vec<TradeSignal> {
        let mut strats = self.strategies.write().await;
        let mut signals = Vec::new();

        for strategy in strats.values_mut() {
            if !strategy.is_enabled() {
                continue;
            }
            let subscribed = strategy.subscribed_symbols();
            if subscribed.is_empty() || subscribed.iter().any(|s| s == &ctx.symbol || s == "*") {
                if let Some(signal) = strategy.on_market_event(ctx).await {
                    signals.push(signal);
                }
            }
        }

        signals
    }

    /// Dispatch a heartbeat to all strategies.
    pub async fn dispatch_heartbeat(&self) -> Vec<TradeSignal> {
        let mut strats = self.strategies.write().await;
        let mut signals = Vec::new();
        for strategy in strats.values_mut() {
            if strategy.is_enabled() {
                signals.extend(strategy.on_heartbeat().await);
            }
        }
        signals
    }

    /// Notify strategies of a fill.
    pub async fn notify_fill(&self, strategy_id: &str, symbol: &str, qty: f64, price: f64) {
        let mut strats = self.strategies.write().await;
        if let Some(s) = strats.get_mut(strategy_id) {
            s.on_fill(symbol, qty, price).await;
        }
    }

    pub async fn list(&self) -> Vec<(String, String, bool)> {
        self.strategies
            .read()
            .await
            .values()
            .map(|s| (s.id().to_string(), s.name().to_string(), s.is_enabled()))
            .collect()
    }
}

// ── Example: AI-Gated Momentum Strategy ──────────────────────────────────────

/// Pluggable strategy that combines momentum signal with AI confidence gate.
pub struct AiGatedMomentum {
    id: String,
    enabled: bool,
    momentum_window: usize,
    ai_confidence_threshold: f64,
    position_size: f64,
    /// Latest AI confidence from the MiroFish swarm.
    last_ai_confidence: f64,
    /// Latest AI action.
    last_ai_action: String,
}

impl AiGatedMomentum {
    pub fn new(
        id: impl Into<String>,
        momentum_window: usize,
        ai_confidence_threshold: f64,
        position_size: f64,
    ) -> Self {
        Self {
            id: id.into(),
            enabled: true,
            momentum_window,
            ai_confidence_threshold,
            position_size,
            last_ai_confidence: 0.0,
            last_ai_action: "HOLD".to_string(),
        }
    }

    /// Call this when a new MiroFish swarm result arrives.
    pub fn update_ai_signal(&mut self, action: impl Into<String>, confidence: f64) {
        self.last_ai_action = action.into();
        self.last_ai_confidence = confidence;
    }
}

#[async_trait]
impl PluggableStrategy for AiGatedMomentum {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        "AI-Gated Momentum"
    }

    fn subscribed_symbols(&self) -> Vec<String> {
        vec!["*".into()] // Subscribe to all
    }

    async fn on_market_event(&mut self, ctx: &MarketContext) -> Option<TradeSignal> {
        if ctx.price_history.len() < self.momentum_window {
            return None;
        }

        // Extremely simple momentum: compare current price to N bars ago
        let old_price = ctx.price_history[ctx.price_history.len() - self.momentum_window];
        let price_diff = ctx.price - old_price;

        let has_momentum_up = price_diff > 0.0;
        let has_momentum_down = price_diff < 0.0;

        // Ensure AI is strongly confident and aligns with momentum
        let is_ai_bullish = self.last_ai_action == "BUY" && self.last_ai_confidence >= self.ai_confidence_threshold;
        let is_ai_bearish = self.last_ai_action == "SELL" && self.last_ai_confidence >= self.ai_confidence_threshold;

        if has_momentum_up && is_ai_bullish && ctx.current_position <= 0.0 {
            // Signal a buy (or cover short and go long)
            let desired_qty = self.position_size - ctx.current_position;
            return Some(TradeSignal {
                strategy_id: self.id.clone(),
                symbol: ctx.symbol.clone(),
                qty: desired_qty,
                limit_price: Some(ctx.ask),
                reason: "momentum_up_ai_confirmed".into(),
                confidence: self.last_ai_confidence,
            });
        }

        if has_momentum_down && is_ai_bearish && ctx.current_position >= 0.0 {
            // Signal a short (or close long and short)
            let desired_qty = -self.position_size - ctx.current_position;
            return Some(TradeSignal {
                strategy_id: self.id.clone(),
                symbol: ctx.symbol.clone(),
                qty: desired_qty,
                limit_price: Some(ctx.bid),
                reason: "momentum_down_ai_confirmed".into(),
                confidence: self.last_ai_confidence,
            });
        }

        None
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}
