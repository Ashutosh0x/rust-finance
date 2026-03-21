use crate::state::EngineState;
use execution::gateway::OpenRequest;
use compact_str::CompactString;

pub enum RiskVerdict {
    Approved,
    Blocked { reason: CompactString },
    Modified { new_request: OpenRequest, reason: CompactString },
}

pub trait RiskInterceptor: Send + Sync {
    fn evaluate(&self, state: &EngineState, req: &OpenRequest) -> RiskVerdict;
}

pub struct RiskChain {
    interceptors: Vec<Box<dyn RiskInterceptor>>,
}

impl RiskChain {
    pub fn new() -> Self {
        Self { interceptors: Vec::new() }
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

pub struct MaxPositionSize { pub max_quantity: f64 }
impl RiskInterceptor for MaxPositionSize {
    fn evaluate(&self, _state: &EngineState, req: &OpenRequest) -> RiskVerdict {
        if req.quantity > self.max_quantity {
            RiskVerdict::Blocked { reason: "Exceeds max position size".into() }
        } else {
            RiskVerdict::Approved
        }
    }
}

pub struct MaxDrawdown { pub max_drawdown_pct: f64 }
impl RiskInterceptor for MaxDrawdown {
    fn evaluate(&self, state: &EngineState, _req: &OpenRequest) -> RiskVerdict {
        if state.current_drawdown_pct > self.max_drawdown_pct {
            RiskVerdict::Blocked { reason: "Exceeds max drawdown".into() }
        } else {
            RiskVerdict::Approved
        }
    }
}

pub struct MaxOpenOrders { pub max_orders: usize }
impl RiskInterceptor for MaxOpenOrders {
    fn evaluate(&self, state: &EngineState, _req: &OpenRequest) -> RiskVerdict {
        if state.open_order_count >= self.max_orders {
            RiskVerdict::Blocked { reason: "Max open orders reached".into() }
        } else {
            RiskVerdict::Approved
        }
    }
}

pub struct DailyLossLimit { pub max_daily_loss: f64 }
impl RiskInterceptor for DailyLossLimit {
    fn evaluate(&self, state: &EngineState, _req: &OpenRequest) -> RiskVerdict {
        if state.daily_pnl <= -self.max_daily_loss {
            RiskVerdict::Blocked { reason: "Max daily loss exceeded".into() }
        } else {
            RiskVerdict::Approved
        }
    }
}
