//! Gap Detection & Recovery Engine for Market Data Feeds
//!
//! Implements the full gap detection pipeline:
//!
//!   Exchange Feed → Heartbeat Monitor → Sequence Checker → Gap Buffer
//!        ↓                                    ↓
//!   Normal Processing              Recovery Strategy Router
//!   (update order book)            ╱          │          ╲
//!                           <50 msgs    50–500 msgs   >500/timeout
//!                           Retransmit  Full Snapshot  Failover Feed
//!                                  ╲         │         ╱
//!                              Reconciliation Engine
//!                                       ↓
//!                              Resume Live Feed
//!                              (unfreeze UI, drain buffer)
//!
//! Each gap is logged to the audit trail with timestamp, size, and source.

use common::events::{Envelope, MarketEvent};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

// ─── Configuration ───────────────────────────────────────────────────────────

/// Tunable parameters for the gap detector.
#[derive(Debug, Clone)]
pub struct GapDetectorConfig {
    /// Heartbeat timeout: if no message for this long, declare stale feed.
    pub heartbeat_timeout: Duration,
    /// Ping interval for keep-alive.
    pub ping_interval: Duration,
    /// Max messages to request via retransmission (small gaps).
    pub retransmit_threshold: u64,
    /// Above this, request a full snapshot instead.
    pub snapshot_threshold: u64,
    /// Above this (or timeout), trigger failover to backup feed.
    pub failover_threshold: u64,
    /// How long to wait for retransmission before escalating.
    pub recovery_timeout: Duration,
    /// Maximum gap buffer size before forced flush.
    pub max_buffer_size: usize,
}

impl Default for GapDetectorConfig {
    fn default() -> Self {
        Self {
            heartbeat_timeout: Duration::from_millis(2500),
            ping_interval: Duration::from_millis(500),
            retransmit_threshold: 50,
            snapshot_threshold: 500,
            failover_threshold: 500,
            recovery_timeout: Duration::from_secs(5),
            max_buffer_size: 10_000,
        }
    }
}

// ─── Gap Alert (audit log) ───────────────────────────────────────────────────

/// Immutable record of a detected gap, written to the audit trail.
#[derive(Debug, Clone)]
pub struct GapAlert {
    /// When the gap was detected.
    pub detected_at: Instant,
    /// Source feed name (e.g., "Binance", "Alpaca").
    pub source: String,
    /// First missing sequence ID.
    pub gap_start: u64,
    /// Last missing sequence ID (inclusive).
    pub gap_end: u64,
    /// Total messages missing.
    pub gap_size: u64,
    /// Recovery strategy selected.
    pub strategy: RecoveryStrategy,
    /// Whether recovery succeeded.
    pub recovered: bool,
    /// Time spent recovering.
    pub recovery_duration: Option<Duration>,
}

/// Which recovery path was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStrategy {
    /// Gap < 50: request specific missed sequence numbers.
    Retransmission,
    /// 50 ≤ Gap < 500: request a full order book snapshot.
    FullSnapshot,
    /// Gap ≥ 500 or timeout: switch to backup data source.
    Failover,
}

impl RecoveryStrategy {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Retransmission => "Retransmission",
            Self::FullSnapshot => "Full Snapshot",
            Self::Failover => "Failover Feed",
        }
    }
}

// ─── Feed State ──────────────────────────────────────────────────────────────

/// Health status of a single feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedStatus {
    /// Receiving messages normally.
    Live,
    /// Gap detected, buffering incoming messages.
    GapDetected,
    /// Recovery in progress (retransmit / snapshot / failover).
    Recovering,
    /// Feed timed out, no heartbeat.
    Stale,
}

// ─── Gap Detector ────────────────────────────────────────────────────────────

/// Per-source gap detection and recovery engine.
///
/// Sits between the raw WebSocket/FIX/ITCH stream and the order book updater.
/// When a sequence gap is detected, it freezes downstream updates, buffers
/// incoming messages, and drives the recovery state machine.
pub struct GapDetector {
    config: GapDetectorConfig,
    source_name: String,

