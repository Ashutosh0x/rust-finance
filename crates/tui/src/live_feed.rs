//! Direct Binance WebSocket feed for the TUI.
//!
//! Connects to Binance combined streams without requiring the daemon.
//! Feeds live kline (candlestick), trade, and bookTicker data
//! directly into the TUI via mpsc channels.

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Events sent from the WebSocket feed to the TUI render loop.
#[derive(Debug, Clone)]
pub enum LiveFeedEvent {
    /// A kline/candlestick bar update (may be in-progress or closed).
    Kline {
        symbol: String,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
        is_closed: bool,
    },
    /// A trade tick.
    Trade {
        symbol: String,
        price: f64,
        quantity: f64,
    },
    /// Best bid/ask update.
    BookTicker {
        symbol: String,
        bid_price: f64,
        bid_size: f64,
        ask_price: f64,
        ask_size: f64,
    },
    /// Connection status change.
    Status(String),
}

/// Spawn a Binance WebSocket feed task.
///
/// Connects to `wss://stream.binance.com:9443/stream` with:
/// - `btcusdt@kline_1m` — 1-minute candlestick bars
/// - `btcusdt@trade`     — individual trades
/// - `btcusdt@bookTicker` — best bid/ask
/// - `ethusdt@trade`, `solusdt@trade`, `bnbusdt@trade` — watchlist trades
///
/// Automatically reconnects with exponential backoff on disconnection.
pub fn spawn_binance_feed(tx: mpsc::Sender<LiveFeedEvent>) {
    tokio::spawn(async move {
        let mut backoff_secs = 1u64;

        loop {
            let url = "wss://stream.binance.com:9443/stream?streams=\
                btcusdt@kline_1m/\
                btcusdt@trade/\
                btcusdt@bookTicker/\
                ethusdt@trade/\
                solusdt@trade/\
                bnbusdt@trade";

            let _ = tx.send(LiveFeedEvent::Status(
                "Connecting to Binance WebSocket...".to_string(),
            )).await;

            match connect_async(url).await {
                Ok((ws, _)) => {
                    backoff_secs = 1; // Reset on success
                    let _ = tx.send(LiveFeedEvent::Status(
                        "Connected to Binance (live feed)".to_string(),
                    )).await;

                    let (_, mut read) = ws.split();

                    while let Some(msg_result) = read.next().await {
                        match msg_result {
                            Ok(Message::Text(text)) => {
                                if let Some(event) = parse_combined_message(&text) {
                                    if tx.send(event).await.is_err() {
                                        return; // Receiver dropped
                                    }
                                }
                            }
                            Ok(Message::Ping(_)) => {
                                // tungstenite auto-responds with Pong
                            }
                            Ok(Message::Close(_)) => {
                                let _ = tx.send(LiveFeedEvent::Status(
                                    "Binance WebSocket closed. Reconnecting...".to_string(),
                                )).await;
                                break;
                            }
                            Err(e) => {
                                let _ = tx.send(LiveFeedEvent::Status(
                                    format!("WebSocket error: {}. Reconnecting...", e),
                                )).await;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(LiveFeedEvent::Status(
                        format!("Connection failed: {}. Retry in {}s...", e, backoff_secs),
                    )).await;
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(30);
        }
    });
}

/// Parse a Binance combined stream JSON message into a LiveFeedEvent.
fn parse_combined_message(text: &str) -> Option<LiveFeedEvent> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let data = json.get("data")?;
    let event_type = data.get("e")?.as_str()?;

    match event_type {
        "kline" => {
            let k = data.get("k")?;
            Some(LiveFeedEvent::Kline {
                symbol: k.get("s")?.as_str()?.to_string(),
                open: k.get("o")?.as_str()?.parse().ok()?,
                high: k.get("h")?.as_str()?.parse().ok()?,
                low: k.get("l")?.as_str()?.parse().ok()?,
                close: k.get("c")?.as_str()?.parse().ok()?,
                volume: k.get("v")?.as_str()?.parse().ok()?,
                is_closed: k.get("x")?.as_bool()?,
            })
        }
        "trade" => {
            Some(LiveFeedEvent::Trade {
                symbol: data.get("s")?.as_str()?.to_string(),
                price: data.get("p")?.as_str()?.parse().ok()?,
                quantity: data.get("q")?.as_str()?.parse().ok()?,
            })
        }
        "bookTicker" => {
            Some(LiveFeedEvent::BookTicker {
                symbol: data.get("s")?.as_str()?.to_string(),
                bid_price: data.get("b")?.as_str()?.parse().ok()?,
                bid_size: data.get("B")?.as_str()?.parse().ok()?,
                ask_price: data.get("a")?.as_str()?.parse().ok()?,
                ask_size: data.get("A")?.as_str()?.parse().ok()?,
            })
        }
        _ => None,
    }
}
