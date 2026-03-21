//! Polymarket CLOB WebSocket integration.
//!
//! Subscribes to the `market` channel for real-time order book and trade
//! updates on prediction market conditions.
//!
//! Polymarket constraints:
//! - Max 500 subscriptions per WebSocket connection (200 default)
//! - Rate limit: 100 requests/minute on REST, Cloudflare throttling
//! - HTTP 429 on rate limit exceeded
//! - Uses condition_id (not slug) for CLOB endpoints
//!
//! Ref: https://docs.polymarket.com/developers/CLOB/websocket/wss-overview

use crate::source::{DataType, IngestionError, MarketDataSource, MarketStream, Subscription};
use async_trait::async_trait;
use common::events::*;
use common::time::{SequenceGenerator, UnixNanos};
use compact_str::CompactString;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

const POLYMARKET_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

pub struct PolymarketSource {
    seq_gen: Arc<SequenceGenerator>,
    /// Max subscriptions per connection (Polymarket limit: 500).
    max_subs_per_conn: usize,
}

impl PolymarketSource {
    pub fn new(seq_gen: Arc<SequenceGenerator>) -> Self {
        Self {
            seq_gen,
            max_subs_per_conn: 200, // Conservative default
        }
    }

    pub fn with_max_subs(mut self, max: usize) -> Self {
        self.max_subs_per_conn = max.min(500);
        self
    }
}

#[async_trait]
impl MarketDataSource for PolymarketSource {
    fn name(&self) -> &str {
        "Polymarket"
    }

    fn supported_data_types(&self) -> &[DataType] {
        &[DataType::Trades, DataType::Quotes]
    }

    async fn connect(&self, subscription: &Subscription) -> Result<MarketStream, IngestionError> {
        let (mut ws, _) = connect_async(POLYMARKET_WS_URL)
            .await
            .map_err(|e| IngestionError::ConnectionFailed(e.to_string()))?;

        info!(provider = "Polymarket", "WebSocket connected");

        // Polymarket market channel subscription requires condition_ids
        // (the hex token IDs, not human-readable slugs).
        //
        // Subscribe to each market's condition_id as an "asset".
        // The subscribe message format per the CLOB WSS docs:
        // {
        //   "type": "market",
        //   "assets_ids": ["0x1234...", "0x5678..."]
        // }
        let batch_size = self.max_subs_per_conn.min(subscription.symbols.len());
        let asset_ids: Vec<&str> = subscription
            .symbols
            .iter()
            .take(batch_size)
            .map(|s| s.as_str())
            .collect();

        let subscribe_msg = serde_json::json!({
            "type": "market",
            "assets_ids": asset_ids,
        });

        ws.send(Message::Text(subscribe_msg.to_string()))
            .await
            .map_err(|e| IngestionError::ConnectionFailed(e.to_string()))?;

        debug!(
            markets = asset_ids.len(),
            "Polymarket subscription sent"
        );

        let seq_gen = Arc::clone(&self.seq_gen);

        let stream = ws.filter_map(move |msg_result| {
            let seq_gen = Arc::clone(&seq_gen);
            async move {
                match msg_result {
                    Ok(Message::Text(text)) => parse_polymarket_message(&text, &seq_gen),
                    Ok(Message::Close(_)) => {
                        warn!("Polymarket WebSocket closed");
                        Some(Err(IngestionError::StreamClosed))
                    }
                    Err(e) => {
                        error!(error = %e, "Polymarket WebSocket error");
                        Some(Err(IngestionError::ConnectionFailed(e.to_string())))
                    }
                    _ => None,
                }
            }
        });

        Ok(Box::pin(stream))
    }

    async fn is_healthy(&self) -> bool {
        true // Public endpoint, no auth needed for market data
    }
}

/// Parse Polymarket WebSocket messages.
///
/// The market channel pushes price changes and trade events.
/// Price events contain the current best bid/ask for YES and NO outcomes.
fn parse_polymarket_message(
    text: &str,
    seq_gen: &SequenceGenerator,
) -> Option<Result<Envelope<MarketEvent>, IngestionError>> {
    let json: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return Some(Err(IngestionError::Deserialize(e.to_string()))),
    };

    let ts_init = UnixNanos::now();

    // Polymarket pushes different event types on the market channel.
    // We handle the main ones:

    // Check for price_change events (best bid/ask updates)
    if let Some(event_type) = json.get("event_type").and_then(|v| v.as_str()) {
        match event_type {
            "price_change" => {
                return parse_price_change(&json, ts_init, seq_gen);
            }
            "trade" | "last_trade_price" => {
                return parse_poly_trade(&json, ts_init, seq_gen);
            }
            _ => {
                debug!(event_type = event_type, "Unhandled Polymarket event");
                return None;
            }
        }
    }

    // Some messages are arrays of changes
    if let Some(arr) = json.as_array() {
        for item in arr {
            if let Some(result) = parse_polymarket_message_inner(item, ts_init, seq_gen) {
                return Some(result);
            }
        }
    }

    None
}

