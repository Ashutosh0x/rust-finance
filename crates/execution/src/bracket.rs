// crates/execution/src/bracket.rs
// Bracket orders: submit take-profit + stop-loss simultaneously
// When one fills/triggers, automatically cancel the other (One-Cancels-Other)

use common::models::order::{Order, OrderId, OrderSide, OrderStatus, OrderType};
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct BracketOrder {
    pub id: BracketId,
    pub symbol: String,
    /// The entry order (market or limit)
    pub entry: Order,
    /// Take-profit limit order
    pub take_profit: BracketLeg,
    /// Stop-loss stop order
    pub stop_loss: BracketLeg,
    pub state: BracketState,
}

#[derive(Debug, Clone)]
pub struct BracketLeg {
    pub price: f64,
    pub quantity: f64,
    /// Set once the leg order is submitted to exchange
    pub order_id: Option<OrderId>,
    pub status: LegStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BracketState {
    /// Entry order not yet filled
    PendingEntry,
    /// Entry filled — both legs are now live
    Active,
    /// Take-profit filled — stop-loss cancelled
    TakeProfitFilled,
    /// Stop-loss triggered — take-profit cancelled
    StopLossFilled,
    /// Manually cancelled
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegStatus {
    Pending,
    Submitted,
    Filled,
    Cancelled,
}

pub type BracketId = String;

pub struct BracketEngine {
    brackets: HashMap<BracketId, BracketOrder>,
    /// Map from child order_id → bracket_id for fast lookup on fill events
    order_to_bracket: HashMap<OrderId, (BracketId, LegType)>,
    order_tx: Sender<Order>,
    cancel_tx: Sender<OrderId>,
}

#[derive(Debug, Clone)]
enum LegType {
    TakeProfit,
    StopLoss,
}

impl BracketEngine {
    pub fn new(order_tx: Sender<Order>, cancel_tx: Sender<OrderId>) -> Self {
        Self {
            brackets: HashMap::new(),
            order_to_bracket: HashMap::new(),
            order_tx,
            cancel_tx,
        }
    }

    /// Submit a new bracket order. Entry must fill before legs are submitted.
    pub async fn submit(&mut self, mut bracket: BracketOrder) -> Result<BracketId, String> {
        let id = bracket.id.clone();
        // Submit entry order first
        self.order_tx
            .send(bracket.entry.clone())
            .await
            .map_err(|e| format!("Failed to submit entry: {}", e))?;

        bracket.state = BracketState::PendingEntry;
        self.brackets.insert(id.clone(), bracket);
        Ok(id)
    }

    /// Call when any order fill event arrives. Handles OCO logic automatically.
    pub async fn on_fill(&mut self, filled_order_id: &OrderId, fill_price: f64) {
        // Check if this is a bracket entry fill
        let bracket_id = self.find_bracket_by_entry(filled_order_id);
        if let Some(bid) = bracket_id {
            self.activate_bracket(&bid).await;
            return;
        }

        // Check if this is a bracket leg fill
        if let Some((bracket_id, leg_type)) = self.order_to_bracket.get(filled_order_id).cloned() {
            self.on_leg_filled(&bracket_id, leg_type, fill_price).await;
        }
    }

    /// Entry order filled — now submit both take-profit and stop-loss legs
    async fn activate_bracket(&mut self, bracket_id: &BracketId) {
        let bracket = match self.brackets.get_mut(bracket_id) {
            Some(b) => b,
            None => return,
        };

        bracket.state = BracketState::Active;

        // Build take-profit limit order
        let tp_id = format!("{}-TP", bracket_id);
        let tp_order = Order {
            id: tp_id.clone(),
            symbol: bracket.symbol.clone(),
            side: match bracket.entry.side {
                OrderSide::Buy => OrderSide::Sell,
                OrderSide::Sell => OrderSide::Buy,
            },
            order_type: OrderType::Limit,
            quantity: bracket.take_profit.quantity,
            limit_price: Some(bracket.take_profit.price),
            stop_price: None,
            status: OrderStatus::Pending,
        };

        // Build stop-loss stop order
        let sl_id = format!("{}-SL", bracket_id);
        let sl_order = Order {
            id: sl_id.clone(),
            symbol: bracket.symbol.clone(),
            side: match bracket.entry.side {
                OrderSide::Buy => OrderSide::Sell,
                OrderSide::Sell => OrderSide::Buy,
            },
            order_type: OrderType::StopMarket,
            quantity: bracket.stop_loss.quantity,
            limit_price: None,
            stop_price: Some(bracket.stop_loss.price),
            status: OrderStatus::Pending,
        };

        bracket.take_profit.order_id = Some(tp_id.clone());
        bracket.take_profit.status = LegStatus::Submitted;
        bracket.stop_loss.order_id = Some(sl_id.clone());
        bracket.stop_loss.status = LegStatus::Submitted;

        let bid = bracket_id.clone();
        self.order_to_bracket
            .insert(tp_id.clone(), (bid.clone(), LegType::TakeProfit));
        self.order_to_bracket
            .insert(sl_id.clone(), (bid, LegType::StopLoss));

        let _ = self.order_tx.send(tp_order).await;
        let _ = self.order_tx.send(sl_order).await;

        tracing::info!(bracket_id = %bracket_id, "Bracket activated — TP and SL legs submitted");
    }

    /// One leg filled — cancel the other (OCO logic)
    async fn on_leg_filled(
        &mut self,
        bracket_id: &BracketId,
        filled_leg: LegType,
        fill_price: f64,
    ) {
        let bracket = match self.brackets.get_mut(bracket_id) {
            Some(b) => b,
            None => return,
        };

        match filled_leg {
            LegType::TakeProfit => {
                bracket.state = BracketState::TakeProfitFilled;
                bracket.take_profit.status = LegStatus::Filled;
                // Cancel the stop-loss
                if let Some(sl_id) = &bracket.stop_loss.order_id {
                    let _ = self.cancel_tx.send(sl_id.clone()).await;
                    bracket.stop_loss.status = LegStatus::Cancelled;
                }
                tracing::info!(bracket_id = %bracket_id, fill_price, "Take-profit filled — stop-loss cancelled");
            }
            LegType::StopLoss => {
                bracket.state = BracketState::StopLossFilled;
                bracket.stop_loss.status = LegStatus::Filled;
                // Cancel the take-profit
                if let Some(tp_id) = &bracket.take_profit.order_id {
                    let _ = self.cancel_tx.send(tp_id.clone()).await;
                    bracket.take_profit.status = LegStatus::Cancelled;
                }
                tracing::info!(bracket_id = %bracket_id, fill_price, "Stop-loss triggered — take-profit cancelled");
            }
        }
    }

    fn find_bracket_by_entry(&self, order_id: &OrderId) -> Option<BracketId> {
        self.brackets
            .iter()
            .find(|(_, b)| &b.entry.id == order_id && b.state == BracketState::PendingEntry)
            .map(|(id, _)| id.clone())
    }

    pub fn get(&self, id: &BracketId) -> Option<&BracketOrder> {
        self.brackets.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn make_bracket(
        side: OrderSide,
        entry_price: f64,
        tp_price: f64,
        sl_price: f64,
    ) -> BracketOrder {
        BracketOrder {
            id: "bracket-001".to_string(),
            symbol: "NVDA".to_string(),
            entry: Order {
                id: "entry-001".to_string(),
                symbol: "NVDA".to_string(),
                side: side.clone(),
                order_type: OrderType::Limit,
                quantity: 10.0,
                limit_price: Some(entry_price),
                stop_price: None,
                status: OrderStatus::Pending,
            },
            take_profit: BracketLeg {
                price: tp_price,
                quantity: 10.0,
                order_id: None,
                status: LegStatus::Pending,
            },
            stop_loss: BracketLeg {
                price: sl_price,
                quantity: 10.0,
                order_id: None,
                status: LegStatus::Pending,
            },
            state: BracketState::PendingEntry,
        }
    }

    #[tokio::test]
    async fn test_bracket_submit_sets_pending_entry() {
        let (order_tx, mut order_rx) = mpsc::channel(16);
        let (cancel_tx, _cancel_rx) = mpsc::channel(16);
        let mut engine = BracketEngine::new(order_tx, cancel_tx);

        let bracket = make_bracket(OrderSide::Buy, 900.0, 970.0, 865.0);
        let id = engine.submit(bracket).await.unwrap();
        assert_eq!(engine.get(&id).unwrap().state, BracketState::PendingEntry);

        // Entry order should have been sent
        let entry = order_rx.recv().await.unwrap();
        assert_eq!(entry.id, "entry-001");
    }

    #[tokio::test]
    async fn test_bracket_oco_tp_cancels_sl() {
        let (order_tx, mut order_rx) = mpsc::channel(16);
        let (cancel_tx, mut cancel_rx) = mpsc::channel(16);
        let mut engine = BracketEngine::new(order_tx, cancel_tx);

        let bracket = make_bracket(OrderSide::Buy, 900.0, 970.0, 865.0);
        let id = engine.submit(bracket).await.unwrap();
        let _ = order_rx.recv().await; // consume entry

        // Simulate entry fill → activates bracket
        engine.on_fill(&"entry-001".to_string(), 900.0).await;
        assert_eq!(engine.get(&id).unwrap().state, BracketState::Active);

        // Consume TP and SL leg orders
        let _tp_order = order_rx.recv().await.unwrap();
        let _sl_order = order_rx.recv().await.unwrap();

        // Simulate TP fill → should cancel SL
        engine.on_fill(&"bracket-001-TP".to_string(), 970.0).await;
        let bracket = engine.get(&id).unwrap();
        assert_eq!(bracket.state, BracketState::TakeProfitFilled);
        assert_eq!(bracket.take_profit.status, LegStatus::Filled);
        assert_eq!(bracket.stop_loss.status, LegStatus::Cancelled);

        // SL cancel should have been sent
        let cancelled_id = cancel_rx.recv().await.unwrap();
        assert_eq!(cancelled_id, "bracket-001-SL");
    }

    #[tokio::test]
    async fn test_bracket_oco_sl_cancels_tp() {
        let (order_tx, mut order_rx) = mpsc::channel(16);
        let (cancel_tx, mut cancel_rx) = mpsc::channel(16);
        let mut engine = BracketEngine::new(order_tx, cancel_tx);

        let bracket = make_bracket(OrderSide::Buy, 900.0, 970.0, 865.0);
        let id = engine.submit(bracket).await.unwrap();
        let _ = order_rx.recv().await; // consume entry
        engine.on_fill(&"entry-001".to_string(), 900.0).await;
        let _ = order_rx.recv().await; // TP
        let _ = order_rx.recv().await; // SL

        // Simulate SL fill → should cancel TP
        engine.on_fill(&"bracket-001-SL".to_string(), 865.0).await;
        let bracket = engine.get(&id).unwrap();
        assert_eq!(bracket.state, BracketState::StopLossFilled);
        assert_eq!(bracket.stop_loss.status, LegStatus::Filled);
        assert_eq!(bracket.take_profit.status, LegStatus::Cancelled);

        let cancelled_id = cancel_rx.recv().await.unwrap();
        assert_eq!(cancelled_id, "bracket-001-TP");
    }

    #[tokio::test]
    async fn test_bracket_legs_opposite_side() {
        let (order_tx, mut order_rx) = mpsc::channel(16);
        let (cancel_tx, _cancel_rx) = mpsc::channel(16);
        let mut engine = BracketEngine::new(order_tx, cancel_tx);

        // Buy entry → sell legs
        let bracket = make_bracket(OrderSide::Buy, 900.0, 970.0, 865.0);
        let _id = engine.submit(bracket).await.unwrap();
        let _ = order_rx.recv().await; // entry
        engine.on_fill(&"entry-001".to_string(), 900.0).await;

        let tp_order = order_rx.recv().await.unwrap();
        let sl_order = order_rx.recv().await.unwrap();
        assert_eq!(
            tp_order.side,
            OrderSide::Sell,
            "TP of Buy entry should be Sell"
        );
        assert_eq!(
            sl_order.side,
            OrderSide::Sell,
            "SL of Buy entry should be Sell"
        );
    }

    /// For a LONG bracket: stop-loss must be below entry, take-profit above.
    /// This invariant violation is the canonical "max-loss instead of max-protection" bug.
    #[test]
    fn test_bracket_long_invariant_sl_below_entry_tp_above() {
        let entry_price = 900.0;
        let tp_price = 970.0;
        let sl_price = 865.0;
        let bracket = make_bracket(OrderSide::Buy, entry_price, tp_price, sl_price);
        assert!(
            bracket.stop_loss.price < entry_price,
            "LONG bracket: SL ({}) must be BELOW entry ({})",
            bracket.stop_loss.price,
            entry_price
        );
        assert!(
            bracket.take_profit.price > entry_price,
            "LONG bracket: TP ({}) must be ABOVE entry ({})",
            bracket.take_profit.price,
            entry_price
        );
    }

    /// For a SHORT bracket: stop-loss must be above entry, take-profit below.
    #[test]
    fn test_bracket_short_invariant_sl_above_entry_tp_below() {
        let entry_price = 900.0;
        let tp_price = 830.0; // profit target when shorting
        let sl_price = 935.0; // stop-loss above entry for short protection
        let bracket = make_bracket(OrderSide::Sell, entry_price, tp_price, sl_price);
        assert!(
            bracket.stop_loss.price > entry_price,
            "SHORT bracket: SL ({}) must be ABOVE entry ({})",
            bracket.stop_loss.price,
            entry_price
        );
        assert!(
            bracket.take_profit.price < entry_price,
            "SHORT bracket: TP ({}) must be BELOW entry ({})",
            bracket.take_profit.price,
            entry_price
        );
    }
}
