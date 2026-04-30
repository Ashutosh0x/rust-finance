// crates/risk/src/wal.rs
//
// Write-Ahead Log (WAL) for Trading State Persistence
//
// Gap: Strategy state (positions, PnL, toxicity EMA, regime) is volatile.
//      A crash means total loss of context — the system restarts "cold."
//
// Solution: Append-only WAL using SQLite's native WAL mode.
//   - Every state mutation is logged BEFORE being applied
//   - Periodic checkpoints snapshot the full state
//   - On restart, replay from last checkpoint + LSN
//
// Based on NautilusTrader's Redis Stream + LSN replay pattern,
// adapted for SQLite (already in the dependency tree).

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

// ── WAL Entry Types ──────────────────────────────────────────────

/// Log Sequence Number — monotonically increasing.
pub type LSN = u64;

/// A single WAL entry representing a state mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WALEntry {
    // ── Order Lifecycle ──────────────────────────────────────
    OrderSubmitted {
        order_id: String,
        symbol: String,
        side: String,
        quantity: f64,
        price: Option<f64>,
        order_type: String,
    },
    OrderFilled {
        order_id: String,
        fill_price: f64,
        fill_quantity: f64,
        commission: f64,
    },
    OrderCancelled {
        order_id: String,
        reason: String,
    },
    OrderRejected {
        order_id: String,
        reason: String,
    },

    // ── Position Lifecycle ────────────────────────────────────
    PositionOpened {
        symbol: String,
        side: String,
        quantity: f64,
        entry_price: f64,
    },
    PositionUpdated {
        symbol: String,
        new_quantity: f64,
        avg_price: f64,
        unrealized_pnl: f64,
    },
    PositionClosed {
        symbol: String,
        exit_price: f64,
        realized_pnl: f64,
    },

    // ── Signal State ─────────────────────────────────────────
    SignalState {
        toxicity_ema: f64,
        regime: String,
        composite_score: f64,
        composite_conviction: f64,
    },
    AlphaHealthState {
        signal_name: String,
        ic: f64,
        hit_rate: f64,
        health: String,
    },

    // ── Risk State ───────────────────────────────────────────
    RiskState {
        portfolio_value: f64,
        daily_pnl: f64,
        drawdown_pct: f64,
        var_95: f64,
    },
    KillSwitchEvent {
        activated: bool,
        reason: String,
    },

    // ── Checkpoint ───────────────────────────────────────────
    /// Full state snapshot. On recovery, start from the latest checkpoint.
    Checkpoint {
        portfolio_value: f64,
        daily_pnl: f64,
        peak_equity: f64,
        open_positions_json: String,
        signal_state_json: String,
    },
}

/// A WAL record with its sequence number and timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WALRecord {
    pub lsn: LSN,
    pub timestamp_ms: i64,
    pub entry: WALEntry,
}

// ── In-Memory WAL Engine ─────────────────────────────────────────

/// WAL engine that can be backed by SQLite or in-memory (for tests).
///
/// The in-memory implementation is used for unit tests.
/// For production, wire this to the SQLite persistence layer
/// via `crates/persistence/src/wal.rs`.
pub struct WALEngine {
    /// Monotonically increasing sequence counter.
    next_lsn: AtomicU64,

    /// In-memory log (for tests and small deployments).
    /// Production systems should replace this with SQLite WAL writes.
    log: Vec<WALRecord>,

    /// Checkpoint interval: auto-checkpoint every N entries.
    checkpoint_interval: usize,

    /// Entries since last checkpoint.
    entries_since_checkpoint: usize,
}

impl WALEngine {
    pub fn new(checkpoint_interval: usize) -> Self {
        Self {
            next_lsn: AtomicU64::new(1),
            log: Vec::with_capacity(1024),
            checkpoint_interval,
            entries_since_checkpoint: 0,
        }
    }

    /// Append an entry to the WAL. Returns the assigned LSN.
    ///
    /// CRITICAL: This must be called BEFORE applying the state mutation.
    /// The pattern is: log → apply → ack.
    pub fn append(&mut self, entry: WALEntry) -> LSN {
        let lsn = self.next_lsn.fetch_add(1, Ordering::SeqCst);
        let record = WALRecord {
            lsn,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            entry,
        };
        self.log.push(record);
        self.entries_since_checkpoint += 1;
        lsn
    }

    /// Check if a checkpoint should be taken.
    pub fn needs_checkpoint(&self) -> bool {
        self.entries_since_checkpoint >= self.checkpoint_interval
    }

