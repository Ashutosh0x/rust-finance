use anyhow::Result;
use common::events::BotEvent;
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tracing::{error, info, warn};

pub struct AlpacaReconnectClient {
    #[allow(dead_code)]
    api_key: String,
    #[allow(dead_code)]
    secret_key: String,
}

impl AlpacaReconnectClient {
    pub fn new(api_key: String, secret_key: String) -> Self {
        Self {
            api_key,
            secret_key,
        }
    }

    pub async fn run(&self, tx: mpsc::UnboundedSender<BotEvent>) -> Result<()> {
        let mut backoff = Duration::from_millis(100);
        let max_backoff = Duration::from_secs(10);

        loop {
            info!("Attempting Alpaca WebSocket connection...");
            match connect_async("wss://stream.data.alpaca.markets/v2/iex").await {
                Ok((ws_stream, _)) => {
                    info!("Connected to Alpaca WebSocket");
                    backoff = Duration::from_millis(100); // reset on success

                    let (_write, mut read) = ws_stream.split();

                    while let Some(msg) = read.next().await {
                        if let Ok(_m) = msg {
                            let _ = tx.send(BotEvent::Feed("Alpaca heartbeat".into()));
                        } else {
                            warn!("Alpaca connection dropped. Initiating reconnect backoff.");
                            break;
                        }
                    }
                }
                Err(e) => {
                    error!("Alpaca connection failed: {:?}", e);
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(max_backoff);
        }
    }
}
