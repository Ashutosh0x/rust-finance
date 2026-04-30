use anyhow::Result;
use common::events::BotEvent;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tokio_retry::{strategy::ExponentialBackoff, Retry};
use tracing::{error, info, warn};

const MAX_FRAME_LEN: usize = 1024 * 1024;

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
                    let (mut reader, _) = tokio::io::split(stream);
                    let mut len_buf = [0u8; 4];

                    loop {
                        if reader.read_exact(&mut len_buf).await.is_err() {
                            break;
                        }

                        let len = u32::from_le_bytes(len_buf) as usize;
                        if len == 0 || len > MAX_FRAME_LEN {
                            warn!(len, "Invalid EventBus frame length; reconnecting");
                            break;
                        }

                        let mut buf = vec![0u8; len];
                        if reader.read_exact(&mut buf).await.is_err() {
                            break;
                        }

                        match postcard::from_bytes::<BotEvent>(&buf) {
                            Ok(event) => {
                                if tx.send(event).await.is_err() {
                                    error!("Receiver dropped, shutting down subscriber");
                                    return Ok(());
                                }
                            }
                            Err(e) => warn!(error = %e, "Malformed EventBus frame"),
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
