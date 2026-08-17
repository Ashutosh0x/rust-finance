//! MoldUDP64 — Nasdaq's one-transmitter-to-many-listeners multicast session layer.
//!
//! Mold is what carries ITCH into a co-location cabinet. Every downstream packet is
//! transmitted exactly once regardless of how many firms are listening, so recovery is the
//! listener's problem: detect the gap, unicast a Request Packet to a re-request server, and
//! the response comes back as an ordinary Downstream Packet on the same socket.
//!
//! Wire layout (all integers big-endian):
//!
//! ```text
//!   Downstream Packet
//!   ┌────────────────────┬──────────────────┬───────────────┐
//!   │ Session   10 bytes │ Sequence 8 bytes │ Count 2 bytes │   (20-byte header)
//!   ├────────────────────┴──────────────────┴───────────────┤
//!   │ [len u16][message …] [len u16][message …] …           │
//!   └───────────────────────────────────────────────────────┘
//! ```
//!
//! Two counts are special: `0` is a heartbeat, and `0xFFFF` marks end of session. Both
//! carry the *next expected* sequence number, which is what makes tail-end loss detectable
//! during a quiet period.
//!
//! The `Session` field matters more than it looks: it is the only signal that the publisher
//! has restarted numbering. A sequence number going backwards is not sufficient evidence,
//! so the receiver keys resets on the session id changing.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::{NasdaqError, Result};
use exchange_core::gap::{GapAction, GapRange, SequenceTracker};
use exchange_core::wire::Writer;

/// Fixed size of the downstream packet header.
pub const HEADER_LEN: usize = 20;
/// `Message Count` value denoting a heartbeat.
pub const COUNT_HEARTBEAT: u16 = 0;
/// `Message Count` value denoting end of session.
pub const COUNT_END_OF_SESSION: u16 = 0xFFFF;

/// A 10-character MoldUDP64 session identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SessionId(pub [u8; 10]);

impl SessionId {
    pub fn from_str_padded(s: &str) -> Self {
        let mut out = [b' '; 10];
        for (slot, byte) in out.iter_mut().zip(s.bytes()) {
            *slot = byte;
        }
        Self(out)
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("<non-ascii>")
    }

    pub fn trimmed(&self) -> &str {
        self.as_str().trim()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.trimmed())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoldHeader {
    pub session: SessionId,
    /// Sequence number of the *first* message in the packet; the rest are implicit.
    pub sequence: u64,
    pub message_count: u16,
}

impl MoldHeader {
    pub fn is_heartbeat(&self) -> bool {
        self.message_count == COUNT_HEARTBEAT
    }

    pub fn is_end_of_session(&self) -> bool {
        self.message_count == COUNT_END_OF_SESSION
    }

    /// Number of messages actually carried (heartbeat and end-of-session carry none).
    pub fn payload_count(&self) -> u16 {
        if self.is_heartbeat() || self.is_end_of_session() {
            0
        } else {
            self.message_count
        }
    }
}

/// A parsed downstream packet: header plus a lazy iterator over its message blocks.
#[derive(Debug, Clone)]
pub struct MoldPacket<'a> {
    pub header: MoldHeader,
    body: &'a [u8],
}

impl<'a> MoldPacket<'a> {
    /// Parse a datagram.
    pub fn parse(datagram: &'a [u8]) -> Result<Self> {
        if datagram.len() < HEADER_LEN {
            return Err(NasdaqError::Protocol(format!(
                "MoldUDP64 datagram of {} bytes is shorter than the {HEADER_LEN}-byte header",
                datagram.len()
            )));
        }
        let mut session = [0u8; 10];
        session.copy_from_slice(&datagram[..10]);
        let sequence = u64::from_be_bytes([
            datagram[10],
            datagram[11],
            datagram[12],
            datagram[13],
            datagram[14],
            datagram[15],
            datagram[16],
            datagram[17],
        ]);
        let message_count = u16::from_be_bytes([datagram[18], datagram[19]]);

        Ok(Self {
            header: MoldHeader {
                session: SessionId(session),
                sequence,
                message_count,
            },
            body: &datagram[HEADER_LEN..],
        })
    }

