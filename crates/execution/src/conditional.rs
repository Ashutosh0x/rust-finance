// crates/execution/src/conditional.rs
// Conditional orders: "If X AND Y AND Z → submit order"
// e.g. "If AAPL > $200 AND RSI < 70 AND BTC dominance falling → buy 100 shares"

use common::models::order::{Order, OrderSide, OrderStatus, OrderType};
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct ConditionalOrder {
    pub id: String,
    pub name: String,
    /// ALL conditions must be true simultaneously to trigger
    pub conditions: Vec<Condition>,
    pub logic: LogicMode,
    pub order_template: OrderTemplate,
    pub state: ConditionalState,
    /// One-shot: disable after first trigger
    pub one_shot: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogicMode {
    /// All conditions must be true (AND)
    All,
    /// Any condition must be true (OR)
    Any,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConditionalState {
    Watching,
    Triggered,
    Disabled,
}

#[derive(Debug, Clone)]
pub enum Condition {
    /// Price of symbol crosses level
    PriceAbove {
        symbol: String,
        level: f64,
    },
    PriceBelow {
        symbol: String,
        level: f64,
    },
    /// RSI of symbol crosses level
    RsiBelow {
        symbol: String,
        level: f64,
    },
    RsiAbove {
        symbol: String,
        level: f64,
    },
    /// Volume exceeds threshold
    VolumeAbove {
        symbol: String,
        level: f64,
    },
    /// Price has moved % from a reference price
    PriceChange {
        symbol: String,
        pct: f64,
        direction: ChangeDirection,
    },
    /// Time-of-day condition (hour, minute in UTC)
    TimeAfter {
        hour: u8,
        minute: u8,
    },
    TimeBefore {
        hour: u8,
        minute: u8,
    },
    /// Another condition group (allows nested AND/OR)
    Group {
        conditions: Vec<Condition>,
        logic: LogicMode,
    },
}

#[derive(Debug, Clone)]
pub enum ChangeDirection {
    Up,
    Down,
    Either,
}

#[derive(Debug, Clone)]
pub struct OrderTemplate {
    pub symbol: String,
    pub side: OrderSide,
    pub quantity: f64,
    pub order_type: OrderType,
    pub limit_price: Option<f64>,
}

/// Current market snapshot for condition evaluation
#[derive(Debug, Default)]
pub struct MarketSnapshot {
    pub prices: HashMap<String, f64>,
    pub rsi: HashMap<String, f64>,
    pub volumes: HashMap<String, f64>,
    pub ref_prices: HashMap<String, f64>, // reference prices (e.g. open of day)
    pub hour_utc: u8,
    pub minute_utc: u8,
}

pub struct ConditionalEngine {
    orders: Vec<ConditionalOrder>,
    order_tx: Sender<Order>,
}

impl ConditionalEngine {
    pub fn new(order_tx: Sender<Order>) -> Self {
        Self {
            orders: Vec::new(),
            order_tx,
        }
    }

    pub fn add(&mut self, order: ConditionalOrder) {
        tracing::info!(id = %order.id, name = %order.name, conditions = order.conditions.len(), "Conditional order registered");
        self.orders.push(order);
    }

    pub fn remove(&mut self, id: &str) {
        self.orders.retain(|o| o.id != id);
    }

    /// Evaluate all watching orders against current market snapshot
    pub async fn evaluate(&mut self, snapshot: &MarketSnapshot) {
        for order in self.orders.iter_mut() {
            if order.state != ConditionalState::Watching {
                continue;
            }

            let triggered = Self::evaluate_conditions(&order.conditions, &order.logic, snapshot);

            if triggered {
                order.state = ConditionalState::Triggered;
                let submitted = Self::build_order(&order.order_template);

                tracing::info!(
                    id = %order.id,
                    name = %order.name,
                    symbol = %submitted.symbol,
                    "Conditional order triggered"
                );

                let _ = self.order_tx.send(submitted).await;

                if order.one_shot {
                    order.state = ConditionalState::Disabled;
                } else {
                    // Reset to watching after trigger (repeating)
                    order.state = ConditionalState::Watching;
                }
            }
        }
    }

    fn evaluate_conditions(
        conditions: &[Condition],
        logic: &LogicMode,
        snap: &MarketSnapshot,
    ) -> bool {
        let results: Vec<bool> = conditions
            .iter()
            .map(|c| Self::eval_condition(c, snap))
            .collect();
        match logic {
            LogicMode::All => results.iter().all(|&r| r),
            LogicMode::Any => results.iter().any(|&r| r),
        }
    }

    fn eval_condition(cond: &Condition, snap: &MarketSnapshot) -> bool {
        match cond {
            Condition::PriceAbove { symbol, level } => {
                snap.prices.get(symbol).is_some_and(|&p| p > *level)
            }
            Condition::PriceBelow { symbol, level } => {
                snap.prices.get(symbol).is_some_and(|&p| p < *level)
            }
            Condition::RsiBelow { symbol, level } => {
                snap.rsi.get(symbol).is_some_and(|&r| r < *level)
            }
            Condition::RsiAbove { symbol, level } => {
                snap.rsi.get(symbol).is_some_and(|&r| r > *level)
            }
            Condition::VolumeAbove { symbol, level } => {
                snap.volumes.get(symbol).is_some_and(|&v| v > *level)
            }
            Condition::PriceChange {
                symbol,
                pct,
                direction,
            } => {
                let price = snap.prices.get(symbol).copied().unwrap_or(0.0);
                let ref_p = snap.ref_prices.get(symbol).copied().unwrap_or(price);
                if ref_p == 0.0 {
                    return false;
                }
                let change = (price - ref_p) / ref_p * 100.0;
                match direction {
                    ChangeDirection::Up => change >= *pct,
                    ChangeDirection::Down => change <= -*pct,
                    ChangeDirection::Either => change.abs() >= *pct,
                }
            }
            Condition::TimeAfter { hour, minute } => {
                snap.hour_utc > *hour || (snap.hour_utc == *hour && snap.minute_utc >= *minute)
            }
            Condition::TimeBefore { hour, minute } => {
                snap.hour_utc < *hour || (snap.hour_utc == *hour && snap.minute_utc < *minute)
            }
            Condition::Group { conditions, logic } => {
                Self::evaluate_conditions(conditions, logic, snap)
            }
        }
    }

    fn build_order(template: &OrderTemplate) -> Order {
        Order {
            id: format!("COND-{}", uuid_v4_simple()),
            symbol: template.symbol.clone(),
            side: template.side.clone(),
            order_type: template.order_type.clone(),
            quantity: template.quantity,
            limit_price: template.limit_price,
            stop_price: None,
            status: OrderStatus::Pending,
        }
    }

    pub fn watching_count(&self) -> usize {
        self.orders
            .iter()
            .filter(|o| o.state == ConditionalState::Watching)
            .count()
    }
}

fn uuid_v4_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:016x}", ts)
}
