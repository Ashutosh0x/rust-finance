//! Sequence-gap detection and retransmission planning.
//!
//! Both venues sequence per channel and both expect the client to notice gaps itself:
//!
//! * MoldUDP64 puts the sequence number of the *first message* in each downstream packet
//!   header and numbers the rest of the packet implicitly, so the next expected sequence is
//!   `header.sequence + header.message_count`.
//! * XDP does the same thing with `SeqNum` + `NumberMsgs` in the 16-byte packet header, and
//!   a heartbeat (`DeliveryFlag == 1`, `NumberMsgs == 0`) carries the next expected number
//!   without consuming one.
//!
//! The tracker is transport-agnostic: it takes `(first_sequence, message_count)` and tells
//! the caller what to do.

use std::collections::BTreeMap;

/// What the caller should do with a freshly received packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapAction {
    /// Contiguous with what we already have — process it.
    InOrder,
    /// Entirely behind the watermark (a duplicate, e.g. the B-side of an A/B feed pair, or
    /// a retransmission we already filled). Drop it.
    Duplicate,
    /// Partially behind the watermark: skip the first `skip` messages, process the rest.
    PartialOverlap { skip: u32 },
    /// A range is missing. Process this packet, but request `[from, to]`.
    Gap { from: u64, to: u64 },
    /// The sequence went backwards past a reset (start of day / publisher failover).
    Reset,
}

/// Tracks the next expected sequence number on one channel.
#[derive(Debug, Clone)]
pub struct SequenceTracker {
    next_expected: u64,
    started: bool,
    /// Outstanding gaps, keyed by first missing sequence.
    outstanding: BTreeMap<u64, GapRange>,
    max_request_span: u64,
    stats: GapStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapRange {
    pub from: u64,
    pub to: u64,
    /// How many retransmission requests we have already sent for this range.
    pub attempts: u32,
}

impl GapRange {
    pub fn len(&self) -> u64 {
        self.to.saturating_sub(self.from) + 1
    }

