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

impl OpenRequest {
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            !self.client_order_id.trim().is_empty(),
            "client_order_id cannot be empty"
        );
        anyhow::ensure!(
            self.client_order_id.len() <= 128,
            "client_order_id too long"
        );
        anyhow::ensure!(!self.symbol.trim().is_empty(), "symbol cannot be empty");
        anyhow::ensure!(self.symbol.len() <= 64, "symbol too long");
        anyhow::ensure!(
            self.symbol
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '-' | '_' | '.')),
            "symbol contains invalid characters"
        );
        anyhow::ensure!(
            self.quantity.is_finite() && self.quantity > 0.0,
            "quantity must be finite and positive"
        );

        match self.order_type {
            OrderType::Limit => {
                let price = self
                    .limit_price
                    .ok_or_else(|| anyhow::anyhow!("limit order requires limit_price"))?;
                anyhow::ensure!(
                    price.is_finite() && price > 0.0,
                    "limit_price must be finite and positive"
                );
            }
            OrderType::Market => {
                anyhow::ensure!(
                    self.limit_price.is_none(),
                    "market order must not include limit_price"
                );
            }
        }

        Ok(())
    }
}

#[async_trait]
pub trait ExecutionGateway: Send + Sync {
    fn name(&self) -> &str;
    async fn submit_order(&self, req: OpenRequest) -> Result<OrderEvent, anyhow::Error>;
}