    /// Iterate the message blocks, yielding each higher-level (ITCH) message.
    pub fn messages(&self) -> MessageIter<'a> {
        MessageIter {
            body: self.body,
            offset: 0,
            remaining: self.header.payload_count(),
        }
    }

    /// Collect and validate every message block, failing if the packet is malformed.
    ///
    /// Use this when correctness matters more than speed (recovery, capture validation);
    /// the streaming [`Self::messages`] iterator simply stops at the first bad block.
    pub fn validate(&self) -> Result<Vec<&'a [u8]>> {
        let mut out = Vec::with_capacity(self.header.payload_count() as usize);
        let mut offset = 0usize;
        for i in 0..self.header.payload_count() {
            if offset + 2 > self.body.len() {
                return Err(NasdaqError::Protocol(format!(
                    "MoldUDP64 packet claims {} messages but ran out of bytes at message {i}",
                    self.header.message_count
                )));
            }
            let len = u16::from_be_bytes([self.body[offset], self.body[offset + 1]]) as usize;
            offset += 2;
            if offset + len > self.body.len() {
                return Err(NasdaqError::Protocol(format!(
                    "MoldUDP64 message {i} declares {len} bytes but only {} remain",
                    self.body.len() - offset
                )));
            }
            out.push(&self.body[offset..offset + len]);
            offset += len;
        }
        Ok(out)
    }
}

/// Iterator over the message blocks of a downstream packet.
pub struct MessageIter<'a> {
    body: &'a [u8],
    offset: usize,
    remaining: u16,
}

impl<'a> Iterator for MessageIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.remaining == 0 || self.offset + 2 > self.body.len() {
            return None;
        }
        let len = u16::from_be_bytes([self.body[self.offset], self.body[self.offset + 1]]) as usize;
        self.offset += 2;
        if self.offset + len > self.body.len() {
            self.remaining = 0;
            return None;
        }
        let msg = &self.body[self.offset..self.offset + len];
        self.offset += len;
        self.remaining -= 1;
        Some(msg)
    }
}

/// Build a downstream packet. Used by the capture replayer and by the tests.
pub fn encode_packet(session: SessionId, sequence: u64, messages: &[&[u8]]) -> Vec<u8> {
    let mut w =
        Writer::with_capacity(HEADER_LEN + messages.iter().map(|m| m.len() + 2).sum::<usize>());
    w.raw(&session.0)
        .be_u64(sequence)
        .be_u16(messages.len() as u16);
    for m in messages {
        w.be_u16(m.len() as u16).raw(m);
    }
    w.into_vec()
}

/// Build a heartbeat carrying the next expected sequence number.
pub fn encode_heartbeat(session: SessionId, next_sequence: u64) -> Vec<u8> {
    let mut w = Writer::with_capacity(HEADER_LEN);
    w.raw(&session.0)
        .be_u64(next_sequence)
        .be_u16(COUNT_HEARTBEAT);
    w.into_vec()
}

/// Build an end-of-session packet.
pub fn encode_end_of_session(session: SessionId, next_sequence: u64) -> Vec<u8> {
    let mut w = Writer::with_capacity(HEADER_LEN);
    w.raw(&session.0)
        .be_u64(next_sequence)
        .be_u16(COUNT_END_OF_SESSION);
    w.into_vec()
}

/// Build a Request Packet for retransmission of `count` messages from `sequence`.
///
/// Sent by unicast to a re-request server. If the requested messages do not all fit in one
/// UDP datagram, only those that fit come back and the rest must be asked for again.
pub fn encode_request(session: SessionId, sequence: u64, count: u16) -> Vec<u8> {
    let mut w = Writer::with_capacity(HEADER_LEN);
    w.raw(&session.0).be_u64(sequence).be_u16(count);
    w.into_vec()
}

/// Receiver configuration.
#[derive(Debug, Clone)]
pub struct MoldConfig {
    /// Multicast group and port to join, e.g. `233.54.12.111:26477`. Nasdaq publishes the
    /// current addresses per feed; there is deliberately no default.
    pub group: SocketAddrV4,
    /// Local interface to join on. `0.0.0.0` lets the OS choose, which on a multi-homed
    /// trading host is rarely what you want — bind the feed NIC explicitly.
    pub interface: Ipv4Addr,
    /// Re-request servers, tried in order.
    pub request_servers: Vec<SocketAddr>,
    /// Cap on a single retransmission request; larger gaps are split.
    pub max_request_span: u16,
    /// Socket receive buffer. A burst on the open can exceed the OS default in
    /// milliseconds, and a dropped datagram costs a round trip to recover.
    pub recv_buffer_bytes: usize,
    /// How long to wait between retransmission attempts for the same range.
    pub rerequest_interval: Duration,
}

impl MoldConfig {
    pub fn new(group: SocketAddrV4) -> Self {
        Self {
            group,
            interface: Ipv4Addr::UNSPECIFIED,
            request_servers: Vec::new(),
            max_request_span: 1000,
            recv_buffer_bytes: 16 * 1024 * 1024,
            rerequest_interval: Duration::from_millis(20),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !self.group.ip().is_multicast() {
            return Err(NasdaqError::NotConfigured(format!(
                "{} is not a multicast address; MoldUDP64 feeds are multicast",
                self.group.ip()
            )));
        }
        if self.max_request_span == 0 {
            return Err(NasdaqError::NotConfigured(
                "max_request_span must be at least 1".into(),
            ));
        }
        Ok(())
    }
}

/// What the receiver produces for each datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoldEvent {
    /// Messages are ready in the receive buffer; iterate them with [`MoldPacket::messages`].
    Data {
        sequence: u64,
        count: u16,
        /// Messages at the front of the packet to skip because they were already processed.
        skip: u32,
    },
    /// A duplicate packet (typical on an A/B redundant feed pair). Nothing to do.
    Duplicate,
    Heartbeat,
    EndOfSession,
    /// The publisher started a new session; all prior state is stale.
    SessionChanged {
        session: SessionId,
    },
}