    /// Write a checkpoint entry and reset the counter.
    pub fn write_checkpoint(
        &mut self,
        portfolio_value: f64,
        daily_pnl: f64,
        peak_equity: f64,
        open_positions_json: &str,
        signal_state_json: &str,
    ) -> LSN {
        let lsn = self.append(WALEntry::Checkpoint {
            portfolio_value,
            daily_pnl,
            peak_equity,
            open_positions_json: open_positions_json.to_string(),
            signal_state_json: signal_state_json.to_string(),
        });
        self.entries_since_checkpoint = 0;
        lsn
    }

    /// Recover state from the WAL.
    ///
    /// Returns:
    /// 1. The latest checkpoint (if any)
    /// 2. All entries AFTER the checkpoint that need to be replayed
    pub fn recover(&self) -> (Option<&WALRecord>, Vec<&WALRecord>) {
        // Find the latest checkpoint
        let checkpoint = self
            .log
            .iter()
            .rev()
            .find(|r| matches!(r.entry, WALEntry::Checkpoint { .. }));

        let replay_from_lsn = checkpoint
            .map(|cp| cp.lsn + 1)
            .unwrap_or(1); // If no checkpoint, replay everything

        let entries_to_replay: Vec<&WALRecord> = self
            .log
            .iter()
            .filter(|r| r.lsn >= replay_from_lsn && !matches!(r.entry, WALEntry::Checkpoint { .. }))
            .collect();

        (checkpoint, entries_to_replay)
    }

    /// Get all records (for debugging/audit).
    pub fn all_records(&self) -> &[WALRecord] {
        &self.log
    }

    /// Get the current LSN (last written).
    pub fn current_lsn(&self) -> LSN {
        self.next_lsn.load(Ordering::SeqCst) - 1
    }

    /// Total number of entries in the log.
    pub fn len(&self) -> usize {
        self.log.len()
    }

