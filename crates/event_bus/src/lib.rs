#![forbid(unsafe_code)]
use common::events::{BotEvent, ControlCommand};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{broadcast, mpsc},
};
use tracing::{info, warn};

pub mod health;
pub mod subscriber;

const MAX_FRAME_LEN: usize = 1024 * 1024;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<BotEvent>,
}

impl EventBus {
    pub async fn start(control_tx: mpsc::Sender<ControlCommand>) -> anyhow::Result<Self> {
        let (tx, _) = broadcast::channel::<BotEvent>(16_384);
        let tx_clone = tx.clone();
        let allow_control = std::env::var("EVENT_BUS_ALLOW_CONTROL")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);

        let listener = TcpListener::bind("127.0.0.1:7001").await?;
        info!("Event bus listening on 127.0.0.1:7001 (Postcard format)");

        tokio::spawn(async move {
            while let Ok((stream, addr)) = listener.accept().await {
                info!("TUI connected from: {}", addr);
                let (mut reader, mut writer) = tokio::io::split(stream);
                let mut client_rx = tx_clone.subscribe();

                let cmd_tx_inner = control_tx.clone();

                // Read task (TUI -> Daemon)
                tokio::spawn(async move {
                    let mut length_buf = [0u8; 4];
                    loop {
                        if reader.read_exact(&mut length_buf).await.is_err() {
                            break;
                        }
                        let len = u32::from_le_bytes(length_buf) as usize;
                        if len == 0 || len > MAX_FRAME_LEN {
                            warn!(len, "Dropping invalid event-bus control frame");
                            break;
                        }

                        let mut buf = vec![0u8; len];
                        if reader.read_exact(&mut buf).await.is_err() {
                            break;
                        }
                        if let Ok(cmd) = postcard::from_bytes::<ControlCommand>(&buf) {
                            if allow_control {
                                info!("Received command: {:?}", cmd);
                                let _ = cmd_tx_inner.try_send(cmd);
                            } else {
                                warn!(
                                    "Ignoring unauthenticated event-bus control command; set EVENT_BUS_ALLOW_CONTROL=1 to enable local control"
                                );
                            }
                        }
                    }
                    info!("TUI read task closed for {}", addr);
                });

                // Write task (Daemon -> TUI)
                tokio::spawn(async move {
                    loop {
                        match client_rx.recv().await {
                            Ok(event) => {
                                if let Ok(bytes) = postcard::to_allocvec(&event) {
                                    if bytes.len() > MAX_FRAME_LEN {
                                        warn!(len = bytes.len(), "Skipping oversized event frame");
                                        continue;
                                    }
                                    let len_bytes = (bytes.len() as u32).to_le_bytes();
                                    if writer.write_all(&len_bytes).await.is_err() {
                                        break;
                                    }
                                    if writer.write_all(&bytes).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                warn!(
                                    skipped,
                                    "Event-bus subscriber lagged; dropping missed events"
                                );
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    info!("TUI write task closed for {}", addr);
                });
            }
        });

        Ok(Self { tx })
    }

    pub fn broadcast(&self, event: BotEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BotEvent> {
        self.tx.subscribe()
    }
}
