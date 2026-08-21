//! XDP multicast receiver with A/B line arbitration and gap detection.
//!
//! NYSE publishes each channel on two independent multicast groups carrying identical,
//! identically sequenced content over separate network paths. Listening to both and taking
//! whichever copy arrives first ("line arbitration") removes single-path packet loss
//! entirely, which is the difference between recovering a gap over a request server in
//! milliseconds and never noticing one. The second copy of each packet is a duplicate by
//! construction, so arbitration is exactly the duplicate/overlap logic the sequence tracker
//! already implements — the two feeds share one tracker.
//!
//! What this receiver does **not** do is silently continue past a gap. When a range cannot
//! be recovered by retransmission (`Message Unavailable`, or the daily request quota is
//! exhausted) the correct action is a full refresh, and callers are told so explicitly.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;

use crate::{NyseError, Result};
use exchange_core::gap::{GapAction, GapRange, SequenceTracker};

use super::packet::{Delivery, Packet};

/// One multicast group to listen on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedLine {
    pub group: SocketAddrV4,
    /// Interface to join on. Bind the feed NIC explicitly on a multi-homed trading host;
    /// `0.0.0.0` lets the OS pick, which is rarely the right path.
    pub interface: Ipv4Addr,
}

impl FeedLine {
    pub fn new(group: SocketAddrV4) -> Self {
        Self {
            group,
            interface: Ipv4Addr::UNSPECIFIED,
        }
    }

    pub fn on_interface(group: SocketAddrV4, interface: Ipv4Addr) -> Self {
        Self { group, interface }
    }
}

/// Receiver configuration for one channel.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// Feed identifier from the product's client specification. Required on every
    /// retransmission and refresh request.
    pub product_id: u8,
    /// Multicast channel number within the product.
    pub channel_id: u8,
    /// Primary ("A") line.
    pub line_a: FeedLine,
    /// Redundant ("B") line, if subscribed. Running only one line is supported but means
    /// every lost packet becomes a request-server round trip.
    pub line_b: Option<FeedLine>,
    /// Maximum sequence span in one retransmission request. NYSE rejects wider requests
    /// with `Status = 3`; the published threshold has been 10,000.
    pub max_request_span: u32,
    /// Minimum wait between retransmission attempts for the same range.
    pub rerequest_interval: Duration,
    /// Escalate to a full refresh after this many failed retransmission attempts.
    pub max_rerequest_attempts: u32,
}

impl ChannelConfig {
    pub fn new(product_id: u8, channel_id: u8, line_a: FeedLine) -> Self {
        Self {
            product_id,
            channel_id,
            line_a,
            line_b: None,
            max_request_span: 10_000,
            rerequest_interval: Duration::from_millis(25),
            max_rerequest_attempts: 3,
        }
    }

    pub fn with_line_b(mut self, line_b: FeedLine) -> Self {
        self.line_b = Some(line_b);
        self
    }

    pub fn validate(&self) -> Result<()> {
        for (name, line) in [("A", Some(self.line_a)), ("B", self.line_b)] {
            let Some(line) = line else { continue };
            if !line.group.ip().is_multicast() {
                return Err(NyseError::NotConfigured(format!(
                    "line {name} address {} is not multicast",
                    line.group.ip()
                )));
            }
        }
        if let Some(b) = self.line_b {
            if b.group == self.line_a.group {
                return Err(NyseError::NotConfigured(
                    "lines A and B are the same group; redundancy would be an illusion".into(),
                ));
            }
        }
        if self.max_request_span == 0 || self.max_request_span > 10_000 {
            return Err(NyseError::NotConfigured(format!(
                "max_request_span {} is outside the 1..=10,000 the Request Server accepts",
                self.max_request_span
            )));
        }
        Ok(())
    }
}

/// Which line a datagram arrived on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    A,
    B,
}

/// What the receiver decided about the packet just read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelEvent {
    /// Process this packet, skipping `skip` messages at its front.
    Data {
        line: Line,
        sequence: u32,
        count: u8,
        skip: u32,
    },
    /// Already seen on the other line, or a retransmission we had already filled.
    Duplicate {
        line: Line,
    },
    Heartbeat {
        line: Line,
        next_sequence: u32,
    },
    /// Sequence numbering restarted; drop all channel state.
    SequenceReset {
        line: Line,
        sequence: u32,
    },
    /// A recovery packet (retransmission or refresh) — process it, but it does not advance
    /// the live watermark.
    Recovery {
        line: Line,
        delivery: Delivery,
        sequence: u32,
        count: u8,
    },
}

