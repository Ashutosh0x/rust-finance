#![forbid(unsafe_code)]
use common::SwapEvent;
use anyhow::Result;
use crossbeam_channel::Receiver;
use tracing::{info, debug};
use serde_json::Value;

pub struct ParserService {
    rx: Receiver<String>,
}

impl ParserService {
    pub fn new(rx: Receiver<String>) -> Self {
        Self { rx }
    }

    pub fn rx(&self) -> &Receiver<String> {
        &self.rx
    }

    pub fn run(&self) -> Result<()> {
        info!("Parser service started on thread: {:?}", std::thread::current().id());
        while let Ok(raw_msg) = self.rx.recv() {
            if let Err(e) = self.process_message(&raw_msg) {
                // Avoid logging in hot path unless critical
                debug!("Error processing message: {:?}", e);
            }
        }
        Ok(())
    }

    pub fn process_message(&self, msg: &str) -> Result<Vec<SwapEvent>> {
        let v: Value = serde_json::from_str(msg)?;
        let mut events = Vec::new();
        
        // Handle Replay Format: { "replay_slot": u64, "logs": [...] }
        if let Some(logs) = v.get("logs").and_then(|l| l.as_array()) {
            let signature = v.get("signature").and_then(|s| s.as_str()).unwrap_or("replay");
            let slot = v.get("replay_slot").and_then(|s| s.as_u64()).unwrap_or(0);
            
            for log in logs {
                if let Some(log_str) = log.as_str() {
                    if let Some(mut event) = self.parse_log_line(log_str, signature) {
                        event.slot = slot;
                        events.push(event);
                    }
                }
            }
            return Ok(events);
        }

        // Handle Standard WS Format: { "params": { "result": { "value": { "logs": [...], "signature": "..." } } } }
        if let Some(params) = v.get("params") {
            if let Some(result) = params.get("result") {
                if let Some(value) = result.get("value") {
                    let signature = value.get("signature").and_then(|s| s.as_str()).unwrap_or("unknown");
                    let slot = result.get("context").and_then(|c| c.get("slot")).and_then(|s| s.as_u64()).unwrap_or(0);
                    
                    if let Some(logs) = value.get("logs").and_then(|l| l.as_array()) {
                        for log in logs {
                            if let Some(log_str) = log.as_str() {
                                if let Some(mut event) = self.parse_log_line(log_str, signature) {
                                    event.slot = slot;
                                    events.push(event);
                                }
                            }
                        }
                    }
                }
            }
        }
        
        Ok(events)
    }

    /// Fast path for log line parsing. 
    /// In a real bot, we'd use a state machine or pre-compiled regex for Raydium/Orca/Jupiter instructions.
    pub fn parse_log_line(&self, line: &str, signature: &str) -> Option<SwapEvent> {
        if line.contains("Instruction: Swap") {
            // In a real bot, we'd regex the logs for "Parsed amount_in: 5000000"
            return Some(SwapEvent {
                tx_sig: signature.to_string(),
                timestamp: std::time::SystemTime::now(),
                token_in: "USDC".into(),
                token_out: "SOL".into(),
                amount_in: 2_000_000, // Trigger simple strategy (threshold 1M)
                amount_out: 5_000_000,
                pool: "mock_pool".into(),
                slot: 0,
            });
        }
        None
    }
}
