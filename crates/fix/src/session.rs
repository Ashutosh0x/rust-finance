// crates/fix/src/session.rs
//
// FIX 4.4 Session Layer implementation.
// Manages TCP connection, logon handshake, sequence numbers, heartbeats,
// and resend requests (Gap Fill).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, Instant};
use tracing::{error, info, warn};
use crate::{serializer::{FixMessage, FixParser, MsgType}, FixError};

const SENDER_COMP_ID: &str = "RUSTFORGE_OMS";
const TARGET_COMP_ID: &str = "EXCHANGE_GW";

pub struct FixSessionConfig {
    pub host: String,
    pub port: u16,
    pub heartbeat_interval: u64,
    pub reset_seq_num: bool,
}

pub struct FixSession {
    config: FixSessionConfig,
    outbound_seq: AtomicU64,
    inbound_seq: AtomicU64,
    /// Channel for the application to send messages to the FIX gateway.
    tx_app: mpsc::Sender<FixMessage>,
    /// Channel for the session to deliver market/fill messages to the app.
    rx_app: mpsc::Receiver<FixMessage>,
}

impl FixSession {
    pub fn new(
        config: FixSessionConfig,
        rx_app: mpsc::Receiver<FixMessage>,
    ) -> (Self, mpsc::Receiver<FixMessage>) {
        let (tx_inbound, rx_inbound) = mpsc::channel(1024);
        let session = Self {
            config,
            outbound_seq: AtomicU64::new(1),
            inbound_seq: AtomicU64::new(0),
            tx_app: tx_inbound,
            rx_app,
        };
        (session, rx_inbound)
    }

    fn next_outbound_seq(&self) -> u64 {
        self.outbound_seq.fetch_add(1, Ordering::SeqCst)
    }