/// Per-channel statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelStats {
    pub datagrams_a: u64,
    pub datagrams_b: u64,
    /// Packets first seen on line A, and on line B respectively. A healthy pair is close to
    /// even; a lopsided split means one path is consistently slower.
    pub won_by_a: u64,
    pub won_by_b: u64,
    pub duplicates: u64,
    pub gaps: u64,
    pub resets: u64,
    pub refresh_required: u64,
}

/// A live XDP channel: one or two multicast sockets sharing a sequence tracker.
pub struct ChannelReceiver {
    config: ChannelConfig,
    socket_a: UdpSocket,
    socket_b: Option<UdpSocket>,
    buf_a: Vec<u8>,
    buf_b: Vec<u8>,
    len: usize,
    last_line: Line,
    tracker: SequenceTracker,
    stats: ChannelStats,
    last_rerequest: Instant,
}

impl ChannelReceiver {
    /// Bind and join. Both lines use the same port as their group, as NYSE publishes them.
    pub async fn bind(config: ChannelConfig) -> Result<Self> {
        config.validate()?;

        let socket_a = join(config.line_a).await?;
        let socket_b = match config.line_b {
            Some(line) => Some(join(line).await?),
            None => None,
        };

        let tracker = SequenceTracker::new(config.max_request_span as u64);
        Ok(Self {
            config,
            socket_a,
            socket_b,
            buf_a: vec![0u8; 2048],
            buf_b: vec![0u8; 2048],
            len: 0,
            last_line: Line::A,
            tracker,
            stats: ChannelStats::default(),
            last_rerequest: Instant::now(),
        })
    }

    pub fn config(&self) -> &ChannelConfig {
        &self.config
    }

    pub fn stats(&self) -> ChannelStats {
        self.stats
    }

    pub fn tracker(&self) -> &SequenceTracker {
        &self.tracker
    }

    /// The datagram most recently accepted, for the caller to decode.
    pub fn datagram(&self) -> &[u8] {
        match self.last_line {
            Line::A => &self.buf_a[..self.len],
            Line::B => &self.buf_b[..self.len],
        }
    }

    /// Await the next datagram from either line and classify it.
    pub async fn recv(&mut self) -> Result<ChannelEvent> {
        let line = if let Some(sock_b) = self.socket_b.as_ref() {
            tokio::select! {
                r = self.socket_a.recv(&mut self.buf_a) => { self.len = r?; Line::A }
                r = sock_b.recv(&mut self.buf_b) => { self.len = r?; Line::B }
            }
        } else {
            self.len = self.socket_a.recv(&mut self.buf_a).await?;
            Line::A
        };
        self.last_line = line;
        match line {
            Line::A => self.stats.datagrams_a += 1,
            Line::B => self.stats.datagrams_b += 1,
        }

        let header = Packet::parse(self.datagram())?.header;

        if header.is_heartbeat() {
            if let Some(gap) = self.tracker.observe_heartbeat(header.sequence as u64) {
                self.stats.gaps += 1;
                tracing::warn!(
                    target: "nyse::xdp",
                    channel = self.config.channel_id,
                    from = gap.from, to = gap.to,
                    "gap revealed by heartbeat"
                );
            }
            return Ok(ChannelEvent::Heartbeat {
                line,
                next_sequence: header.sequence,
            });
        }

        // A Sequence Number Reset arrives in its own packet with SeqNum 1; the delivery
        // flag distinguishes a genuine restart from a stale low sequence number.
        if matches!(
            header.delivery,
            Delivery::SequenceReset | Delivery::Failover
        ) {
            self.tracker.observe_reset(header.sequence as u64);
            self.stats.resets += 1;
            return Ok(ChannelEvent::SequenceReset {
                line,
                sequence: header.sequence,
            });
        }

        // Recovery traffic replays already-numbered messages and must not move the live
        // watermark; the caller applies it and then calls `note_recovered`.
        if header.delivery.is_recovery() {
            return Ok(ChannelEvent::Recovery {
                line,
                delivery: header.delivery,
                sequence: header.sequence,
                count: header.message_count,
            });
        }

        Ok(
            match self
                .tracker
                .observe(header.sequence as u64, header.message_count as u32)
            {
                GapAction::InOrder | GapAction::Reset => {
                    match line {
                        Line::A => self.stats.won_by_a += 1,
                        Line::B => self.stats.won_by_b += 1,
                    }
                    ChannelEvent::Data {
                        line,
                        sequence: header.sequence,
                        count: header.message_count,
                        skip: 0,
                    }
                }
                GapAction::PartialOverlap { skip } => ChannelEvent::Data {
                    line,
                    sequence: header.sequence,
                    count: header.message_count,
                    skip,
                },
                GapAction::Gap { from, to } => {
                    self.stats.gaps += 1;
                    tracing::warn!(
                        target: "nyse::xdp",
                        channel = self.config.channel_id,
                        from, to,
                        "sequence gap on both lines"
                    );
                    match line {
                        Line::A => self.stats.won_by_a += 1,
                        Line::B => self.stats.won_by_b += 1,
                    }
                    ChannelEvent::Data {
                        line,
                        sequence: header.sequence,
                        count: header.message_count,
                        skip: 0,
                    }
                }
                GapAction::Duplicate => {
                    self.stats.duplicates += 1;
                    ChannelEvent::Duplicate { line }
                }
            },
        )
    }