    // ── Sequence tracking ─────────────────────────────────────────────
    /// Last successfully processed sequence ID (raw u64).
    last_seq: Option<u64>,
    /// Current feed status.
    status: FeedStatus,

    // ── Heartbeat ─────────────────────────────────────────────────────
    /// Timestamp of last received message (any type).
    last_message_at: Instant,
    /// Timestamp of last sent ping.
    last_ping_at: Instant,

    // ── Gap buffer (freeze zone) ──────────────────────────────────────
    /// Messages received while in gap-recovery mode, keyed by seq ID.
    /// BTreeMap keeps them sorted for efficient drain on recovery.
    gap_buffer: BTreeMap<u64, Envelope<MarketEvent>>,
    /// When the current gap was first detected.
    gap_detected_at: Option<Instant>,

    // ── Recovery state ────────────────────────────────────────────────
    /// Which strategy is currently active.
    active_recovery: Option<RecoveryStrategy>,
    /// Missing sequence IDs to retransmit.
    missing_seqs: Vec<u64>,

    // ── Audit log ─────────────────────────────────────────────────────
    /// Historical gap alerts for this source.
    gap_log: Vec<GapAlert>,
    /// Counters.
    total_gaps_detected: u64,
    total_messages_buffered: u64,
    total_messages_recovered: u64,
}

