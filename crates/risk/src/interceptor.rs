use crate::state::EngineState;
use compact_str::CompactString;
use execution::gateway::OpenRequest;

#[derive(Debug)]
pub enum RiskVerdict {
    Approved,
    Blocked {
        reason: CompactString,
    },
    Modified {
        new_request: OpenRequest,
        reason: CompactString,
    },
}

pub trait RiskInterceptor: Send + Sync {
    fn evaluate(&self, state: &EngineState, req: &OpenRequest) -> RiskVerdict;
}

pub struct RiskChain {
    interceptors: Vec<Box<dyn RiskInterceptor>>,
}

impl RiskChain {
    pub fn new() -> Self {
        Self {
            interceptors: Vec::new(),
        }
    }

    pub fn add(mut self, interceptor: impl RiskInterceptor + 'static) -> Self {
        self.interceptors.push(Box::new(interceptor));
        self
    }

    pub fn evaluate(&self, state: &EngineState, req: &OpenRequest) -> RiskVerdict {
        for interceptor in &self.interceptors {
            match interceptor.evaluate(state, req) {
                RiskVerdict::Approved => continue,
                other => return other,
            }
        }
        RiskVerdict::Approved
    }
}

// Concrete Implementations

pub struct MaxPositionSize {
    pub max_quantity: f64,
}
impl RiskInterceptor for MaxPositionSize {
    fn evaluate(&self, _state: &EngineState, req: &OpenRequest) -> RiskVerdict {
        if req.quantity > self.max_quantity {
            RiskVerdict::Blocked {
                reason: "Exceeds max position size".into(),
            }
        } else {
            RiskVerdict::Approved
        }
    }
}

pub struct MaxDrawdown {
    pub max_drawdown_pct: f64,
}
impl RiskInterceptor for MaxDrawdown {
    fn evaluate(&self, state: &EngineState, _req: &OpenRequest) -> RiskVerdict {
        if state.current_drawdown_pct > self.max_drawdown_pct {
            RiskVerdict::Blocked {
                reason: "Exceeds max drawdown".into(),
            }
        } else {
            RiskVerdict::Approved
        }
    }
}

pub struct MaxOpenOrders {
    pub max_orders: usize,
}
impl RiskInterceptor for MaxOpenOrders {
    fn evaluate(&self, state: &EngineState, _req: &OpenRequest) -> RiskVerdict {
        if state.open_order_count >= self.max_orders {
            RiskVerdict::Blocked {
                reason: "Max open orders reached".into(),
            }
        } else {
            RiskVerdict::Approved
        }
    }
}

