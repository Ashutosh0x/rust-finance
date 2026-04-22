use anyhow::Result;
use common::events::BotEvent;
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{error, info, warn};

#[derive(serde::Deserialize)]
struct FinnhubTradeMsg {
    r#type: String,
    data: Option<Vec<FhTrade>>,
}

#[derive(serde::Deserialize)]
struct FhTrade {
    s: String,
    p: f64,
    v: Option<f64>,
    #[allow(dead_code)]
    t: i64,
}

pub struct FinnhubWs {
    api_key: String,
    symbols: Vec<String>,
}

impl FinnhubWs {
    pub fn new(api_key: String, symbols: Vec<String>) -> Self {
        Self { api_key, symbols }
    }

    pub async fn run(&self, tx: mpsc::UnboundedSender<BotEvent>) -> Result<()> {
        loop {
            match self.connect_and_stream(tx.clone()).await {
                Ok(()) => {
                    warn!("Finnhub WS stream ended, reconnecting in 5s...");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                Err(e) => {
                    error!("Finnhub error: {:?}. Reconnecting in 10s...", e);
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
            }
        }
    }

    async fn connect_and_stream(&self, tx: mpsc::UnboundedSender<BotEvent>) -> Result<()> {
        let url = format!("wss://ws.finnhub.io?token={}", self.api_key);
        info!("Connecting to Finnhub WebSocket...");

        let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;
        info!("Finnhub connected.");

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to symbols
        for symbol in &self.symbols {
            let msg = format!(r#"{{"type":"subscribe","symbol":"{}"}}"#, symbol);
            write.send(Message::Text(msg)).await?;
        }

        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(parsed) = serde_json::from_str::<FinnhubTradeMsg>(&text) {
                        if parsed.r#type == "trade" {
                            for t in parsed.data.unwrap_or_default() {
                                let _ = tx.send(BotEvent::MarketEvent {
                                    symbol: t.s,
                                    price: t.p,
                                    event_type: "trade".into(),
                                    volume: t.v,
                                });
                            }
                        }
                    }
                }
                Ok(Message::Ping(p)) => {
                    let _ = write.send(Message::Pong(p)).await;
                }
                Err(e) => {
                    error!("Finnhub WS error: {:?}", e);
                    return Err(e.into());
                }
                _ => {}
            }
        }

        Err(anyhow::anyhow!("WebSocket stream unexpectedly ended"))
    }
}
