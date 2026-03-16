use anyhow::Result;
use common::events::BotEvent;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info, warn};

pub struct AlpacaWs {
    api_key: String,
    secret_key: String,
    symbols: Vec<String>,
}

impl AlpacaWs {
    pub fn new(api_key: String, secret_key: String, symbols: Vec<String>) -> Self {
        Self { api_key, secret_key, symbols }
    }

    pub async fn run(&self, tx: mpsc::UnboundedSender<BotEvent>) -> Result<()> {
        let url = "wss://stream.data.alpaca.markets/v2/iex";
        info!("Connecting to Alpaca WS at {}", url);

        let (ws_stream, _) = match connect_async(url).await {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to connect to Alpaca WS: {}", e);
                return Err(e.into());
            }
        };

        info!("Alpaca WS Connected.");
        let (mut write, mut read) = ws_stream.split();

        // 1. Authenticate
        let auth_msg = json!({
            "action": "auth",
            "key": self.api_key,
            "secret": self.secret_key
        });
        write.send(Message::Text(auth_msg.to_string())).await?;

        // 2. Wait for auth response (simplified)
        if let Some(Ok(Message::Text(msg))) = read.next().await {
            if !msg.contains(r#""T":"success""#) {
                warn!("Alpaca auth may have failed: {}", msg);
            } else {
                info!("Alpaca auth successful.");
            }
        }

        // 3. Subscribe to trades and quotes
        let sub_msg = json!({
            "action": "subscribe",
            "trades": self.symbols,
            "quotes": self.symbols,
            "bars": self.symbols
        });
        write.send(Message::Text(sub_msg.to_string())).await?;

        // 4. Listen for streams
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Quick and dirty Alpaca json parsing
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(arr) = value.as_array() {
                            for item in arr {
                                if item["T"] == "t" {
                                    // Trade event
                                    let symbol = item["S"].as_str().unwrap_or("").to_string();
                                    let price = item["p"].as_f64().unwrap_or(0.0);
                                    let ts = item["t"].as_str()
                                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                                        .map(|dt| dt.timestamp_millis())
                                        .unwrap_or(0);
                                    let volume = item["s"].as_f64();

                                    let event = BotEvent::MarketEvent {
                                        symbol: symbol.into(),
                                        price,
                                        timestamp: ts,
                                        event_type: "trade".into(),
                                        volume,
                                    };
                                    let _ = tx.send(event);
                                } else if item["T"] == "q" {
                                    // Quote event (L2 Order Book)
                                    let symbol = item["S"].as_str().unwrap_or("").to_string();
                                    let bid_price = item["bp"].as_f64().unwrap_or(0.0);
                                    let bid_size = item["bs"].as_u64().unwrap_or(0);
                                    let ask_price = item["ap"].as_f64().unwrap_or(0.0);
                                    let ask_size = item["as"].as_u64().unwrap_or(0);
                                    let ts = item["t"].as_str()
                                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                                        .map(|dt| dt.timestamp_millis())
                                        .unwrap_or(0);

                                    let event = BotEvent::QuoteEvent {
                                        symbol: symbol.into(),
                                        bid_price,
                                        bid_size,
                                        ask_price,
                                        ask_size,
                                        timestamp: ts,
                                    };
                                    let _ = tx.send(event);
                                } else if item["T"] == "b" {
                                    // Bar event (OHLCV)
                                    let symbol = item["S"].as_str().unwrap_or("").to_string();
                                    let price = item["c"].as_f64().unwrap_or(0.0); // close price
                                    let ts = item["t"].as_str()
                                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                                        .map(|dt| dt.timestamp_millis())
                                        .unwrap_or(0);
                                    let volume = item["v"].as_f64();

                                    let event = BotEvent::MarketEvent {
                                        symbol: symbol.into(),
                                        price,
                                        timestamp: ts,
                                        event_type: "bar".into(),
                                        volume,
                                    };
                                    let _ = tx.send(event);
                                }
                            }
                        }
                    }
                }
                Ok(Message::Ping(p)) => {
                    let _ = write.send(Message::Pong(p)).await;
                }
                Err(e) => {
                    error!("Alpaca WS Error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }
}
