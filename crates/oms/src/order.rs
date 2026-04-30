// crates/oms/src/order.rs
//
// Order lifecycle state machine.
// PENDING → SUBMITTED → PARTIAL_FILL → FILLED → CANCELLED / REJECTED

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// All possible states an order can occupy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrderStatus {
    Pending,
    Submitted,
    PartialFill { filled_qty: f64 },
    Filled,
    Cancelled,
    Rejected { reason: String },
    Expired,
}

impl OrderStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            OrderStatus::Filled
                | OrderStatus::Cancelled
                | OrderStatus::Rejected { .. }
                | OrderStatus::Expired
        )
    }

    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit { price: f64 },
    StopLimit { stop: f64, limit: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TimeInForce {
    Day,
    GoodTillCancel,
    ImmediateOrCancel,
    FillOrKill,
}

/// A complete order record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: Uuid,
    /// Client-assigned idempotency key — prevents double-sends on reconnect.
    pub client_order_id: String,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub quantity: f64,
    pub time_in_force: TimeInForce,
    pub status: OrderStatus,
    /// Cumulative filled quantity.
    pub filled_qty: f64,
    /// Volume-weighted average fill price.
    pub avg_fill_price: Option<f64>,
    /// Exchange-assigned order ID (set after submission).
    pub exchange_order_id: Option<String>,
    /// AI signal that triggered this order.
    pub signal_source: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub fills: Vec<Fill>,
}

/// A single fill event against an order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fill {
    pub fill_id: Uuid,
    pub order_id: Uuid,
    pub qty: f64,
    pub price: f64,
    pub timestamp: DateTime<Utc>,
    pub commission: f64,
}

/// Events that drive order state transitions.
#[derive(Debug, Clone)]
pub enum OrderEvent {
    Submit {
        exchange_order_id: String,
    },
    FillReceived {
        qty: f64,
        price: f64,
        commission: f64,
    },
    Cancel,
    Reject {
        reason: String,
    },
    Expire,
}

impl Order {
    pub fn new(
        client_order_id: impl Into<String>,
        symbol: impl Into<String>,
        side: Side,
        order_type: OrderType,
        quantity: f64,
        tif: TimeInForce,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            client_order_id: client_order_id.into(),
            symbol: symbol.into(),
            side,
            order_type,
            quantity,
            time_in_force: tif,
            status: OrderStatus::Pending,
            filled_qty: 0.0,
            avg_fill_price: None,
            exchange_order_id: None,
            signal_source: None,
            created_at: now,
            updated_at: now,
            fills: Vec::new(),
        }
    }

    /// Apply an event, transitioning the order state machine.
    /// Returns `Err` if the transition is invalid for the current state.
    pub fn apply(&mut self, event: OrderEvent) -> Result<(), String> {
        if self.status.is_terminal() {
            return Err(format!(
                "Cannot apply {:?} to terminal order {:?}",
                event, self.status
            ));
        }

        match event {
            OrderEvent::Submit { exchange_order_id } => {
                if self.status != OrderStatus::Pending {
                    return Err("Only Pending orders can be submitted".into());
                }
                self.exchange_order_id = Some(exchange_order_id);
                self.status = OrderStatus::Submitted;
            }

            OrderEvent::FillReceived {
                qty,
                price,
                commission,
            } => {
                if !qty.is_finite() || qty <= 0.0 {
                    return Err(format!("Invalid fill quantity: {qty}"));
                }
                if !price.is_finite() || price <= 0.0 {
                    return Err(format!("Invalid fill price: {price}"));
                }
                if !commission.is_finite() || commission < 0.0 {
                    return Err(format!("Invalid commission: {commission}"));
                }
                if qty > self.remaining_qty() + 1e-9 {
                    return Err(format!(
                        "Overfill rejected: fill {qty} exceeds remaining {}",
                        self.remaining_qty()
                    ));
                }

                // Update VWAP
                let prev_notional = self.avg_fill_price.unwrap_or(0.0) * self.filled_qty;
                self.filled_qty += qty;
                self.avg_fill_price = Some((prev_notional + price * qty) / self.filled_qty);

                self.fills.push(Fill {
                    fill_id: Uuid::new_v4(),
                    order_id: self.id,
                    qty,
                    price,
                    timestamp: Utc::now(),
                    commission,
                });

                self.status = if (self.filled_qty - self.quantity).abs() < 1e-9 {
                    OrderStatus::Filled
                } else {
                    OrderStatus::PartialFill {
                        filled_qty: self.filled_qty,
                    }
                };
            }

            OrderEvent::Cancel => {
                self.status = OrderStatus::Cancelled;
            }

            OrderEvent::Reject { reason } => {
                self.status = OrderStatus::Rejected { reason };
            }

            OrderEvent::Expire => {
                self.status = OrderStatus::Expired;
            }
        }

        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn remaining_qty(&self) -> f64 {
        (self.quantity - self.filled_qty).max(0.0)
    }

    pub fn notional_value(&self) -> Option<f64> {
        self.avg_fill_price.map(|p| p * self.filled_qty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_order() -> Order {
        Order::new(
            "CL-001",
            "AAPL",
            Side::Buy,
            OrderType::Limit { price: 175.0 },
            100.0,
            TimeInForce::Day,
        )
    }

    #[test]
    fn test_full_fill_lifecycle() {
        let mut o = make_order();
        assert_eq!(o.status, OrderStatus::Pending);

        o.apply(OrderEvent::Submit {
            exchange_order_id: "EX-001".into(),
        })
        .unwrap();
        assert_eq!(o.status, OrderStatus::Submitted);

        o.apply(OrderEvent::FillReceived {
            qty: 50.0,
            price: 174.5,
            commission: 0.5,
        })
        .unwrap();
        assert!(matches!(o.status, OrderStatus::PartialFill { .. }));

        o.apply(OrderEvent::FillReceived {
            qty: 50.0,
            price: 175.0,
            commission: 0.5,
        })
        .unwrap();
        assert_eq!(o.status, OrderStatus::Filled);

        // VWAP: (174.5*50 + 175.0*50) / 100 = 174.75
        assert!((o.avg_fill_price.unwrap() - 174.75).abs() < 0.001);
    }

    #[test]
    fn test_cannot_apply_event_to_terminal_order() {
        let mut o = make_order();
        o.apply(OrderEvent::Cancel).unwrap();
        assert!(o.apply(OrderEvent::Cancel).is_err());
    }

    #[test]
    fn test_reject_transition() {
        let mut o = make_order();
        o.apply(OrderEvent::Reject {
            reason: "Insufficient funds".into(),
        })
        .unwrap();
        assert!(o.status.is_terminal());
    }

    #[test]
    fn test_overfill_rejected() {
        let mut o = make_order();
        o.apply(OrderEvent::Submit {
            exchange_order_id: "EX-001".into(),
        })
        .unwrap();

        let result = o.apply(OrderEvent::FillReceived {
            qty: 101.0,
            price: 175.0,
            commission: 0.0,
        });

        assert!(result.is_err());
        assert_eq!(o.filled_qty, 0.0);
    }
}
