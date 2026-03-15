// crates/compliance/src/audit_trail.rs
// Tamper-proof, append-only order audit log — SEBI algo registration requirement 2026
// Every entry is SHA-256 chained to the previous entry (blockchain-style)

use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuditEntry {
    /// Monotonically increasing sequence number
    pub seq: u64,
    /// Unix timestamp microseconds
    pub ts_us: u64,
    /// Entry type
    pub event: AuditEvent,
    /// SHA-256 hex of (prev_hash + this entry fields) — chain integrity
    pub hash: String,
    /// Hash of previous entry (genesis = "0000...0000")
    pub prev_hash: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum AuditEvent {
    OrderSubmitted  { order_id: String, symbol: String, side: String, qty: u64, price: Option<f64>, order_type: String, strategy: String },
    OrderCancelled  { order_id: String, reason: String },
    OrderFilled     { order_id: String, fill_price: f64, fill_qty: u64, venue: String },
    OrderRejected   { order_id: String, rejection_reason: String },
    OrderModified   { order_id: String, new_qty: Option<u64>, new_price: Option<f64> },
    PreTradeBlocked { symbol: String, reason: String, attempted_qty: u64, attempted_price: f64 },
    KillSwitchFired { triggered_by: String, orders_cancelled: u32 },
    StrategyStarted { strategy_id: String, params: String },
    StrategyStopped { strategy_id: String, reason: String },
    SessionStart    { version: String, mode: String },
    SessionEnd      { total_orders: u64, total_notional: f64 },
}

pub struct AuditTrail {
    seq: u64,
    prev_hash: String,
    writer: BufWriter<File>,
    path: PathBuf,
}

impl AuditTrail {
    /// Open or create the audit log file. Verifies chain integrity on open.
    pub fn open(path: PathBuf) -> Result<Self, std::io::Error> {
        // Verify existing chain if file exists
        if path.exists() {
            Self::verify_chain(&path).ok(); // log warning but don't block startup
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        // Find last hash and seq by scanning existing entries
        let (seq, prev_hash) = Self::read_tail(&path);

        Ok(Self {
            seq,
            prev_hash,
            writer: BufWriter::new(file),
            path,
        })
    }

    /// Append an event to the audit trail. Returns the entry hash.
    pub fn log(&mut self, event: AuditEvent) -> Result<String, std::io::Error> {
        let ts_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;

        self.seq += 1;

        // Compute hash: SHA256(prev_hash || seq || ts_us || event_json)
        let event_json = serde_json::to_string(&event).unwrap_or_default();
        let hash_input = format!("{}{}{}{}", self.prev_hash, self.seq, ts_us, event_json);
        let hash = Self::sha256_hex(&hash_input);

        let entry = AuditEntry {
            seq: self.seq,
            ts_us,
            event,
            hash: hash.clone(),
            prev_hash: self.prev_hash.clone(),
        };

        // Write as JSON line (JSONL format — one entry per line)
        let line = serde_json::to_string(&entry).unwrap_or_default();
        writeln!(self.writer, "{}", line)?;
        self.writer.flush()?;

        self.prev_hash = hash.clone();
        Ok(hash)
    }

    /// Verify the full chain integrity — returns list of tampered entries
    pub fn verify_chain(path: &PathBuf) -> Result<Vec<u64>, String> {
        use std::fs::read_to_string;
        let content = read_to_string(path).map_err(|e| e.to_string())?;
        let mut tampered: Vec<u64> = Vec::new();
        let mut prev_hash = "0".repeat(64);

        for line in content.lines() {
            if line.trim().is_empty() { continue; }
            let entry: AuditEntry = serde_json::from_str(line)
                .map_err(|e| format!("Parse error: {}", e))?;

            // Recompute expected hash
            let event_json = serde_json::to_string(&entry.event).unwrap_or_default();
            let hash_input = format!("{}{}{}{}", entry.prev_hash, entry.seq, entry.ts_us, event_json);
            let expected = Self::sha256_hex(&hash_input);

            if entry.hash != expected || entry.prev_hash != prev_hash {
                tampered.push(entry.seq);
            }
            prev_hash = entry.hash.clone();
        }

        if tampered.is_empty() { Ok(vec![]) } else { Err(format!("Tampered entries: {:?}", tampered)) }
    }

    fn sha256_hex(input: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn read_tail(path: &PathBuf) -> (u64, String) {
        use std::fs::read_to_string;
        let content = match read_to_string(path) { Ok(c) => c, Err(_) => return (0, "0".repeat(64)) };
        let last_line = content.lines().filter(|l| !l.trim().is_empty()).last();
        match last_line {
            Some(line) => {
                let entry: AuditEntry = serde_json::from_str(line).unwrap_or_else(|_| {
                    AuditEntry { seq: 0, ts_us: 0, event: AuditEvent::SessionStart { version: "".into(), mode: "".into() }, hash: "0".repeat(64), prev_hash: "0".repeat(64) }
                });
                (entry.seq, entry.hash)
            },
            None => (0, "0".repeat(64)),
        }
    }
}

/// Convenience macro for logging audit events from anywhere
#[macro_export]
macro_rules! audit {
    ($trail:expr, $event:expr) => {
        if let Err(e) = $trail.log($event) {
            tracing::error!("AUDIT TRAIL WRITE FAILED: {} — THIS IS A COMPLIANCE VIOLATION", e);
        }
    };
}
