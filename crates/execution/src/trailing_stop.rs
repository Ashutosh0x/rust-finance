// crates/execution/src/trailing_stop.rs
// Dynamic stop that follows price at a fixed $ or % distance.
// On each price tick, the stop moves up (for longs) but NEVER back down.

use common::models::order::{Order, OrderId, OrderSide, OrderStatus, OrderType};
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct TrailingStop {
    pub id: String,
    pub symbol: String,
    pub side: TrailingStopSide,
    pub trail: TrailSpec,
    /// Current stop price — updates as market moves in our favour
    pub current_stop_price: f64,
    /// Best price seen since position opened (highest for long, lowest for short)
    pub best_price: f64,
    pub quantity: f64,
    pub triggered: bool,
    pub order_id: Option<OrderId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrailingStopSide {
    /// Long position — stop trails below price
    Long,
    /// Short position — stop trails above price
    Short,
}

#[derive(Debug, Clone)]
pub enum TrailSpec {
    /// Fixed dollar amount below/above price
    Fixed { distance: f64 },
    /// Percentage of current price
    Percent { pct: f64 },
    /// ATR-based trail (multiplier × ATR)
    Atr { multiplier: f64, atr: f64 },
}

impl TrailSpec {
    pub fn distance_at(&self, price: f64) -> f64 {
        match self {
            TrailSpec::Fixed { distance } => *distance,
            TrailSpec::Percent { pct } => price * pct / 100.0,
            TrailSpec::Atr { multiplier, atr } => multiplier * atr,
        }
    }
}

pub struct TrailingStopEngine {
    stops: HashMap<String, TrailingStop>,
    order_tx: Sender<Order>,
}

impl TrailingStopEngine {
    pub fn new(order_tx: Sender<Order>) -> Self {
        Self {
            stops: HashMap::new(),
            order_tx,
        }
    }

    /// Register a new trailing stop. entry_price is the fill price of the position.
    pub fn add(&mut self, mut stop: TrailingStop, entry_price: f64) {
        let dist = stop.trail.distance_at(entry_price);
        stop.best_price = entry_price;
        stop.current_stop_price = match stop.side {
            TrailingStopSide::Long => entry_price - dist,
            TrailingStopSide::Short => entry_price + dist,
        };
        tracing::info!(
            id = %stop.id, symbol = %stop.symbol,
            initial_stop = stop.current_stop_price,
            "Trailing stop registered"
        );
        self.stops.insert(stop.id.clone(), stop);
    }

    /// Process a price update tick. Call on every MarketEvent for the symbol.
    /// Returns list of stop IDs that were triggered.
    pub async fn on_price(&mut self, symbol: &str, price: f64) -> Vec<String> {
        let mut triggered = Vec::new();

        for stop in self.stops.values_mut() {
            if stop.symbol != symbol || stop.triggered {
                continue;
            }

            match stop.side {
                TrailingStopSide::Long => {
                    // Check if stop was hit
                    if price <= stop.current_stop_price {
                        stop.triggered = true;
                        triggered.push(stop.id.clone());
                        tracing::warn!(
                            id = %stop.id, symbol, price, stop = stop.current_stop_price,
                            "Trailing stop triggered (LONG)"
                        );
                    } else if price > stop.best_price {
                        // Price moved in our favour — raise the stop
                        stop.best_price = price;
                        let dist = stop.trail.distance_at(price);
                        let new_stop = price - dist;
                        if new_stop > stop.current_stop_price {
                            stop.current_stop_price = new_stop;
                            tracing::debug!(id = %stop.id, new_stop, "Trailing stop raised");
                        }
                    }
                }
                TrailingStopSide::Short => {
                    // Check if stop was hit
                    if price >= stop.current_stop_price {
                        stop.triggered = true;
                        triggered.push(stop.id.clone());
                        tracing::warn!(
                            id = %stop.id, symbol, price, stop = stop.current_stop_price,
                            "Trailing stop triggered (SHORT)"
                        );
                    } else if price < stop.best_price {
                        // Price moved in our favour — lower the stop
                        stop.best_price = price;
                        let dist = stop.trail.distance_at(price);
                        let new_stop = price + dist;
                        if new_stop < stop.current_stop_price {
                            stop.current_stop_price = new_stop;
                            tracing::debug!(id = %stop.id, new_stop, "Trailing stop lowered");
                        }
                    }
                }
            }
        }

        // Submit market orders for triggered stops
        for stop_id in &triggered {
            if let Some(stop) = self.stops.get(stop_id) {
                let close_side = match stop.side {
                    TrailingStopSide::Long => OrderSide::Sell,
                    TrailingStopSide::Short => OrderSide::Buy,
                };
                let order = Order {
                    id: format!("{}-TS-FILL", stop_id),
                    symbol: stop.symbol.clone(),
                    side: close_side,
                    order_type: OrderType::Market,
                    quantity: stop.quantity,
                    limit_price: None,
                    stop_price: None,
                    status: OrderStatus::Pending,
                };
                let _ = self.order_tx.send(order).await;
            }
        }

        triggered
    }

    pub fn remove(&mut self, id: &str) {
        self.stops.remove(id);
    }

    pub fn get_all_for_symbol(&self, symbol: &str) -> Vec<&TrailingStop> {
        self.stops.values().filter(|s| s.symbol == symbol).collect()
    }
}
