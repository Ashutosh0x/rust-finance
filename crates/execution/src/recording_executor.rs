/// RecordingExecutor — captures submitted orders for post-test assertion.
/// Used exclusively in integration tests to verify the full pipeline
/// (signal → risk interceptor → OMS → executor) composes correctly.
use crate::gateway::{ExecutionGateway, OpenRequest};
use async_trait::async_trait;
use common::events::{OrderEvent, OrderFilled};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct RecordingExecutor {
    submissions: Arc<Mutex<Vec<OpenRequest>>>,
}

impl RecordingExecutor {
    pub fn new() -> Self {
        Self {
            submissions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns a snapshot of all orders submitted so far.
    pub fn submitted_orders(&self) -> Vec<OpenRequest> {
        self.submissions.lock().unwrap().clone()
    }

    /// How many orders have been submitted.
    pub fn submission_count(&self) -> usize {
        self.submissions.lock().unwrap().len()
    }

    /// Clear recorded submissions.
    pub fn clear(&self) {
        self.submissions.lock().unwrap().clear();
    }
}

#[async_trait]
impl ExecutionGateway for RecordingExecutor {
    fn name(&self) -> &str {
        "RecordingExecutor"
    }

    async fn submit_order(&self, req: OpenRequest) -> Result<OrderEvent, anyhow::Error> {
        req.validate()?;

        self.submissions.lock().unwrap().push(req.clone());
        // Return a synthetic fill so the caller can proceed
        Ok(OrderEvent::Filled(OrderFilled {
            client_order_id: req.client_order_id,
            fill_price: req.limit_price.unwrap_or(100.0),
            fill_quantity: req.quantity,
            commission: 0.0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::TimeInForce;
    use common::events::{OrderSide, OrderType};

    fn sample_request() -> OpenRequest {
        OpenRequest {
            client_order_id: "test-001".into(),
            symbol: "NVDA".into(),
            side: OrderSide::Buy,
            quantity: 10.0,
            order_type: OrderType::Limit,
            limit_price: Some(900.0),
            time_in_force: TimeInForce::DAY,
        }
    }

    #[tokio::test]
    async fn test_recording_executor_captures_orders() {
        let exec = RecordingExecutor::new();
        assert_eq!(exec.submission_count(), 0);

        let result = exec.submit_order(sample_request()).await;
        assert!(result.is_ok());
        assert_eq!(exec.submission_count(), 1);

        let orders = exec.submitted_orders();
        assert_eq!(orders[0].symbol.as_str(), "NVDA");
        assert_eq!(orders[0].quantity, 10.0);
    }

    #[tokio::test]
    async fn test_recording_executor_multiple_orders() {
        let exec = RecordingExecutor::new();
        for i in 0..5 {
            let mut req = sample_request();
            req.client_order_id = format!("order-{}", i).into();
            let _ = exec.submit_order(req).await;
        }
        assert_eq!(exec.submission_count(), 5);
    }

    #[tokio::test]
    async fn test_recording_executor_clear() {
        let exec = RecordingExecutor::new();
        let _ = exec.submit_order(sample_request()).await;
        assert_eq!(exec.submission_count(), 1);
        exec.clear();
        assert_eq!(exec.submission_count(), 0);
    }
}
