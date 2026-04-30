use crate::gateway::{ExecutionGateway, OpenRequest};
use async_trait::async_trait;
use common::events::{OrderEvent, OrderFilled};

pub struct MockExecutor;

impl MockExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ExecutionGateway for MockExecutor {
    fn name(&self) -> &str {
        "MockExecutor"
    }

    async fn submit_order(&self, req: OpenRequest) -> Result<OrderEvent, anyhow::Error> {
        req.validate()?;

        Ok(OrderEvent::Filled(OrderFilled {
            client_order_id: req.client_order_id,
            fill_price: req.limit_price.unwrap_or(100.0), // Stub price
            fill_quantity: req.quantity,
            commission: 0.0,
        }))
    }
}
