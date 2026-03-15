use anyhow::Result;
use common::events::BotEvent;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_retry::{Retry, strategy::ExponentialBackoff};
use tracing::{info, error, warn};
use futures_util::StreamExt;
use std::time::Duration;

pub struct AlpacaReconnectClient {
    api_key: String,
    secret_key: String,
}

impl AlpacaReconnectClient {
    pub fn new(api_key: String, secret_key: String) -> Self {
        Self { api_key, secret_key }
    }

    pub async fn run(&self, tx: mpsc::UnboundedSender<BotEvent>) -> Result<()> {
        let strategy = ExponentialBackoff::from_millis(100)
            .factor(2)
            .max_delay(Duration::from_secs(10));

        loop {
            info!("Attempting Alpaca WebSocket connection...");
            let connect_result = Retry::spawn(strategy.clone(), || async {
                connect_async("wss://stream.data.alpaca.markets/v2/iex").await
            }).await;

            match connect_result {
                Ok((ws_stream, _)) => {
                    info!("Connected to Alpaca WebSocket");
                    let (_, mut read) = ws_stream.split();
                    
                    while let Some(msg) = read.next().await {
                        if let Ok(_m) = msg {
                            // Dummy connection keepalive 
                            let _ = tx.send(BotEvent::Feed("Alpaca heartbeat".into()));
                        } else {
                            warn!("Alpaca connection dropped. Initiating reconnect backoff.");
                            break;
                        }
                    }
                }
                Err(e) => {
                    error!("Alpaca connection completely failed: {:?}", e);
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