    /// Is the log empty?
    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }

    /// Truncate entries before a given LSN (garbage collection).
    /// Keep the latest checkpoint and everything after it.
    pub fn truncate_before_checkpoint(&mut self) {
        if let Some(checkpoint_idx) = self
            .log
            .iter()
            .rposition(|r| matches!(r.entry, WALEntry::Checkpoint { .. }))
        {
            if checkpoint_idx > 0 {
                self.log.drain(..checkpoint_idx);
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wal_append_and_lsn() {
        let mut wal = WALEngine::new(100);

        let lsn1 = wal.append(WALEntry::OrderSubmitted {
            order_id: "ORD-001".into(),
            symbol: "BTCUSDT".into(),
            side: "BUY".into(),
            quantity: 0.5,
            price: Some(60000.0),
            order_type: "LIMIT".into(),
        });

        let lsn2 = wal.append(WALEntry::OrderFilled {
            order_id: "ORD-001".into(),
            fill_price: 60000.0,
            fill_quantity: 0.5,
            commission: 12.0,
        });

        assert_eq!(lsn1, 1);
        assert_eq!(lsn2, 2);
        assert_eq!(wal.len(), 2);
        assert_eq!(wal.current_lsn(), 2);
    }

    #[test]
    fn test_checkpoint_and_recovery() {
        let mut wal = WALEngine::new(100);

        // Pre-checkpoint entries
        wal.append(WALEntry::OrderSubmitted {
            order_id: "ORD-001".into(),
            symbol: "BTCUSDT".into(),
            side: "BUY".into(),
            quantity: 0.5,
            price: Some(60000.0),
            order_type: "LIMIT".into(),
        });
        wal.append(WALEntry::OrderFilled {
            order_id: "ORD-001".into(),
            fill_price: 60000.0,
            fill_quantity: 0.5,
            commission: 12.0,
        });

        // Checkpoint at LSN 3
        let cp_lsn = wal.write_checkpoint(
            100_000.0,
            500.0,
            100_500.0,
            r#"[{"symbol":"BTCUSDT","qty":0.5}]"#,
            r#"{"regime":"Normal","toxicity":0.3}"#,
        );
        assert_eq!(cp_lsn, 3);

        // Post-checkpoint entries
        wal.append(WALEntry::OrderSubmitted {
            order_id: "ORD-002".into(),
            symbol: "ETHUSDT".into(),
            side: "BUY".into(),
            quantity: 5.0,
            price: Some(3000.0),
            order_type: "MARKET".into(),
        });

        // Recovery
        let (checkpoint, replay) = wal.recover();
        assert!(checkpoint.is_some(), "Should find checkpoint");
        assert_eq!(checkpoint.unwrap().lsn, 3);
        assert_eq!(replay.len(), 1, "Should have 1 entry to replay");
        assert!(matches!(replay[0].entry, WALEntry::OrderSubmitted { .. }));
    }

    #[test]
    fn test_recovery_no_checkpoint() {
        let mut wal = WALEngine::new(100);

        wal.append(WALEntry::PositionOpened {
            symbol: "AAPL".into(),
            side: "LONG".into(),
            quantity: 100.0,
            entry_price: 175.0,
        });

        let (checkpoint, replay) = wal.recover();
        assert!(checkpoint.is_none(), "No checkpoint should exist");
        assert_eq!(replay.len(), 1, "All entries should be replayed");
    }

    #[test]
    fn test_needs_checkpoint() {
        let mut wal = WALEngine::new(3); // checkpoint every 3 entries

        wal.append(WALEntry::SignalState {
            toxicity_ema: 0.3,
            regime: "Normal".into(),
            composite_score: 0.5,
            composite_conviction: 0.7,
        });
        assert!(!wal.needs_checkpoint());

        wal.append(WALEntry::SignalState {
            toxicity_ema: 0.35,
            regime: "Normal".into(),
            composite_score: 0.6,
            composite_conviction: 0.8,
        });
        assert!(!wal.needs_checkpoint());

        wal.append(WALEntry::RiskState {
            portfolio_value: 100_000.0,
            daily_pnl: 200.0,
            drawdown_pct: 0.01,
            var_95: 1500.0,
        });
        assert!(wal.needs_checkpoint(), "Should need checkpoint after 3 entries");
    }

    #[test]
    fn test_truncate_before_checkpoint() {
        let mut wal = WALEngine::new(100);

        // Old entries
        for i in 0..5 {
            wal.append(WALEntry::SignalState {
                toxicity_ema: i as f64 * 0.1,
                regime: "Normal".into(),
                composite_score: 0.0,
                composite_conviction: 0.0,
            });
        }

        // Checkpoint
        wal.write_checkpoint(100_000.0, 0.0, 100_000.0, "[]", "{}");

        // New entries
        wal.append(WALEntry::SignalState {
            toxicity_ema: 0.5,
            regime: "HighVol".into(),
            composite_score: 0.0,
            composite_conviction: 0.0,
        });

        assert_eq!(wal.len(), 7); // 5 + 1 checkpoint + 1 new

        wal.truncate_before_checkpoint();

        // Should keep checkpoint + entries after it
        assert!(wal.len() <= 3, "Should have truncated old entries, len={}", wal.len());
    }

    #[test]
    fn test_kill_switch_event_logged() {
        let mut wal = WALEngine::new(100);

        wal.append(WALEntry::KillSwitchEvent {
            activated: true,
            reason: "Daily loss $5000 exceeds limit $3000".into(),
        });

        let records = wal.all_records();
        assert_eq!(records.len(), 1);
        if let WALEntry::KillSwitchEvent { activated, reason } = &records[0].entry {
            assert!(*activated);
            assert!(reason.contains("Daily loss"));
        } else {
            panic!("Expected KillSwitchEvent");
        }
    }

    #[test]
    fn test_lsn_monotonically_increasing() {
        let mut wal = WALEngine::new(100);
        let mut prev_lsn = 0;
        for _ in 0..50 {
            let lsn = wal.append(WALEntry::SignalState {
                toxicity_ema: 0.0,
                regime: "Normal".into(),
                composite_score: 0.0,
                composite_conviction: 0.0,
            });
            assert!(lsn > prev_lsn, "LSN must be monotonically increasing");
            prev_lsn = lsn;
        }
    }

    #[test]
    fn test_multiple_checkpoints_recovery_uses_latest() {
        let mut wal = WALEngine::new(100);

        // First batch
        wal.append(WALEntry::PositionOpened {
            symbol: "OLD".into(), side: "LONG".into(), quantity: 10.0, entry_price: 100.0,
        });
        wal.write_checkpoint(50_000.0, -200.0, 50_200.0, r#"[{"OLD":10}]"#, "{}");

        // Second batch
        wal.append(WALEntry::PositionOpened {
            symbol: "NEW".into(), side: "SHORT".into(), quantity: 5.0, entry_price: 200.0,
        });
        wal.write_checkpoint(55_000.0, 300.0, 55_000.0, r#"[{"NEW":5}]"#, "{}");

        // Third batch (post-checkpoint)
        wal.append(WALEntry::OrderSubmitted {
            order_id: "FINAL".into(), symbol: "LATEST".into(), side: "BUY".into(),
            quantity: 1.0, price: Some(500.0), order_type: "MARKET".into(),
        });

        let (checkpoint, replay) = wal.recover();
        let cp = checkpoint.unwrap();

        // Should use the LATEST checkpoint (LSN 4)
        if let WALEntry::Checkpoint { portfolio_value, .. } = &cp.entry {
            assert!((*portfolio_value - 55_000.0).abs() < 1e-10,
                "Should use latest checkpoint, got {}", portfolio_value);
        }

        // Only 1 entry to replay (the FINAL order)
        assert_eq!(replay.len(), 1);
    }
}
