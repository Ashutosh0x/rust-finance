use anyhow::Result;
use common::events::BotEvent;
use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{error, info, warn};

/// Alpaca WebSocket data feed type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlpacaFeed {
    /// IEX — ~2.5% US market volume (free, no subscription required)
    Iex,
    /// SIP — 100% US market volume (NYSE CTA + NASDAQ UTP consolidated)
    Sip,
    /// Delayed SIP — 15-minute delayed SIP feed
    DelayedSip,
    /// BOATS — Blue Ocean ATS after-hours trading
    Boats,
    /// Overnight — Alpaca derived overnight feed (15 min delayed)
    Overnight,
}

impl AlpacaFeed {
    fn url_path(&self) -> &str {
        match self {
            AlpacaFeed::Iex => "v2/iex",
            AlpacaFeed::Sip => "v2/sip",
            AlpacaFeed::DelayedSip => "v2/delayed_sip",
            AlpacaFeed::Boats => "v1beta1/boats",
            AlpacaFeed::Overnight => "v1beta1/overnight",
        }
    }

    fn name(&self) -> &str {
        match self {
            AlpacaFeed::Iex => "IEX",
            AlpacaFeed::Sip => "SIP",
            AlpacaFeed::DelayedSip => "Delayed SIP",
            AlpacaFeed::Boats => "BOATS",
            AlpacaFeed::Overnight => "Overnight",
        }
    }
}

impl Default for AlpacaFeed {
    fn default() -> Self { AlpacaFeed::Iex }
}

impl std::str::FromStr for AlpacaFeed {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "iex" => Ok(AlpacaFeed::Iex),
            "sip" => Ok(AlpacaFeed::Sip),
            "delayed_sip" | "delayed-sip" => Ok(AlpacaFeed::DelayedSip),
            "boats" => Ok(AlpacaFeed::Boats),
            "overnight" => Ok(AlpacaFeed::Overnight),
            _ => Err(anyhow::anyhow!("Unknown Alpaca feed: '{}'", s)),
        }
    }
}

pub struct AlpacaWs {
    api_key: String,
    secret_key: String,
    symbols: Vec<String>,
    feed: AlpacaFeed,
    sandbox: bool,
}

impl AlpacaWs {
    pub fn new(api_key: String, secret_key: String, symbols: Vec<String>) -> Self {
        Self { api_key, secret_key, symbols, feed: AlpacaFeed::Iex, sandbox: false }
    }

    pub fn with_feed(mut self, feed: AlpacaFeed) -> Self { self.feed = feed; self }
    pub fn with_sandbox(mut self, sandbox: bool) -> Self { self.sandbox = sandbox; self }

    fn ws_url(&self) -> String {
        let host = if self.sandbox {
            "stream.data.sandbox.alpaca.markets"
        } else {
            "stream.data.alpaca.markets"
        };
        format!("wss://{}/{}", host, self.feed.url_path())
    }

