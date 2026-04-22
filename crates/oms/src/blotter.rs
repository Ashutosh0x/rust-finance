// crates/oms/src/blotter.rs
//
// Order blotter — live registry of all orders with pre-trade compliance checks.
// Acts as the single source of truth for order state.

use crate::order::{Order, OrderEvent, OrderType, Side, TimeInForce};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Pre-trade compliance limits.
#[derive(Debug, Clone)]
pub struct ComplianceLimits {
    /// Max quantity per single order.
    pub max_order_qty: f64,
    /// Max notional value per single order (USD).
    pub max_order_notional: f64,
    /// Max open orders across the book.
    pub max_open_orders: usize,
    /// Max gross exposure per symbol.
    pub max_symbol_exposure: f64,
    /// Daily traded value cap (resets at midnight).
    pub daily_turnover_cap: f64,
}

impl Default for ComplianceLimits {
    fn default() -> Self {
        Self {
            max_order_qty: 10_000.0,
            max_order_notional: 1_000_000.0,
            max_open_orders: 50,
            max_symbol_exposure: 500_000.0,
            daily_turnover_cap: 5_000_000.0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ComplianceError {
    #[error("Order quantity {qty} exceeds max {max}")]
    QuantityExceeded { qty: f64, max: f64 },
    #[error("Order notional {notional:.2} exceeds max {max:.2}")]
    NotionalExceeded { notional: f64, max: f64 },
    #[error("Too many open orders: {count} / {max}")]
    TooManyOpenOrders { count: usize, max: usize },
    #[error("Symbol exposure {exposure:.2} would exceed limit {limit:.2}")]
    ExposureExceeded { exposure: f64, limit: f64 },
    #[error("Daily turnover cap {cap:.2} would be breached")]
    DailyCapBreached { cap: f64 },
    #[error("Duplicate client_order_id: {id}")]
    DuplicateOrderId { id: String },
    #[error("Order {id} not found")]
    OrderNotFound { id: Uuid },
}

struct BlotterState {
    /// All orders indexed by their UUID.
    orders: HashMap<Uuid, Order>,
    /// Dedup map: client_order_id → order UUID.
    client_id_index: HashMap<String, Uuid>,
    /// Daily turnover accumulator.
    daily_turnover: f64,
    /// Per-symbol gross exposure.
    symbol_exposure: HashMap<String, f64>,
}

/// Thread-safe order blotter.
#[derive(Clone)]
pub struct OrderBlotter {
    state: Arc<RwLock<BlotterState>>,
    limits: ComplianceLimits,
}

impl OrderBlotter {
    pub fn new(limits: ComplianceLimits) -> Self {
        Self {
            state: Arc::new(RwLock::new(BlotterState {
                orders: HashMap::new(),
                client_id_index: HashMap::new(),
                daily_turnover: 0.0,
                symbol_exposure: HashMap::new(),
            })),
            limits,
        }
    }

    /// Run pre-trade compliance checks and submit the order to the blotter.
    /// Returns the order UUID if accepted.
    pub async fn submit(
        &self,
        client_order_id: impl Into<String>,
        symbol: impl Into<String>,
        side: Side,
        order_type: OrderType,
        quantity: f64,
        tif: TimeInForce,
    ) -> Result<Uuid, ComplianceError> {
        let client_order_id = client_order_id.into();
        let symbol = symbol.into();

        // Estimate notional
        let price_est = match &order_type {
            OrderType::Limit { price } => *price,
            OrderType::StopLimit { limit, .. } => *limit,
            OrderType::Market => f64::MAX, // worst-case for compliance
        };
        let notional = if price_est == f64::MAX {
            self.limits.max_order_notional // treat market orders as max notional
        } else {
            price_est * quantity
        };

        // SINGLE write lock — prevents TOCTOU race between compliance check and insert
        let mut state = self.state.write().await;

        // Dedup check
        if state.client_id_index.contains_key(&client_order_id) {
            return Err(ComplianceError::DuplicateOrderId {
                id: client_order_id,
            });
        }

        // Quantity check
        if quantity > self.limits.max_order_qty {
            return Err(ComplianceError::QuantityExceeded {
                qty: quantity,
                max: self.limits.max_order_qty,
            });
        }

        // Notional check
        if notional > self.limits.max_order_notional {
            return Err(ComplianceError::NotionalExceeded {
                notional,
                max: self.limits.max_order_notional,
            });
        }

        // Open orders check
        let open_count = state
            .orders
            .values()
            .filter(|o| o.status.is_active())
            .count();
        if open_count >= self.limits.max_open_orders {
            return Err(ComplianceError::TooManyOpenOrders {
                count: open_count,
                max: self.limits.max_open_orders,
            });
        }

        // Symbol exposure check
        let current_exposure = state.symbol_exposure.get(&symbol).copied().unwrap_or(0.0);
        if current_exposure + notional > self.limits.max_symbol_exposure {
            return Err(ComplianceError::ExposureExceeded {
                exposure: current_exposure + notional,
                limit: self.limits.max_symbol_exposure,
            });
        }

        // Daily turnover check
        if state.daily_turnover + notional > self.limits.daily_turnover_cap {
            return Err(ComplianceError::DailyCapBreached {
                cap: self.limits.daily_turnover_cap,
            });
        }

        // All checks passed — record the order (still under the same write lock)
        let order = Order::new(
            client_order_id.clone(),
            symbol.clone(),
            side,
            order_type,
            quantity,
            tif,
        );
        let order_id = order.id;

        state.client_id_index.insert(client_order_id, order_id);
        *state.symbol_exposure.entry(symbol).or_insert(0.0) += notional;
        state.orders.insert(order_id, order);

        Ok(order_id)
    }

    /// Apply an event to an existing order.
    pub async fn apply_event(
        &self,
        order_id: Uuid,
        event: OrderEvent,
    ) -> Result<(), ComplianceError> {
        let mut state = self.state.write().await;

        // Update daily turnover on fills
        if let OrderEvent::FillReceived { qty, price, .. } = &event {
            state.daily_turnover += qty * price;
        }

        let order = state
            .orders
            .get_mut(&order_id)
            .ok_or(ComplianceError::OrderNotFound { id: order_id })?;

        order
            .apply(event)
            .map_err(|_e| ComplianceError::OrderNotFound { id: order_id })
    }

    pub async fn get_order(&self, id: Uuid) -> Option<Order> {
        self.state.read().await.orders.get(&id).cloned()
    }

    pub async fn open_orders(&self) -> Vec<Order> {
        self.state
            .read()
            .await
            .orders
            .values()
            .filter(|o| o.status.is_active())
            .cloned()
            .collect()
    }

    pub async fn all_orders(&self) -> Vec<Order> {
        self.state.read().await.orders.values().cloned().collect()
    }

    pub async fn reset_daily_counters(&self) {
        let mut state = self.state.write().await;
        state.daily_turnover = 0.0;
        tracing::info!("Daily compliance counters reset");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::OrderStatus;

    #[tokio::test]
    async fn test_order_submit_and_retrieve() {
        let blotter = OrderBlotter::new(ComplianceLimits::default());
        let id = blotter
            .submit(
                "C001",
                "AAPL",
                Side::Buy,
                OrderType::Limit { price: 150.0 },
                100.0,
                TimeInForce::Day,
            )
            .await
            .unwrap();

        let order = blotter.get_order(id).await.unwrap();
        assert_eq!(order.symbol, "AAPL");
        assert_eq!(order.status, OrderStatus::Pending);
    }

    #[tokio::test]
    async fn test_duplicate_order_rejected() {
        let blotter = OrderBlotter::new(ComplianceLimits::default());
        blotter
            .submit(
                "C001",
                "AAPL",
                Side::Buy,
                OrderType::Limit { price: 150.0 },
                100.0,
                TimeInForce::Day,
            )
            .await
            .unwrap();
        let result = blotter
            .submit(
                "C001",
                "AAPL",
                Side::Buy,
                OrderType::Limit { price: 150.0 },
                100.0,
                TimeInForce::Day,
            )
            .await;
        assert!(matches!(
            result,
            Err(ComplianceError::DuplicateOrderId { .. })
        ));
    }

    #[tokio::test]
    async fn test_quantity_limit_enforced() {
        let limits = ComplianceLimits {
            max_order_qty: 50.0,
            ..Default::default()
        };
        let blotter = OrderBlotter::new(limits);
        let result = blotter
            .submit(
                "C001",
                "AAPL",
                Side::Buy,
                OrderType::Market,
                100.0,
                TimeInForce::Day,
            )
            .await;
        assert!(matches!(
            result,
            Err(ComplianceError::QuantityExceeded { .. })
        ));
    }
}
