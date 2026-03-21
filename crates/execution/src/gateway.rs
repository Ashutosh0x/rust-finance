use async_trait::async_trait;
use common::events::{OrderEvent, OrderSide, OrderType};
use compact_str::CompactString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInForce {
    DAY,
    GTC,
    IOC,
    FOK,
}

#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub client_order_id: CompactString,
    pub symbol: CompactString,
    pub side: OrderSide,
    pub quantity: f64,
    pub order_type: OrderType,
    pub limit_price: Option<f64>,
    pub time_in_force: TimeInForce,
}

#[async_trait]
pub trait ExecutionGateway: Send + Sync {
    fn name(&self) -> &str;
    async fn submit_order(&self, req: OpenRequest) -> Result<OrderEvent, anyhow::Error>;
}