    /// Ranges that need recovery right now, rate limited by `rerequest_interval`.
    ///
    /// Ranges that have exceeded `max_rerequest_attempts` are returned separately: those
    /// have to be recovered with a full refresh, and the caller must not keep retrying a
    /// retransmission that is not going to arrive.
    pub fn pending_recovery(&mut self) -> Recovery {
        if !self.tracker.has_gaps()
            || self.last_rerequest.elapsed() < self.config.rerequest_interval
        {
            return Recovery::default();
        }
        self.last_rerequest = Instant::now();

        let mut retransmit = Vec::new();
        let mut refresh_required = Vec::new();
        for range in self.tracker.pending_requests() {
            if range.attempts > self.config.max_rerequest_attempts {
                refresh_required.push(range);
            } else {
                retransmit.push(range);
            }
        }
        if !refresh_required.is_empty() {
            self.stats.refresh_required += refresh_required.len() as u64;
        }
        Recovery {
            retransmit,
            refresh_required,
        }
    }

    /// Mark a range as recovered.
    pub fn note_recovered(&mut self, from: u32, to: u32) {
        self.tracker.fill(from as u64, to as u64);
    }

    /// Resume the live channel at a known sequence, after applying a refresh snapshot.
    pub fn resume_at(&mut self, sequence: u32) {
        self.tracker.observe_reset(sequence as u64);
    }
}

/// Outstanding recovery work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Ask the Request Server to retransmit these.
    pub retransmit: Vec<GapRange>,
    /// Retransmission has been tried enough times; only a full refresh will close these.
    pub refresh_required: Vec<GapRange>,
}

impl Recovery {
    pub fn is_empty(&self) -> bool {
        self.retransmit.is_empty() && self.refresh_required.is_empty()
    }
}

async fn join(line: FeedLine) -> Result<UdpSocket> {
    let socket =
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, line.group.port())).await?;
    socket.join_multicast_v4(*line.group.ip(), line.interface)?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(s: &str) -> SocketAddrV4 {
        s.parse().unwrap()
    }

    #[test]
    fn config_requires_multicast_addresses() {
        let cfg = ChannelConfig::new(1, 1, FeedLine::new(group("10.0.0.1:11111")));
        assert!(cfg.validate().is_err());
        let cfg = ChannelConfig::new(1, 1, FeedLine::new(group("224.0.59.1:11111")));
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn config_rejects_identical_a_and_b_lines() {
        let a = FeedLine::new(group("224.0.59.1:11111"));
        let cfg = ChannelConfig::new(1, 1, a).with_line_b(a);
        assert!(
            cfg.validate().is_err(),
            "identical lines give no path redundancy"
        );
        let cfg = ChannelConfig::new(1, 1, a).with_line_b(FeedLine::new(group("224.0.59.2:11111")));
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn config_rejects_a_request_span_the_server_would_refuse() {
        let mut cfg = ChannelConfig::new(1, 1, FeedLine::new(group("224.0.59.1:11111")));
        cfg.max_request_span = 50_000;
        assert!(cfg.validate().is_err());
        cfg.max_request_span = 0;
        assert!(cfg.validate().is_err());
        cfg.max_request_span = 10_000;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn recovery_is_empty_when_there_is_nothing_to_do() {
        assert!(Recovery::default().is_empty());
        assert!(!Recovery {
            retransmit: vec![GapRange {
                from: 1,
                to: 2,
                attempts: 1
            }],
            refresh_required: vec![],
        }
        .is_empty());
    }
}
