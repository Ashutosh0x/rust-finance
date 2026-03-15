use tokio::{
    net::TcpListener,
    io::{AsyncWriteExt, AsyncBufReadExt, BufReader},
    sync::mpsc,
};
use common::events::{BotEvent, ControlCommand};
use tracing::{info, error};

pub mod subscriber;
pub mod health;

enum BusCmd {
    AddClient(mpsc::Sender<BotEvent>),
    Broadcast(BotEvent),
}

#[derive(Clone)]
pub struct EventBus {
    cmd_tx: mpsc::Sender<BusCmd>,
}

impl EventBus {
    pub async fn start(control_tx: mpsc::Sender<ControlCommand>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:7001").await?;
        info!("Event bus listening on 127.0.0.1:7001");
        
        let (bus_tx, mut bus_rx) = mpsc::channel::<BusCmd>(4096);
        let bus_clone = bus_tx.clone();

        // Single dedicated state manager task for routing events
        tokio::spawn(async move {
            let mut clients: Vec<mpsc::Sender<BotEvent>> = Vec::new();

            while let Some(cmd) = bus_rx.recv().await {
                match cmd {
                    BusCmd::AddClient(tx) => clients.push(tx),
                    BusCmd::Broadcast(ev) => {
                        clients.retain(|tx| tx.try_send(ev.clone()).is_ok());
                    }
                }
            }
        });

        tokio::spawn(async move {
            while let Ok((stream, addr)) = listener.accept().await {
                info!("TUI connected from: {}", addr);
                let (reader, mut writer) = tokio::io::split(stream);
                let (client_tx, mut client_rx) = mpsc::channel::<BotEvent>(1024);
                
                let _ = bus_clone.send(BusCmd::AddClient(client_tx)).await;

                // Read task (TUI -> Daemon)
                let cmd_tx_inner = control_tx.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(reader).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Ok(cmd) = serde_json::from_str::<ControlCommand>(&line) {
                            info!("Received command: {:?}", cmd);
                            let _ = cmd_tx_inner.try_send(cmd);
                        }
                    }
                });

                // Write task (Daemon -> TUI)
                tokio::spawn(async move {
                    while let Some(event) = client_rx.recv().await {
                        if let Ok(json) = serde_json::to_string(&event) {
                            if let Err(e) = writer.write_all((json + "\n").as_bytes()).await {
                                error!("Failed to write to client {}: {:?}", addr, e);
                                break;
                            }
                        }
                    }
                });
            }
        });

        Ok(Self { cmd_tx: bus_tx })
    }

    pub fn broadcast(&self, event: BotEvent) {
        let _ = self.cmd_tx.try_send(BusCmd::Broadcast(event));
    }
}
