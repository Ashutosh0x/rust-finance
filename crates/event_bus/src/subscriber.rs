use anyhow::Result;
use common::events::BotEvent;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tokio_retry::{strategy::ExponentialBackoff, Retry};
use tracing::{error, info, warn};

pub struct EventBusClient {
    addr: String,
}

impl EventBusClient {
    pub fn new(addr: String) -> Self {
        Self { addr }
    }

    pub async fn subscribe(&self, tx: mpsc::Sender<BotEvent>) -> Result<()> {
        let strategy = ExponentialBackoff::from_millis(500)
            .factor(2)
            .max_delay(Duration::from_secs(30));

        loop {
            let addr_clone = self.addr.clone();
            let connect_result = Retry::spawn(strategy.clone(), || async {
                info!("Attempting to connect to EventBus at {}...", addr_clone);
                TcpStream::connect(&addr_clone).await
            })
            .await;

            match connect_result {
                Ok(stream) => {
                    info!("Successfully connected to EventBus!");
                    let (reader, _) = tokio::io::split(stream);
                    let mut lines = BufReader::new(reader).lines();

                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Ok(event) = serde_json::from_str::<BotEvent>(&line) {
                            if tx.send(event).await.is_err() {
                                error!("Receiver dropped, shutting down subscriber");
                                return Ok(());
                            }
                        }
                    }
                    warn!("EventBus connection lost. Reconnecting...");
                }
                Err(e) => {
                    error!("Fatal EventBus connection error: {:?}", e);
                    sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }
}