fn parse_polymarket_message_inner(
    json: &serde_json::Value,
    ts_init: UnixNanos,
    seq_gen: &SequenceGenerator,
) -> Option<Result<Envelope<MarketEvent>, IngestionError>> {
    let event_type = json.get("event_type")?.as_str()?;

    match event_type {
        "price_change" => parse_price_change(json, ts_init, seq_gen),
        "trade" | "last_trade_price" => parse_poly_trade(json, ts_init, seq_gen),
        _ => None,
    }
}

fn parse_price_change(
    json: &serde_json::Value,
    ts_init: UnixNanos,
    seq_gen: &SequenceGenerator,
) -> Option<Result<Envelope<MarketEvent>, IngestionError>> {
    // Extract the condition/token identifier
    let asset_id = json
        .get("asset_id")
        .or_else(|| json.get("condition_id"))
        .and_then(|v| v.as_str())?;

    // Truncate hex ID for display: "0x1234...5678" -> "PM:1234..5678"
    let symbol = if asset_id.len() > 16 {
        CompactString::new(format!(
            "PM:{}..{}",
            &asset_id[2..6.min(asset_id.len())],
            &asset_id[asset_id.len().saturating_sub(4)..]
        ))
    } else {
        CompactString::new(asset_id)
    };

    // Polymarket prices are in cents (0.00 to 1.00)
    let price = json
        .get("price")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or(v.as_f64()))
        .unwrap_or(0.0);

    // Best bid/ask if available
    let bid = json
        .get("best_bid")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or(v.as_f64()))
        .unwrap_or(price - 0.01);

    let ask = json
        .get("best_ask")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or(v.as_f64()))
        .unwrap_or(price + 0.01);

    let event = MarketEvent::Quote(QuoteEvent {
        symbol,
        bid,
        bid_size: 0.0, // Polymarket doesn't always provide size in price_change
        ask,
        ask_size: 0.0,
    });

    let ts_event = json
        .get("timestamp")
        .and_then(|v| v.as_u64())
        .map(UnixNanos::from_millis)
        .unwrap_or(ts_init);

    Some(Ok(Envelope {
        ts_event,
        ts_init,
        sequence_id: seq_gen.next_id(),
        payload: event,
    }))
}

fn parse_poly_trade(
    json: &serde_json::Value,
    ts_init: UnixNanos,
    seq_gen: &SequenceGenerator,
) -> Option<Result<Envelope<MarketEvent>, IngestionError>> {
    let asset_id = json
        .get("asset_id")
        .or_else(|| json.get("condition_id"))
        .and_then(|v| v.as_str())?;

    let symbol = if asset_id.len() > 16 {
        CompactString::new(format!(
            "PM:{}..{}",
            &asset_id[2..6.min(asset_id.len())],
            &asset_id[asset_id.len().saturating_sub(4)..]
        ))
    } else {
        CompactString::new(asset_id)
    };

    let price = json
        .get("price")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or(v.as_f64()))
        .unwrap_or(0.0);

    let size = json
        .get("size")
        .or_else(|| json.get("amount"))
        .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or(v.as_f64()))
        .unwrap_or(0.0);

    let side_str = json.get("side").and_then(|v| v.as_str()).unwrap_or("");
    let side = match side_str.to_uppercase().as_str() {
        "BUY" => TradeSide::Buy,
        "SELL" => TradeSide::Sell,
        _ => TradeSide::Unknown,
    };

    let event = MarketEvent::Trade(TradeEvent {
        symbol,
        price,
        quantity: size,
        side,
    });

    let ts_event = json
        .get("timestamp")
        .and_then(|v| v.as_u64())
        .map(UnixNanos::from_millis)
        .unwrap_or(ts_init);

    Some(Ok(Envelope {
        ts_event,
        ts_init,
        sequence_id: seq_gen.next_id(),
        payload: event,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_price_change_event() {
        let seq_gen = SequenceGenerator::new();
        let msg = r#"{
            "event_type": "price_change",
            "asset_id": "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "price": "0.65",
            "best_bid": "0.64",
            "best_ask": "0.66",
            "timestamp": 1700000000000
        }"#;

        let result = parse_polymarket_message(msg, &seq_gen);
        let envelope = result.unwrap().unwrap();

        match &envelope.payload {
            MarketEvent::Quote(q) => {
                assert!(q.symbol.starts_with("PM:"));
                assert!((q.bid - 0.64).abs() < f64::EPSILON);
                assert!((q.ask - 0.66).abs() < f64::EPSILON);
            }
            _ => panic!("Expected Quote"),
        }
    }

    #[test]
    fn parse_trade_event() {
        let seq_gen = SequenceGenerator::new();
        let msg = r#"{
            "event_type": "trade",
            "asset_id": "0x1234abcd",
            "price": "0.72",
            "size": "100.0",
            "side": "BUY",
            "timestamp": 1700000000000
        }"#;

        let result = parse_polymarket_message(msg, &seq_gen);
        let envelope = result.unwrap().unwrap();

        match &envelope.payload {
            MarketEvent::Trade(t) => {
                assert!((t.price - 0.72).abs() < f64::EPSILON);
                assert!((t.quantity - 100.0).abs() < f64::EPSILON);
                assert_eq!(t.side, TradeSide::Buy);
            }
            _ => panic!("Expected Trade"),
        }
    }
}