impl GapDetector {
    pub fn new(source_name: impl Into<String>, config: GapDetectorConfig) -> Self {
        let now = Instant::now();
        Self {
            config,
            source_name: source_name.into(),
            last_seq: None,
            status: FeedStatus::Live,
            last_message_at: now,
            last_ping_at: now,
            gap_buffer: BTreeMap::new(),
            gap_detected_at: None,
            active_recovery: None,
            missing_seqs: Vec::new(),
            gap_log: Vec::new(),
            total_gaps_detected: 0,
            total_messages_buffered: 0,
            total_messages_recovered: 0,
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // PUBLIC API
    // ═══════════════════════════════════════════════════════════════════

    /// Process an incoming envelope. Returns `Some(events)` to forward
    /// downstream, or `None` if the message was buffered during gap recovery.
    pub fn on_message(
        &mut self,
        envelope: Envelope<MarketEvent>,
    ) -> Option<Vec<Envelope<MarketEvent>>> {
        self.last_message_at = Instant::now();
        let incoming_seq = envelope.sequence_id.as_u64();

        match self.status {
            FeedStatus::Live => self.handle_live(envelope),
            FeedStatus::GapDetected | FeedStatus::Recovering => {
                // Buffer the message — UI is frozen
                self.gap_buffer.insert(incoming_seq, envelope);
                self.total_messages_buffered += 1;


                // Safety: flush if buffer gets too large
                if self.gap_buffer.len() > self.config.max_buffer_size {
                    warn!(
                        source = %self.source_name,
                        buffer_size = self.gap_buffer.len(),
                        "Gap buffer overflow — force-draining"
                    );
                    return Some(self.drain_buffer());
                }
                None
            }
            FeedStatus::Stale => {
                // Feed came back to life
                info!(source = %self.source_name, "Feed revived after stale state");
                self.status = FeedStatus::Live;
                self.last_seq = Some(incoming_seq);
                Some(vec![envelope])
            }
        }
    }

    /// Called on a timer tick (e.g., every 100ms) to check heartbeat
    /// and drive recovery timeouts.
    pub fn tick(&mut self) -> GapTickResult {
        let now = Instant::now();

        // Heartbeat check
        if now.duration_since(self.last_message_at) > self.config.heartbeat_timeout {
            if self.status == FeedStatus::Live {
                warn!(
                    source = %self.source_name,
                    timeout_ms = self.config.heartbeat_timeout.as_millis(),
                    "Heartbeat timeout — feed stale"
                );
                self.status = FeedStatus::Stale;
                return GapTickResult::FeedStale;
            }
        }

        // Ping interval
        if now.duration_since(self.last_ping_at) >= self.config.ping_interval {
            self.last_ping_at = now;
            return GapTickResult::SendPing;
        }

        // Recovery timeout
        if let Some(gap_start) = self.gap_detected_at {
            if now.duration_since(gap_start) > self.config.recovery_timeout {
                warn!(
                    source = %self.source_name,
                    "Recovery timeout — escalating"
                );
                return self.escalate_recovery();
            }
        }

        GapTickResult::Ok
    }

    /// Notify that a retransmission response was received (recovered msgs).
    pub fn on_retransmit_response(
        &mut self,
        recovered: Vec<Envelope<MarketEvent>>,
    ) -> Vec<Envelope<MarketEvent>> {
        info!(
            source = %self.source_name,
            count = recovered.len(),
            "Retransmission response received"
        );

        // Insert recovered messages into the buffer
        for env in recovered {
            self.gap_buffer.insert(env.sequence_id.as_u64(), env);
        }

        self.total_messages_recovered += self.missing_seqs.len() as u64;
        self.complete_recovery()
    }

    /// Notify that a full snapshot was applied (order book re-synced).
    pub fn on_snapshot_applied(&mut self) -> Vec<Envelope<MarketEvent>> {
        info!(
            source = %self.source_name,
            "Full snapshot applied — reconciling buffer"
        );
        self.complete_recovery()
    }

    /// Notify that failover to backup feed completed.
    pub fn on_failover_complete(&mut self, new_source: &str) -> Vec<Envelope<MarketEvent>> {
        info!(
            source = %self.source_name,
            new_source = new_source,
            "Failover complete — resuming from backup"
        );
        self.source_name = new_source.to_string();
        self.complete_recovery()
    }

    // ═══════════════════════════════════════════════════════════════════
    // ACCESSORS
    // ═══════════════════════════════════════════════════════════════════

    pub fn status(&self) -> FeedStatus { self.status }
    pub fn source_name(&self) -> &str { &self.source_name }
    pub fn last_sequence(&self) -> Option<u64> { self.last_seq }
    pub fn buffer_size(&self) -> usize { self.gap_buffer.len() }
    pub fn gap_log(&self) -> &[GapAlert] { &self.gap_log }
    pub fn total_gaps(&self) -> u64 { self.total_gaps_detected }
    pub fn is_frozen(&self) -> bool {
        matches!(self.status, FeedStatus::GapDetected | FeedStatus::Recovering)
    }

    /// Returns the missing sequence IDs for retransmission requests.
    pub fn missing_sequences(&self) -> &[u64] { &self.missing_seqs }

    /// Returns the active recovery strategy, if any.
    pub fn active_recovery(&self) -> Option<RecoveryStrategy> { self.active_recovery }

    // ═══════════════════════════════════════════════════════════════════
    // INTERNAL
    // ═══════════════════════════════════════════════════════════════════

    /// Handle a message in Live state — check for gaps.
    fn handle_live(
        &mut self,
        envelope: Envelope<MarketEvent>,
    ) -> Option<Vec<Envelope<MarketEvent>>> {
        let incoming_seq = envelope.sequence_id.as_u64();

        match self.last_seq {
            None => {
                // First message ever — establish baseline
                self.last_seq = Some(incoming_seq);
                Some(vec![envelope])
            }
            Some(prev_seq) => {
                let expected = prev_seq + 1;

                if incoming_seq == expected {
                    // Perfect — no gap
                    self.last_seq = Some(incoming_seq);
                    Some(vec![envelope])
                } else if incoming_seq > expected {
                    // GAP DETECTED
                    let gap_size = incoming_seq - expected;
                    self.enter_gap_state(expected, incoming_seq - 1, gap_size, envelope);
                    None
                } else {
                    // Duplicate or out-of-order (seq <= last_seq)
                    debug!(
                        source = %self.source_name,
                        incoming = incoming_seq,
                        last = prev_seq,
                        "Duplicate/old seq — dropping"
                    );
                    None
                }
            }
        }
    }

    /// Transition to GapDetected state and select recovery strategy.
    fn enter_gap_state(
        &mut self,
        gap_start: u64,
        gap_end: u64,
        gap_size: u64,
        first_buffered: Envelope<MarketEvent>,
    ) {
        self.total_gaps_detected += 1;
        self.status = FeedStatus::GapDetected;
        self.gap_detected_at = Some(Instant::now());

        // Buffer the first post-gap message
        self.gap_buffer
            .insert(first_buffered.sequence_id.as_u64(), first_buffered);
        self.total_messages_buffered += 1;

        // Compute missing seq IDs
        self.missing_seqs = (gap_start..=gap_end).collect();

        // Select recovery strategy based on gap size
        let strategy = if gap_size < self.config.retransmit_threshold {
            RecoveryStrategy::Retransmission
        } else if gap_size < self.config.snapshot_threshold {
            RecoveryStrategy::FullSnapshot
        } else {
            RecoveryStrategy::Failover
        };

        self.active_recovery = Some(strategy);
        self.status = FeedStatus::Recovering;

        warn!(
            source = %self.source_name,
            gap_start = gap_start,
            gap_end = gap_end,
            gap_size = gap_size,
            strategy = strategy.label(),
            "Gap detected — UI frozen, recovery initiated"
        );
    }

    /// Escalate recovery when timeout fires.
    fn escalate_recovery(&mut self) -> GapTickResult {
        match self.active_recovery {
            Some(RecoveryStrategy::Retransmission) => {
                // Retransmit timed out → try full snapshot
                warn!(source = %self.source_name, "Retransmit timeout → requesting snapshot");
                self.active_recovery = Some(RecoveryStrategy::FullSnapshot);
                self.gap_detected_at = Some(Instant::now()); // reset timer
                GapTickResult::RequestSnapshot
            }
            Some(RecoveryStrategy::FullSnapshot) => {
                // Snapshot timed out → failover
                warn!(source = %self.source_name, "Snapshot timeout → triggering failover");
                self.active_recovery = Some(RecoveryStrategy::Failover);
                self.gap_detected_at = Some(Instant::now());
                GapTickResult::TriggerFailover
            }
            Some(RecoveryStrategy::Failover) | None => {
                // Failover also timed out — force drain and resume
                error!(source = %self.source_name, "All recovery failed — force-resuming");
                let _drained = self.drain_buffer();
                self.status = FeedStatus::Live;
                self.gap_detected_at = None;
                self.active_recovery = None;
                GapTickResult::ForceResume
            }
        }
    }

    /// Complete recovery: log the gap, drain buffer, resume live state.
    fn complete_recovery(&mut self) -> Vec<Envelope<MarketEvent>> {
        let duration = self.gap_detected_at.map(|t| t.elapsed());
        let strategy = self.active_recovery.unwrap_or(RecoveryStrategy::Retransmission);

        // Log the gap alert
        if let (Some(&start), Some(&end)) = (self.missing_seqs.first(), self.missing_seqs.last()) {
            self.gap_log.push(GapAlert {
                detected_at: self.gap_detected_at.unwrap_or_else(Instant::now),
                source: self.source_name.clone(),
                gap_start: start,
                gap_end: end,
                gap_size: self.missing_seqs.len() as u64,
                strategy,
                recovered: true,
                recovery_duration: duration,
            });
        }

        info!(
            source = %self.source_name,
            strategy = strategy.label(),
            duration_ms = duration.map(|d| d.as_millis()).unwrap_or(0),
            buffer_size = self.gap_buffer.len(),
            "Recovery complete — draining buffer, unfreezing UI"
        );

        // Drain buffer in sequence order
        let events = self.drain_buffer();

        // Reset state
        self.status = FeedStatus::Live;
        self.gap_detected_at = None;
        self.active_recovery = None;
        self.missing_seqs.clear();

        events
    }

    /// Drain gap buffer → sorted Vec, update last_seq.
    fn drain_buffer(&mut self) -> Vec<Envelope<MarketEvent>> {
        let events: Vec<Envelope<MarketEvent>> = self
            .gap_buffer
            .values()
            .cloned()
            .collect();

        if let Some(last) = events.last() {
            self.last_seq = Some(last.sequence_id.as_u64());
        }

        self.gap_buffer.clear();
        events
    }
}

// ─── Tick Result ──────────────────────────────────────────────────────────────

/// Action requested by `tick()` — the caller must dispatch accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapTickResult {
    /// Nothing to do.
    Ok,
    /// Send a ping/heartbeat to the exchange.
    SendPing,
    /// Feed heartbeat timed out.
    FeedStale,
    /// Retransmit timed out — request a full snapshot instead.
    RequestSnapshot,
    /// Snapshot timed out — switch to backup data source.
    TriggerFailover,
    /// All recovery failed — forcibly resumed with potential data loss.
    ForceResume,
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use common::time::UnixNanos;

    fn make_envelope(seq: u64) -> Envelope<MarketEvent> {
        use common::events::{QuoteEvent, MarketEvent};
        use compact_str::CompactString;
        Envelope {
            ts_event: UnixNanos::now(),
            ts_init: UnixNanos::now(),
            sequence_id: common::time::SequenceId::new(seq),
            payload: MarketEvent::Quote(QuoteEvent {
                symbol: CompactString::new("BTCUSDT"),
                bid: 67000.0,
                bid_size: 1.0,
                ask: 67001.0,
                ask_size: 1.0,
            }),
        }
    }

    #[test]
    fn no_gap_in_sequential_messages() {
        let mut gd = GapDetector::new("test", GapDetectorConfig::default());
        assert!(gd.on_message(make_envelope(1)).is_some());
        assert!(gd.on_message(make_envelope(2)).is_some());
        assert!(gd.on_message(make_envelope(3)).is_some());
        assert_eq!(gd.status(), FeedStatus::Live);
        assert_eq!(gd.total_gaps(), 0);
    }

    #[test]
    fn small_gap_triggers_retransmission() {
        let mut gd = GapDetector::new("test", GapDetectorConfig::default());
        gd.on_message(make_envelope(1));
        // Skip 2..=11 (10 messages)
        let result = gd.on_message(make_envelope(12));
        assert!(result.is_none()); // buffered
        assert_eq!(gd.status(), FeedStatus::Recovering);
        assert_eq!(gd.active_recovery(), Some(RecoveryStrategy::Retransmission));
        assert_eq!(gd.missing_sequences().len(), 10);
        assert_eq!(gd.buffer_size(), 1);
    }

    #[test]
    fn large_gap_triggers_snapshot() {
        let config = GapDetectorConfig {
            retransmit_threshold: 50,
            ..Default::default()
        };
        let mut gd = GapDetector::new("test", config);
        gd.on_message(make_envelope(1));
        // Skip 2..=101 (100 messages)
        gd.on_message(make_envelope(102));
        assert_eq!(gd.active_recovery(), Some(RecoveryStrategy::FullSnapshot));
    }

    #[test]
    fn huge_gap_triggers_failover() {
        let config = GapDetectorConfig {
            retransmit_threshold: 50,
            snapshot_threshold: 500,
            ..Default::default()
        };
        let mut gd = GapDetector::new("test", config);
        gd.on_message(make_envelope(1));
        gd.on_message(make_envelope(1002));
        assert_eq!(gd.active_recovery(), Some(RecoveryStrategy::Failover));
    }

    #[test]
    fn recovery_drains_buffer() {
        let mut gd = GapDetector::new("test", GapDetectorConfig::default());
        gd.on_message(make_envelope(1));
        gd.on_message(make_envelope(5)); // gap: 2,3,4
        gd.on_message(make_envelope(6)); // also buffered
        gd.on_message(make_envelope(7)); // also buffered

        assert_eq!(gd.buffer_size(), 3);

        // Simulate retransmission of 2,3,4
        let recovered = vec![make_envelope(2), make_envelope(3), make_envelope(4)];
        let events = gd.on_retransmit_response(recovered);

        // Should drain all 6 messages (2,3,4 + 5,6,7)
        assert_eq!(events.len(), 6);
        assert_eq!(gd.status(), FeedStatus::Live);
        assert_eq!(gd.buffer_size(), 0);
        assert_eq!(gd.gap_log().len(), 1);
    }

    #[test]
    fn duplicate_seq_is_dropped() {
        let mut gd = GapDetector::new("test", GapDetectorConfig::default());
        gd.on_message(make_envelope(1));
        gd.on_message(make_envelope(2));
        let result = gd.on_message(make_envelope(2)); // duplicate
        assert!(result.is_none());
        assert_eq!(gd.status(), FeedStatus::Live); // no gap triggered
    }
}
