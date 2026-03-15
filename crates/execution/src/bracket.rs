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
    pub quantity: u64,
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
enum LegType { TakeProfit, StopLoss }

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
        self.order_tx.send(bracket.entry.clone()).await
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
            side: match bracket.entry.side { OrderSide::Buy => OrderSide::Sell, OrderSide::Sell => OrderSide::Buy },
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
            side: match bracket.entry.side { OrderSide::Buy => OrderSide::Sell, OrderSide::Sell => OrderSide::Buy },
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
        self.order_to_bracket.insert(tp_id.clone(), (bid.clone(), LegType::TakeProfit));
        self.order_to_bracket.insert(sl_id.clone(), (bid, LegType::StopLoss));

        let _ = self.order_tx.send(tp_order).await;
        let _ = self.order_tx.send(sl_order).await;

        tracing::info!(bracket_id = %bracket_id, "Bracket activated — TP and SL legs submitted");
    }

    /// One leg filled — cancel the other (OCO logic)
    async fn on_leg_filled(&mut self, bracket_id: &BracketId, filled_leg: LegType, fill_price: f64) {
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
        self.brackets.iter()
            .find(|(_, b)| &b.entry.id == order_id && b.state == BracketState::PendingEntry)
            .map(|(id, _)| id.clone())
    }

    pub fn get(&self, id: &BracketId) -> Option<&BracketOrder> {
        self.brackets.get(id)
    }
}
