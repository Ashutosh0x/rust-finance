use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize)]
struct MarketSubscription {
    assets_ids: Vec<String>,
    #[serde(rename = "type")]
    channel_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_feature_enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BookEvent {
    pub event_type: String,
    pub asset_id: String,
    pub market: Option<String>,
    pub bids: Option<Vec<BookLevel>>,
    pub asks: Option<Vec<BookLevel>>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BookLevel {
    pub price: String,
    pub size: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PriceChangeEvent {
    pub event_type: String,
    pub market: Option<String>,
    pub price_changes: Option<Vec<PriceChange>>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PriceChange {
    pub asset_id: String,
    pub price: String,
    pub size: Option<String>,
    pub side: Option<String>,
    pub best_bid: Option<String>,
    pub best_ask: Option<String>,
}

/// Unified event enum for the event_bus
#[derive(Debug, Clone)]
pub enum PolymarketEvent {
    BookSnapshot(BookEvent),
    PriceUpdate(PriceChangeEvent),
    LastTradePrice {
        asset_id: String,
        price: String,
    },
    TickSizeChange {
        asset_id: String,
        tick_size: String,
    },
    MarketResolved {
        market: String,
        winning_asset_id: String,
        winning_outcome: String,
    },
    Disconnected,
}

pub struct PolymarketWs {
    ws_url: String,
    event_tx: broadcast::Sender<PolymarketEvent>,
}

impl PolymarketWs {
    pub fn new(ws_url: &str, event_tx: broadcast::Sender<PolymarketEvent>) -> Self {
        Self {
            ws_url: ws_url.to_string(),
            event_tx,
        }
    }

    /// Connect and subscribe to market data for given asset IDs
    pub async fn connect_market_channel(&self, asset_ids: Vec<String>) -> anyhow::Result<()> {
        let url = format!("{}", self.ws_url);

        loop {
            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    info!("Connected to Polymarket WebSocket {}", url);
                    let (mut write, mut read) = ws_stream.split();

                    // Send subscription
                    let sub = MarketSubscription {
                        assets_ids: asset_ids.clone(),
                        channel_type: "market".to_string(),
                        custom_feature_enabled: Some(true),
                    };
                    write
                        .send(Message::Text(serde_json::to_string(&sub)?))
                        .await?;

                    // Heartbeat task (ping every 10s)
                    // Note: tungstenite handles ping/pong automatically if correctly configured,
                    // but we can also manually send ping frames.
                    let _ping_msg = Message::Ping(vec![]);

                    // Simple manual heartbeat (since tungstenite requires passing a channel or clone, etc.)
                    // Usually you'd spawn a task with a clone of the sink, or interleave the stream/sink.
                    // For simplicity, we just rely on tokio-tungstenite's built-in ping and regular activity.

                    // Read loop
                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                self.handle_message(&text);
                            }
                            Ok(Message::Ping(_data)) => {
                                // tungstenite auto-replies to ping, but if we need manual:
                                // let _ = write.send(Message::Pong(data)).await;
                            }
                            Ok(Message::Close(_)) => {
                                warn!("WebSocket closed by server");
                                break;
                            }
                            Err(e) => {
                                error!("WebSocket error: {}", e);
                                break;
                            }
                            _ => {}
                        }
                    }

                    let _ = self.event_tx.send(PolymarketEvent::Disconnected);
                }
                Err(e) => {
                    error!("Failed to connect: {}. Retrying in 5s...", e);
                }
            }

            // Exponential backoff reconnect
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    fn handle_message(&self, text: &str) {
        // Parse the event_type field first
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
            match val.get("event_type").and_then(|v| v.as_str()) {
                Some("book") => {
                    if let Ok(evt) = serde_json::from_str::<BookEvent>(text) {
                        let _ = self.event_tx.send(PolymarketEvent::BookSnapshot(evt));
                    }
                }
                Some("price_change") => {
                    if let Ok(evt) = serde_json::from_str::<PriceChangeEvent>(text) {
                        let _ = self.event_tx.send(PolymarketEvent::PriceUpdate(evt));
                    }
                }
                Some("last_trade_price") => {
                    let asset_id = val["asset_id"].as_str().unwrap_or("").to_string();
                    let price = val["price"].as_str().unwrap_or("0").to_string();
                    let _ = self
                        .event_tx
                        .send(PolymarketEvent::LastTradePrice { asset_id, price });
                }
                Some("tick_size_change") => {
                    let asset_id = val["asset_id"].as_str().unwrap_or("").to_string();
                    let tick_size = val["tick_size"].as_str().unwrap_or("").to_string();
                    let _ = self.event_tx.send(PolymarketEvent::TickSizeChange {
                        asset_id,
                        tick_size,
                    });
                }
                Some("market_resolved") => {
                    let market = val["market"].as_str().unwrap_or("").to_string();
                    let winning_asset_id =
                        val["winning_asset_id"].as_str().unwrap_or("").to_string();
                    let winning_outcome = val["winning_outcome"].as_str().unwrap_or("").to_string();
                    let _ = self.event_tx.send(PolymarketEvent::MarketResolved {
                        market,
                        winning_asset_id,
                        winning_outcome,
                    });
                }
                _ => {
                    // Unknown or pong
                }
            }
        }
    }
}