    pub fn is_empty(&self) -> bool {
        self.to < self.from
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GapStats {
    pub packets_seen: u64,
    pub messages_seen: u64,
    pub duplicates: u64,
    pub gaps_detected: u64,
    pub messages_missing: u64,
    pub resets: u64,
}

impl SequenceTracker {
    /// `max_request_span` caps how many messages a single retransmission request may ask
    /// for. NYSE rejects requests wider than its published threshold with
    /// `Status = 3 (maximum sequence range)`, so gaps are split before they are sent.
    pub fn new(max_request_span: u64) -> Self {
        Self {
            next_expected: 0,
            started: false,
            outstanding: BTreeMap::new(),
            max_request_span: max_request_span.max(1),
            stats: GapStats::default(),
        }
    }

    /// Start (or restart) the tracker at a known sequence, e.g. from a SoupBinTCP Login
    /// Accepted packet or an XDP Sequence Number Reset message.
    pub fn reset_to(&mut self, next_expected: u64) {
        self.next_expected = next_expected;
        self.started = true;
        self.outstanding.clear();
        self.stats.resets += 1;
    }

    #[inline]
    pub fn next_expected(&self) -> u64 {
        self.next_expected
    }

    #[inline]
    pub fn stats(&self) -> GapStats {
        self.stats
    }

    #[inline]
    pub fn outstanding_gaps(&self) -> impl Iterator<Item = &GapRange> {
        self.outstanding.values()
    }

    #[inline]
    pub fn has_gaps(&self) -> bool {
        !self.outstanding.is_empty()
    }

    /// Note a heartbeat that advertises the next expected sequence without carrying data.
    ///
    /// A heartbeat ahead of our watermark is itself gap evidence: MoldUDP64 heartbeats
    /// "contain the next expected Sequence Number", so during a quiet period this is how a
    /// tail-end loss is discovered at all.
    pub fn observe_heartbeat(&mut self, advertised_next: u64) -> Option<GapRange> {
        if !self.started {
            self.next_expected = advertised_next;
            self.started = true;
            return None;
        }
        if advertised_next > self.next_expected {
            let range = GapRange {
                from: self.next_expected,
                to: advertised_next - 1,
                attempts: 0,
            };
            self.record_gap(range);
            self.next_expected = advertised_next;
            return Some(range);
        }
        None
    }

    /// Handle an explicit sequence reset from the transport.
    ///
    /// Reset is never inferred from the sequence numbers alone, because "sequence 1 while
    /// we are at 900" is indistinguishable from a stale duplicate. Each transport knows the
    /// real signal and calls this: MoldUDP64 when the 10-byte session id in the downstream
    /// header changes, XDP when it receives a Sequence Number Reset message (type 1, packet
    /// `DeliveryFlag` 12 at startup or 10 on publisher failover).
    pub fn observe_reset(&mut self, next_expected: u64) -> GapAction {
        self.next_expected = next_expected;
        self.started = true;
        self.outstanding.clear();
        self.stats.resets += 1;
        GapAction::Reset
    }

    /// Observe a data packet carrying `count` messages starting at `first`.
    pub fn observe(&mut self, first: u64, count: u32) -> GapAction {
        self.stats.packets_seen += 1;
        if count == 0 {
            return GapAction::InOrder;
        }
        let last = first + count as u64 - 1;

        if !self.started {
            self.started = true;
            self.next_expected = last + 1;
            self.stats.messages_seen += count as u64;
            return GapAction::InOrder;
        }

        if last < self.next_expected {
            self.stats.duplicates += 1;
            self.fill(first, last);
            return GapAction::Duplicate;
        }

        if first < self.next_expected {
            let skip = (self.next_expected - first) as u32;
            self.fill(first, self.next_expected - 1);
            self.next_expected = last + 1;
            self.stats.messages_seen += (count - skip) as u64;
            return GapAction::PartialOverlap { skip };
        }

        if first > self.next_expected {
            let range = GapRange {
                from: self.next_expected,
                to: first - 1,
                attempts: 0,
            };
            self.record_gap(range);
            self.next_expected = last + 1;
            self.stats.messages_seen += count as u64;
            return GapAction::Gap {
                from: range.from,
                to: range.to,
            };
        }

        self.next_expected = last + 1;
        self.stats.messages_seen += count as u64;
        GapAction::InOrder
    }

    /// Requests to send now, each no wider than `max_request_span`.
    ///
    /// Increments the attempt counter on every returned range so a caller can back off or
    /// escalate to a full refresh after N tries.
    pub fn pending_requests(&mut self) -> Vec<GapRange> {
        let mut out = Vec::new();
        for range in self.outstanding.values_mut() {
            range.attempts += 1;
            let mut from = range.from;
            while from <= range.to {
                let to = (from + self.max_request_span - 1).min(range.to);
                out.push(GapRange {
                    from,
                    to,
                    attempts: range.attempts,
                });
                from = to + 1;
            }
        }
        out
    }

    /// Mark `[from, to]` as recovered (retransmission arrived).
    pub fn fill(&mut self, from: u64, to: u64) {
        let keys: Vec<u64> = self
            .outstanding
            .range(..=to)
            .filter(|(_, r)| r.to >= from)
            .map(|(k, _)| *k)
            .collect();

        for key in keys {
            let Some(range) = self.outstanding.remove(&key) else {
                continue;
            };
            // Anything before the filled window is still missing.
            if range.from < from {
                self.outstanding.insert(
                    range.from,
                    GapRange {
                        from: range.from,
                        to: from - 1,
                        attempts: range.attempts,
                    },
                );
            }
            // …and anything after it.
            if range.to > to {
                self.outstanding.insert(
                    to + 1,
                    GapRange {
                        from: to + 1,
                        to: range.to,
                        attempts: range.attempts,
                    },
                );
            }
        }
    }

    fn record_gap(&mut self, range: GapRange) {
        if range.is_empty() {
            return;
        }
        self.stats.gaps_detected += 1;
        self.stats.messages_missing += range.len();
        self.outstanding.insert(range.from, range);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_packet_establishes_the_watermark() {
        let mut t = SequenceTracker::new(1000);
        assert_eq!(t.observe(500, 3), GapAction::InOrder);
        assert_eq!(t.next_expected(), 503);
    }

    #[test]
    fn contiguous_packets_stay_in_order() {
        let mut t = SequenceTracker::new(1000);
        t.observe(1, 10);
        assert_eq!(t.observe(11, 5), GapAction::InOrder);
        assert!(!t.has_gaps());
        assert_eq!(t.stats().messages_seen, 15);
    }

    #[test]
    fn a_missing_range_is_detected_and_recorded() {
        let mut t = SequenceTracker::new(1000);
        t.observe(1, 10);
        assert_eq!(t.observe(21, 5), GapAction::Gap { from: 11, to: 20 });
        assert_eq!(t.next_expected(), 26);
        assert_eq!(t.stats().messages_missing, 10);
    }

    #[test]
    fn duplicate_packets_from_a_redundant_feed_are_dropped() {
        let mut t = SequenceTracker::new(1000);
        t.observe(1, 10);
        assert_eq!(t.observe(1, 10), GapAction::Duplicate);
        assert_eq!(t.next_expected(), 11);
        assert_eq!(t.stats().duplicates, 1);
    }

    #[test]
    fn partial_overlap_reports_how_many_messages_to_skip() {
        let mut t = SequenceTracker::new(1000);
        t.observe(1, 10); // next expected 11
        assert_eq!(t.observe(8, 6), GapAction::PartialOverlap { skip: 3 });
        assert_eq!(t.next_expected(), 14);
    }

    #[test]
    fn an_explicit_reset_clears_outstanding_gaps_and_rebases() {
        let mut t = SequenceTracker::new(1000);
        t.observe(900, 10);
        t.observe(920, 1); // opens a gap
        assert!(t.has_gaps());
        assert_eq!(t.observe_reset(1), GapAction::Reset);
        assert_eq!(t.next_expected(), 1);
        assert!(!t.has_gaps());
        assert_eq!(t.observe(1, 5), GapAction::InOrder);
    }

    #[test]
    fn a_stale_low_sequence_is_a_duplicate_not_an_inferred_reset() {
        // Sequence 1 arriving while we are at 910 is ambiguous on the wire, so the tracker
        // treats it as stale and requires the transport to signal a real reset.
        let mut t = SequenceTracker::new(1000);
        t.observe(900, 10);
        assert_eq!(t.observe(1, 5), GapAction::Duplicate);
        assert_eq!(t.next_expected(), 910);
    }

    #[test]
    fn heartbeat_reveals_a_tail_loss_during_a_quiet_period() {
        let mut t = SequenceTracker::new(1000);
        t.observe(1, 10);
        let gap = t.observe_heartbeat(15).expect("heartbeat is ahead of us");
        assert_eq!((gap.from, gap.to), (11, 14));
        assert_eq!(t.next_expected(), 15);
    }

    #[test]
    fn wide_gaps_are_split_to_respect_the_request_limit() {
        let mut t = SequenceTracker::new(10);
        t.observe(1, 1);
        t.observe(100, 1); // gap 2..=99, 98 messages
        let reqs = t.pending_requests();
        assert_eq!(reqs.len(), 10);
        assert_eq!((reqs[0].from, reqs[0].to), (2, 11));
        assert_eq!((reqs[9].from, reqs[9].to), (92, 99));
        assert!(reqs.iter().all(|r| r.len() <= 10));
    }

    #[test]
    fn attempts_increment_so_callers_can_escalate() {
        let mut t = SequenceTracker::new(1000);
        t.observe(1, 1);
        t.observe(10, 1);
        assert_eq!(t.pending_requests()[0].attempts, 1);
        assert_eq!(t.pending_requests()[0].attempts, 2);
    }

    #[test]
    fn filling_the_middle_of_a_gap_leaves_both_edges_outstanding() {
        let mut t = SequenceTracker::new(1000);
        t.observe(1, 1);
        t.observe(100, 1); // gap 2..=99
        t.fill(50, 60);
        let gaps: Vec<_> = t.outstanding_gaps().copied().collect();
        assert_eq!(gaps.len(), 2);
        assert_eq!((gaps[0].from, gaps[0].to), (2, 49));
        assert_eq!((gaps[1].from, gaps[1].to), (61, 99));
    }

    #[test]
    fn filling_a_gap_entirely_clears_it() {
        let mut t = SequenceTracker::new(1000);
        t.observe(1, 1);
        t.observe(10, 1);
        t.fill(2, 9);
        assert!(!t.has_gaps());
    }
}