pub struct DailyLossLimit {
    pub max_daily_loss: f64,
}
impl RiskInterceptor for DailyLossLimit {
    fn evaluate(&self, state: &EngineState, _req: &OpenRequest) -> RiskVerdict {
        if state.daily_pnl <= -self.max_daily_loss {
            RiskVerdict::Blocked {
                reason: "Max daily loss exceeded".into(),
            }
        } else {
            RiskVerdict::Approved
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::events::{OrderSide, OrderType};
    use execution::gateway::TimeInForce;

    fn default_state() -> EngineState {
        EngineState {
            total_equity: 100_000.0,
            daily_pnl: 0.0,
            current_drawdown_pct: 0.0,
            open_order_count: 2,
            daily_trade_count: 5,
            open_orders: vec![],
        }
    }

    fn sample_request(qty: f64) -> OpenRequest {
        OpenRequest {
            client_order_id: "test-001".into(),
            symbol: "NVDA".into(),
            side: OrderSide::Buy,
            quantity: qty,
            order_type: OrderType::Limit,
            limit_price: Some(900.0),
            time_in_force: TimeInForce::DAY,
        }
    }

    #[test]
    fn test_chain_passes_clean_order() {
        let chain = RiskChain::new()
            .add(MaxPositionSize {
                max_quantity: 100.0,
            })
            .add(MaxDrawdown {
                max_drawdown_pct: 0.05,
            })
            .add(MaxOpenOrders { max_orders: 10 })
            .add(DailyLossLimit {
                max_daily_loss: 5000.0,
            });

        let state = default_state();
        let req = sample_request(10.0);
        let verdict = chain.evaluate(&state, &req);
        assert!(matches!(verdict, RiskVerdict::Approved));
    }

    #[test]
    fn test_chain_blocks_oversized_order() {
        let chain = RiskChain::new().add(MaxPositionSize { max_quantity: 5.0 });

        let state = default_state();
        let req = sample_request(10.0); // exceeds max of 5
        let verdict = chain.evaluate(&state, &req);
        assert!(matches!(verdict, RiskVerdict::Blocked { .. }));
        if let RiskVerdict::Blocked { reason } = verdict {
            assert!(reason.contains("position size"), "Block reason: {}", reason);
        }
    }

    #[test]
    fn test_chain_blocks_drawdown() {
        let chain = RiskChain::new().add(MaxDrawdown {
            max_drawdown_pct: 0.05,
        });

        let mut state = default_state();
        state.current_drawdown_pct = 0.08; // 8% > 5% limit
        let req = sample_request(1.0);
        let verdict = chain.evaluate(&state, &req);
        assert!(matches!(verdict, RiskVerdict::Blocked { .. }));
    }

    #[test]
    fn test_chain_blocks_daily_loss() {
        let chain = RiskChain::new().add(DailyLossLimit {
            max_daily_loss: 2000.0,
        });

        let mut state = default_state();
        state.daily_pnl = -2500.0; // -$2500 > -$2000 limit
        let req = sample_request(1.0);
        let verdict = chain.evaluate(&state, &req);
        assert!(matches!(verdict, RiskVerdict::Blocked { .. }));
    }

    #[test]
    fn test_chain_blocks_max_open_orders() {
        let chain = RiskChain::new().add(MaxOpenOrders { max_orders: 2 });

        let mut state = default_state();
        state.open_order_count = 3; // at limit
        let req = sample_request(1.0);
        let verdict = chain.evaluate(&state, &req);
        assert!(matches!(verdict, RiskVerdict::Blocked { .. }));
    }

    #[test]
    fn test_chain_first_failure_wins() {
        // Both interceptors would block, but the first one's reason should be returned
        let chain = RiskChain::new()
            .add(MaxPositionSize { max_quantity: 1.0 }) // will block first
            .add(MaxDrawdown {
                max_drawdown_pct: 0.01,
            }); // would also block

        let mut state = default_state();
        state.current_drawdown_pct = 0.10;
        let req = sample_request(100.0);
        let verdict = chain.evaluate(&state, &req);
        if let RiskVerdict::Blocked { reason } = verdict {
            assert!(
                reason.contains("position size"),
                "First interceptor should win, got: {}",
                reason
            );
        } else {
            panic!("Should have been blocked");
        }
    }

    #[test]
    fn test_empty_chain_approves_everything() {
        let chain = RiskChain::new();
        let state = default_state();
        let req = sample_request(999999.0);
        let verdict = chain.evaluate(&state, &req);
        assert!(
            matches!(verdict, RiskVerdict::Approved),
            "Empty chain should approve everything"
        );
    }

    /// E2E: feed an order through risk chain → if approved → submit to RecordingExecutor.
    /// This tests the full composition path without any network calls.
    #[tokio::test]
    async fn test_e2e_signal_to_recording_executor() {
        use execution::gateway::ExecutionGateway;
        use execution::recording_executor::RecordingExecutor;

        let chain = RiskChain::new()
            .add(MaxPositionSize {
                max_quantity: 100.0,
            })
            .add(MaxDrawdown {
                max_drawdown_pct: 0.05,
            });

        let executor = RecordingExecutor::new();
        let state = default_state();

        // Test 1: Clean order → should reach executor
        let req = sample_request(10.0);
        match chain.evaluate(&state, &req) {
            RiskVerdict::Approved => {
                let result = executor.submit_order(req).await;
                assert!(result.is_ok());
            }
            other => panic!("Expected Approved, got {:?}", other),
        }
        assert_eq!(
            executor.submission_count(),
            1,
            "Order should reach executor"
        );

        // Test 2: Oversized order → should NOT reach executor
        let oversized = sample_request(200.0);
        match chain.evaluate(&state, &oversized) {
            RiskVerdict::Blocked { reason } => {
                assert!(reason.contains("position size"));
                // Do NOT submit — this is the point
            }
            other => panic!("Expected Blocked, got {:?}", other),
        }
        assert_eq!(
            executor.submission_count(),
            1,
            "Blocked order must NOT reach executor"
        );
    }
}
