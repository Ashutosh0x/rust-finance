use tokio::{
    net::TcpListener,
    io::{AsyncWriteExt, AsyncReadExt},
    sync::{mpsc, broadcast},
};
use common::events::{BotEvent, ControlCommand};
use tracing::{info, error};

pub mod subscriber;
pub mod health;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<BotEvent>,
}

impl EventBus {
    pub async fn start(control_tx: mpsc::Sender<ControlCommand>) -> anyhow::Result<Self> {
        let (tx, _) = broadcast::channel::<BotEvent>(100_000);
        let tx_clone = tx.clone();

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
                        if len > 1024 * 1024 { break; } // Safety limit
                        
                        let mut buf = vec![0u8; len];
                        if reader.read_exact(&mut buf).await.is_err() {
                            break;
                        }
                        if let Ok(cmd) = postcard::from_bytes::<ControlCommand>(&buf) {
                            info!("Received command: {:?}", cmd);
                            let _ = cmd_tx_inner.try_send(cmd);
                        }
                    }
                    info!("TUI read task closed for {}", addr);
                });

                // Write task (Daemon -> TUI)
                tokio::spawn(async move {
                    while let Ok(event) = client_rx.recv().await {
                        if let Ok(bytes) = postcard::to_allocvec(&event) {
                            let len_bytes = (bytes.len() as u32).to_le_bytes();
                            if writer.write_all(&len_bytes).await.is_err() {
                                break;
                            }
                            if writer.write_all(&bytes).await.is_err() {
                                break;
                            }
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