    pub async fn run(&self, tx: mpsc::UnboundedSender<BotEvent>) -> Result<()> {
        let url = self.ws_url();
        info!("Connecting to Alpaca WS ({} feed) at {}", self.feed.name(), url);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;
        info!("Alpaca WS Connected (feed: {}).", self.feed.name());
        let (mut write, mut read) = ws_stream.split();

        // 1. Authenticate
        let auth_msg = json!({
            "action": "auth",
            "key": self.api_key,
            "secret": self.secret_key
        });
        write.send(Message::Text(auth_msg.to_string())).await?;

        // 2. Wait for auth response
        if let Some(Ok(Message::Text(msg))) = read.next().await {
            if msg.contains(r#""T":"success""#) {
                info!("Alpaca auth successful.");
            } else {
                warn!("Alpaca auth response: {}", msg);
            }
        }

        // 3. Subscribe
        let mut sub = json!({
            "action": "subscribe",
            "trades": self.symbols,
            "quotes": self.symbols,
            "bars": self.symbols
        });

        if self.feed == AlpacaFeed::Sip {
            sub["corrections"] = json!(self.symbols);
            sub["cancelErrors"] = json!(self.symbols);
            sub["lulds"] = json!(self.symbols);
            sub["statuses"] = json!(self.symbols);
        }

        write.send(Message::Text(sub.to_string())).await?;
        info!("Subscribed to {} symbols on {} feed", self.symbols.len(), self.feed.name());

        // 4. Process messages
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(arr) = value.as_array() {
                            for item in arr {
                                let msg_type = item["T"].as_str().unwrap_or("");
                                match msg_type {
                                    "t" => {
                                        let symbol = item["S"].as_str().unwrap_or("").to_string();
                                        let price = item["p"].as_f64().unwrap_or(0.0);
                                        let volume = item["s"].as_f64();
                                        let _ = tx.send(BotEvent::MarketEvent {
                                            symbol, price, volume,
                                            event_type: "trade".into(),
                                        });
                                    }
                                    "q" => {
                                        let symbol = item["S"].as_str().unwrap_or("").to_string();
                                        let _ = tx.send(BotEvent::QuoteEvent {
                                            symbol,
                                            bid_price: item["bp"].as_f64().unwrap_or(0.0),
                                            bid_size: item["bs"].as_u64().unwrap_or(0),
                                            ask_price: item["ap"].as_f64().unwrap_or(0.0),
                                            ask_size: item["as"].as_u64().unwrap_or(0),
                                        });
                                    }
                                    "b" => {
                                        let symbol = item["S"].as_str().unwrap_or("").to_string();
                                        let price = item["c"].as_f64().unwrap_or(0.0);
                                        let volume = item["v"].as_f64();
                                        let _ = tx.send(BotEvent::MarketEvent {
                                            symbol, price, volume,
                                            event_type: "bar".into(),
                                        });
                                    }
                                    "c" => {
                                        let symbol = item["S"].as_str().unwrap_or("").to_string();
                                        let price = item["p"].as_f64().unwrap_or(0.0);
                                        info!("[SIP] Trade correction: {} @ {}", symbol, price);
                                        let _ = tx.send(BotEvent::MarketEvent {
                                            symbol, price,
                                            event_type: "correction".into(),
                                            volume: item["s"].as_f64(),
                                        });
                                    }
                                    "x" => {
                                        let symbol = item["S"].as_str().unwrap_or("").to_string();
                                        warn!("[SIP] Trade cancel: {} id={}", symbol, item["i"]);
                                        let _ = tx.send(BotEvent::MarketEvent {
                                            symbol, price: 0.0,
                                            event_type: "cancel".into(),
                                            volume: None,
                                        });
                                    }
                                    "l" => {
                                        let symbol = item["S"].as_str().unwrap_or("").to_string();
                                        let upper = item["u"].as_f64().unwrap_or(0.0);
                                        let lower = item["d"].as_f64().unwrap_or(0.0);
                                        info!("[SIP] LULD {}: [{:.2}, {:.2}]", symbol, lower, upper);
                                        let _ = tx.send(BotEvent::MarketEvent {
                                            symbol, price: upper,
                                            event_type: "luld".into(),
                                            volume: Some(lower),
                                        });
                                    }
                                    "s" => {
                                        let symbol = item["S"].as_str().unwrap_or("").to_string();
                                        let sc = item["sc"].as_str().unwrap_or("");
                                        let sm = item["sm"].as_str().unwrap_or("");
                                        info!("[SIP] Trading status {}: {}", symbol, sm);
                                        let _ = tx.send(BotEvent::MarketEvent {
                                            symbol, price: 0.0,
                                            event_type: format!("status:{}", sc),
                                            volume: None,
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Ok(Message::Ping(p)) => { let _ = write.send(Message::Pong(p)).await; }
                Err(e) => { error!("Alpaca WS Error: {}", e); break; }
                _ => {}
            }
        }
        Ok(())
    }
}