/// A MoldUDP64 multicast listener with gap detection and re-request.
pub struct MoldReceiver {
    socket: UdpSocket,
    config: MoldConfig,
    buf: Vec<u8>,
    len: usize,
    session: Option<SessionId>,
    tracker: SequenceTracker,
    last_rerequest: std::time::Instant,
}

impl MoldReceiver {
    /// Bind the socket and join the multicast group.
    pub async fn bind(config: MoldConfig) -> Result<Self> {
        config.validate()?;

        // Bind to the group port on the wildcard address so several feeds can share the
        // port with SO_REUSEADDR semantics provided by the platform.
        let socket = UdpSocket::bind(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            config.group.port(),
        ))
        .await?;
        socket.join_multicast_v4(*config.group.ip(), config.interface)?;

        let tracker = SequenceTracker::new(config.max_request_span as u64);
        Ok(Self {
            socket,
            buf: vec![0u8; 65_536],
            len: 0,
            config,
            session: None,
            tracker,
            last_rerequest: std::time::Instant::now(),
        })
    }

    pub fn tracker(&self) -> &SequenceTracker {
        &self.tracker
    }

    pub fn session(&self) -> Option<SessionId> {
        self.session
    }

    /// The datagram most recently received.
    pub fn packet(&self) -> Result<MoldPacket<'_>> {
        MoldPacket::parse(&self.buf[..self.len])
    }

    /// Receive and classify the next datagram.
    ///
    /// After a `Data` event the caller reads the messages with [`Self::packet`]; the buffer
    /// is not overwritten until the next call.
    pub async fn recv(&mut self) -> Result<MoldEvent> {
        let (n, _from) = self.socket.recv_from(&mut self.buf).await?;
        self.len = n;

        let header = MoldPacket::parse(&self.buf[..n])?.header;

        // A changed session id is the only reliable restart signal on this transport.
        if self.session != Some(header.session) {
            let previous = self.session.replace(header.session);
            self.tracker.observe_reset(header.sequence);
            if previous.is_some() {
                return Ok(MoldEvent::SessionChanged {
                    session: header.session,
                });
            }
        }

        if header.is_end_of_session() {
            return Ok(MoldEvent::EndOfSession);
        }
        if header.is_heartbeat() {
            if let Some(gap) = self.tracker.observe_heartbeat(header.sequence) {
                tracing::warn!(
                    target: "nasdaq::mold",
                    from = gap.from, to = gap.to,
                    "gap revealed by heartbeat"
                );
            }
            return Ok(MoldEvent::Heartbeat);
        }

        let count = header.payload_count();
        Ok(match self.tracker.observe(header.sequence, count as u32) {
            GapAction::InOrder | GapAction::Reset => MoldEvent::Data {
                sequence: header.sequence,
                count,
                skip: 0,
            },
            GapAction::PartialOverlap { skip } => MoldEvent::Data {
                sequence: header.sequence,
                count,
                skip,
            },
            GapAction::Gap { from, to } => {
                tracing::warn!(target: "nasdaq::mold", from, to, "sequence gap");
                MoldEvent::Data {
                    sequence: header.sequence,
                    count,
                    skip: 0,
                }
            }
            GapAction::Duplicate => MoldEvent::Duplicate,
        })
    }

    /// Send retransmission requests for any outstanding gaps, rate limited by
    /// `rerequest_interval`.
    ///
    /// Returns the ranges requested. Does nothing — and says so by returning an error —
    /// when no re-request server is configured, because silently running without recovery
    /// is how a book drifts for an entire session.
    pub async fn request_retransmissions(&mut self) -> Result<Vec<GapRange>> {
        if !self.tracker.has_gaps() {
            return Ok(Vec::new());
        }
        if self.config.request_servers.is_empty() {
            return Err(NasdaqError::NotConfigured(
                "sequence gap detected but no MoldUDP64 re-request server is configured".into(),
            ));
        }
        if self.last_rerequest.elapsed() < self.config.rerequest_interval {
            return Ok(Vec::new());
        }
        self.last_rerequest = std::time::Instant::now();

        let session = self.session.unwrap_or_default();
        let requests = self.tracker.pending_requests();
        for range in &requests {
            let payload =
                encode_request(session, range.from, range.len().min(u16::MAX as u64) as u16);
            // Requests go to one server; the others are failover targets.
            let server = self.config.request_servers
                [(range.attempts as usize - 1) % self.config.request_servers.len()];
            self.socket.send_to(&payload, server).await?;
        }
        Ok(requests)
    }

    /// Mark a recovered range so it stops being re-requested.
    pub fn note_recovered(&mut self, from: u64, to: u64) {
        self.tracker.fill(from, to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sess() -> SessionId {
        SessionId::from_str_padded("20260817AA")
    }

    #[test]
    fn header_layout_matches_the_specification() {
        let pkt = encode_packet(sess(), 42, &[b"hello", b"world!"]);
        assert_eq!(&pkt[..10], b"20260817AA");
        assert_eq!(u64::from_be_bytes(pkt[10..18].try_into().unwrap()), 42);
        assert_eq!(u16::from_be_bytes([pkt[18], pkt[19]]), 2);
        // First message block starts immediately after the 20-byte header.
        assert_eq!(u16::from_be_bytes([pkt[20], pkt[21]]), 5);
        assert_eq!(&pkt[22..27], b"hello");
    }

    #[test]
    fn messages_iterate_in_order() {
        let pkt = encode_packet(sess(), 1, &[b"a", b"bb", b"ccc"]);
        let parsed = MoldPacket::parse(&pkt).unwrap();
        let msgs: Vec<&[u8]> = parsed.messages().collect();
        assert_eq!(msgs, vec![&b"a"[..], &b"bb"[..], &b"ccc"[..]]);
        assert_eq!(parsed.header.sequence, 1);
    }

    #[test]
    fn zero_length_messages_are_legal() {
        let pkt = encode_packet(sess(), 1, &[b"", b"x"]);
        let parsed = MoldPacket::parse(&pkt).unwrap();
        assert_eq!(parsed.validate().unwrap(), vec![&b""[..], &b"x"[..]]);
    }

    #[test]
    fn heartbeat_carries_the_next_expected_sequence_and_no_messages() {
        let pkt = encode_heartbeat(sess(), 500);
        let parsed = MoldPacket::parse(&pkt).unwrap();
        assert!(parsed.header.is_heartbeat());
        assert_eq!(parsed.header.sequence, 500);
        assert_eq!(parsed.messages().count(), 0);
    }

    #[test]
    fn end_of_session_uses_the_ffff_sentinel() {
        let pkt = encode_end_of_session(sess(), 999);
        let parsed = MoldPacket::parse(&pkt).unwrap();
        assert!(parsed.header.is_end_of_session());
        assert!(!parsed.header.is_heartbeat());
        assert_eq!(parsed.header.payload_count(), 0);
    }

    #[test]
    fn a_truncated_datagram_is_rejected() {
        assert!(MoldPacket::parse(&[0u8; 19]).is_err());
    }

    #[test]
    fn a_lying_message_count_is_caught_by_validate() {
        let mut pkt = encode_packet(sess(), 1, &[b"abc"]);
        pkt[18] = 0;
        pkt[19] = 5; // claim five messages
        let parsed = MoldPacket::parse(&pkt).unwrap();
        assert!(parsed.validate().is_err());
        // The streaming iterator stops rather than reading past the end.
        assert_eq!(parsed.messages().count(), 1);
    }

    #[test]
    fn a_message_length_past_the_end_is_caught() {
        let mut pkt = encode_packet(sess(), 1, &[b"abc"]);
        pkt[20] = 0xFF;
        pkt[21] = 0xFF;
        let parsed = MoldPacket::parse(&pkt).unwrap();
        assert!(parsed.validate().is_err());
        assert_eq!(parsed.messages().count(), 0);
    }

    #[test]
    fn request_packet_has_the_same_shape_as_a_header() {
        let req = encode_request(sess(), 100, 50);
        assert_eq!(req.len(), HEADER_LEN);
        assert_eq!(u64::from_be_bytes(req[10..18].try_into().unwrap()), 100);
        assert_eq!(u16::from_be_bytes([req[18], req[19]]), 50);
    }

    #[test]
    fn session_ids_are_space_padded_to_ten_bytes() {
        let s = SessionId::from_str_padded("ABC");
        assert_eq!(&s.0, b"ABC       ");
        assert_eq!(s.trimmed(), "ABC");
    }

    #[test]
    fn config_rejects_a_unicast_group() {
        let cfg = MoldConfig::new("10.0.0.1:1234".parse().unwrap());
        assert!(cfg.validate().is_err());
        let cfg = MoldConfig::new("233.54.12.111:26477".parse().unwrap());
        assert!(cfg.validate().is_ok());
    }
}