    fn build_header(&self, msg: &mut FixMessage) {
        msg.set_field(8, "FIX.4.4"); // BeginString
        msg.set_field(49, SENDER_COMP_ID);
        msg.set_field(56, TARGET_COMP_ID);
        msg.set_field(34, &self.next_outbound_seq().to_string());
        msg.set_field(
            52,
            &chrono::Utc::now().format("%Y%m%d-%H:%M:%S.%3f").to_string(),
        ); // SendingTime
    }

    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) -> Result<(), FixError> {
        loop {
            let addr = format!("{}:{}", self.config.host, self.config.port);
            info!("Attempting FIX TCP connection to {}", addr);

            match TcpStream::connect(&addr).await {
                Ok(mut stream) => {
                    info!("FIX connected to {}", addr);
                    let (mut reader, mut writer) = stream.split();
                    
                    // 1. Send Logon
                    let mut logon = FixMessage::new(MsgType::Logon);
                    logon.set_field(108, &self.config.heartbeat_interval.to_string()); // HeartBtInt
                    if self.config.reset_seq_num {
                        logon.set_field(141, "Y"); // ResetSeqNumFlag
                        self.outbound_seq.store(1, Ordering::SeqCst);
                        self.inbound_seq.store(0, Ordering::SeqCst);
                    } else {
                        logon.set_field(141, "N");
                    }
                    self.build_header(&mut logon);
                    let out_bytes = logon.encode();
                    writer.write_all(&out_bytes).await.map_err(|e| FixError::Io(e))?;

                    let mut heartbeat_timer = interval(Duration::from_secs(self.config.heartbeat_interval));
                    let mut read_buf = vec![0u8; 8192];
                    let mut parser = FixParser::new();
                    let mut last_rx = Instant::now();

                    loop {
                        tokio::select! {
                            // Listen for shutdown
                            _ = shutdown.changed() => {
                                if *shutdown.borrow() {
                                    info!("FIX Session shutting down gracefully...");
                                    let mut logout = FixMessage::new(MsgType::Logout);
                                    self.build_header(&mut logout);
                                    let _ = writer.write_all(&logout.encode()).await;
                                    return Ok(());
                                }
                            }

                            // Application wants to send a message
                            Some(mut app_msg) = self.rx_app.recv() => {
                                self.build_header(&mut app_msg);
                                if let Err(e) = writer.write_all(&app_msg.encode()).await {
                                    error!("FIX Send error: {}", e);
                                    break;
                                }
                            }

                            // Heartbeat tick
                            _ = heartbeat_timer.tick() => {
                                let mut hb = FixMessage::new(MsgType::Heartbeat);
                                self.build_header(&mut hb);
                                if let Err(e) = writer.write_all(&hb.encode()).await {
                                    error!("FIX Heartbeat write error: {}", e);
                                    break;
                                }

                                // Check if target is silent (TestRequest / Disconnect logic)
                                if last_rx.elapsed() > Duration::from_secs(self.config.heartbeat_interval as u64 + 5) {
                                    warn!("FIX connection timed out (no heartbeat from target)");
                                    break;
                                }
                            }

                            // Read incoming bytes
                            n = reader.read(&mut read_buf) => {
                                match n {
                                    Ok(0) => {
                                        warn!("FIX connection closed by peer");
                                        break;
                                    }
                                    Ok(n) => {
                                        last_rx = Instant::now();
                                        parser.push_bytes(&read_buf[..n]);
                                        while let Some(msg) = parser.next_message() {
                                            self.handle_inbound(msg, &mut writer).await?;
                                        }
                                    }
                                    Err(e) => {
                                        error!("FIX Read error: {}", e);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("FIX connect error: {}. Retrying in 5s...", e);
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    async fn handle_inbound(
        &self,
        msg: FixMessage,
        writer: &mut tokio::net::tcp::WriteHalf<'_>,
    ) -> Result<(), FixError> {
        let incoming_seq: u64 = msg.get_field(34)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        
        let expected_seq = self.inbound_seq.load(Ordering::SeqCst) + 1;

        if incoming_seq > expected_seq {
            warn!("FIX SeqNum Gap detected. Expected {}, got {}. Sending ResendRequest.", expected_seq, incoming_seq);
            let mut resend = FixMessage::new(MsgType::ResendRequest);
            self.build_header(&mut resend);
            resend.set_field(7, &expected_seq.to_string()); // BeginSeqNo
            resend.set_field(16, "0"); // EndSeqNo (0 = infinity)
            writer.write_all(&resend.encode()).await.map_err(|e| FixError::Io(e))?;
            return Ok(());
        } else if incoming_seq < expected_seq {
            let msg_type = msg.get_field(35).unwrap_or("");
            if msg_type != "4" { // 4 = SequenceReset
                error!("FIX Fatal: Incoming seq num {} lower than expected {}", incoming_seq, expected_seq);
                // In production, send Logout + disconnect here
                return Ok(());
            }
        }

        self.inbound_seq.store(incoming_seq, Ordering::SeqCst);

        // Route administrative vs application messages
        let msg_type = msg.msg_type();
        match msg_type {
            MsgType::Logon => info!("FIX Logon accepted by exchange"),
            MsgType::Logout => warn!("FIX Logout received from exchange"),
            MsgType::Heartbeat | MsgType::TestRequest => { /* Handled automatically by parser loop tick */ },
            MsgType::ResendRequest => {
                // Should replay historical messages from our store here
                warn!("Ignoring ResendRequest from exchange (Unimplemented replay buffer)");
                let mut seq_reset = FixMessage::new(MsgType::SequenceReset);
                self.build_header(&mut seq_reset);
                seq_reset.set_field(36, &(self.outbound_seq.load(Ordering::SeqCst)).to_string()); // NewSeqNo
                seq_reset.set_field(123, "Y"); // GapFillFlag
                writer.write_all(&seq_reset.encode()).await.map_err(|e| FixError::Io(e))?;
            }
            MsgType::ExecutionReport | MsgType::OrderCancelReject => {
                // Forward execution reports up to the OMS
                let _ = self.tx_app.send(msg).await;
            }
            _ => {
                // Forward other app messages
                let _ = self.tx_app.send(msg).await;
            }
        }

        Ok(())
    }
}
