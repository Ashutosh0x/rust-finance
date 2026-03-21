use async_trait::async_trait;
use common::events::{Envelope, MarketEvent, SignalEvent, OrderSide};
use compact_str::CompactString;

#[async_trait]
pub trait Strategy: Send + Sync {
    fn name(&self) -> &str;
    async fn on_market_event(&mut self, event: &Envelope<MarketEvent>) -> Vec<SignalEvent>;
}

pub struct SimpleMomentum {
    window: usize,
}

impl SimpleMomentum {
    pub fn new(window: usize) -> Self {
        Self { window }
    }
}

#[async_trait]
impl Strategy for SimpleMomentum {
    fn name(&self) -> &str {
        "SimpleMomentum"
    }

    async fn on_market_event(&mut self, env: &Envelope<MarketEvent>) -> Vec<SignalEvent> {
        vec![SignalEvent {
            symbol: CompactString::new(env.payload.symbol()),
            direction: OrderSide::Buy,
            confidence: 0.1,
            strategy_id: CompactString::new("SM1"),
        }]
    }
}
