//! SoupBinTCP 3.00 — Nasdaq's point-to-point session layer.
//!
//! Soup carries ITCH (market data, server → client) and OUCH (order entry, both ways) over
//! one TCP socket. Its whole job is to make the *server → client* stream a reliable,
//! resumable, sequenced stream of opaque messages:
//!
//! * Every logical packet is `[u16 big-endian length][u8 type][payload]`, where the length
//!   counts the type byte. The length is **not** a message boundary in the TCP stream — a
//!   packet may be split across segments or several packets may arrive together.
//! * Sequenced Data packets carry no explicit sequence number. Both sides count: the first
//!   sequenced message of a session is 1, and the Login Accepted packet states the number of
//!   the next one the server will send.
//! * On a socket failure the client reconnects and logs in again with the session id it was
//!   in and its next expected sequence number, and the server resumes exactly there. That
//!   is the entire recovery story for a Soup feed, and it is why the sequence counter below
//!   is authoritative rather than advisory.
//! * Client → server (Unsequenced Data) messages are **not** guaranteed. OUCH is designed
//!   around this: every inbound message can be resent benignly.
//! * Both sides must send something at least once a second; silence for an extended period
//!   means the link is down.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Instant, MissedTickBehavior};

use crate::{NasdaqError, Result};
use exchange_core::wire::{parse_ascii_u64, Writer};

/// Packet type bytes, from the specification's packet tables.
pub mod packet_type {
    /// Free-form human readable text; ignored by application software.
    pub const DEBUG: u8 = b'+';
    // Server → client
    pub const LOGIN_ACCEPTED: u8 = b'A';
    pub const LOGIN_REJECTED: u8 = b'J';
    pub const SEQUENCED_DATA: u8 = b'S';
    pub const SERVER_HEARTBEAT: u8 = b'H';
    pub const END_OF_SESSION: u8 = b'Z';
    // Client → server
    pub const LOGIN_REQUEST: u8 = b'L';
    pub const UNSEQUENCED_DATA: u8 = b'U';
    pub const CLIENT_HEARTBEAT: u8 = b'R';
    pub const LOGOUT_REQUEST: u8 = b'O';
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRejectReason {
    /// `A` — invalid username/password combination.
    NotAuthorized,
    /// `S` — the requested session was invalid or is not available.
    SessionNotAvailable,
    Unknown(char),
}

impl std::fmt::Display for LoginRejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAuthorized => f.write_str("not authorized (bad username/password)"),
            Self::SessionNotAvailable => f.write_str("requested session not available"),
            Self::Unknown(c) => write!(f, "unknown reject code {c:?}"),
        }
    }
}

impl LoginRejectReason {
    fn parse(ch: char) -> Self {
        match ch {
            'A' => Self::NotAuthorized,
            'S' => Self::SessionNotAvailable,
            other => Self::Unknown(other),
        }
    }
}

/// A decoded inbound packet, borrowing from the read buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoupPacket<'a> {
    LoginAccepted {
        session: &'a str,
        next_sequence: u64,
    },
    LoginRejected(LoginRejectReason),
    /// One higher-level message (an ITCH or OUCH message).
    SequencedData(&'a [u8]),
    ServerHeartbeat,
    EndOfSession,
    Debug(&'a [u8]),
    /// Present for completeness; a client never receives these.
    UnsequencedData(&'a [u8]),
    ClientHeartbeat,
    LogoutRequest,
}

/// A framed packet described by offsets rather than by a borrow.
///
/// The session loop needs to advance its read cursor *and* hand the payload to the caller;
/// describing the frame positionally keeps those two operations from fighting over the same
/// mutable borrow of the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Frame {
    ty: u8,
    /// Payload offset relative to the start of the slice that was scanned.
    payload_start: usize,
    payload_len: usize,
    /// Total bytes the packet occupies, including the 2-byte length prefix.
    total: usize,
}

/// Locate one complete packet at the front of `buf` without interpreting its payload.
fn scan_frame(buf: &[u8]) -> Result<Option<Frame>> {
    if buf.len() < 3 {
        return Ok(None);
    }
    // Length counts the type byte plus the payload, but not itself.
    let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    if len == 0 {
        return Err(NasdaqError::Protocol(
            "SoupBinTCP packet length of 0 (must include the type byte)".into(),
        ));
    }
    let total = 2 + len;
    if buf.len() < total {
        return Ok(None);
    }
    let ty = buf[2];
    if !matches!(
        ty,
        packet_type::DEBUG
            | packet_type::LOGIN_ACCEPTED
            | packet_type::LOGIN_REJECTED
            | packet_type::SEQUENCED_DATA
            | packet_type::SERVER_HEARTBEAT
            | packet_type::END_OF_SESSION
            | packet_type::LOGIN_REQUEST
            | packet_type::UNSEQUENCED_DATA
            | packet_type::CLIENT_HEARTBEAT
            | packet_type::LOGOUT_REQUEST
    ) {
        return Err(NasdaqError::Protocol(format!(
            "unknown SoupBinTCP packet type {:?} ({:#04x})",
            ty as char, ty
        )));
    }
    Ok(Some(Frame {
        ty,
        payload_start: 3,
        payload_len: total - 3,
        total,
    }))
}

/// Parse a Login Accepted payload: 10-byte session id then a 20-byte ASCII sequence number.
fn parse_login_accepted(payload: &[u8]) -> Result<(&str, u64)> {
    if payload.len() < 30 {
        return Err(NasdaqError::Protocol(format!(
            "Login Accepted payload is {} bytes, expected 30",
            payload.len()
        )));
    }
    let session = std::str::from_utf8(&payload[..10])
        .map_err(|_| NasdaqError::Protocol("session id is not ASCII".into()))?
        .trim();
    let next_sequence = parse_ascii_u64(&payload[10..30], "Sequence Number")?;
    Ok((session, next_sequence))
}

/// Decode one packet from the front of `buf`.
///
/// Returns `Ok(None)` when `buf` does not yet hold a complete packet — the normal case on a
/// stream socket — and the number of bytes consumed otherwise.
pub fn decode_packet(buf: &[u8]) -> Result<Option<(SoupPacket<'_>, usize)>> {
    let Some(frame) = scan_frame(buf)? else {
        return Ok(None);
    };
    let payload = &buf[frame.payload_start..frame.payload_start + frame.payload_len];

    let packet = match frame.ty {
        packet_type::LOGIN_ACCEPTED => {
            let (session, next_sequence) = parse_login_accepted(payload)?;
            SoupPacket::LoginAccepted {
                session,
                next_sequence,
            }
        }
        packet_type::LOGIN_REJECTED => {
            let ch = payload.first().copied().unwrap_or(b'?') as char;
            SoupPacket::LoginRejected(LoginRejectReason::parse(ch))
        }
        packet_type::SEQUENCED_DATA => SoupPacket::SequencedData(payload),
        packet_type::SERVER_HEARTBEAT => SoupPacket::ServerHeartbeat,
        packet_type::END_OF_SESSION => SoupPacket::EndOfSession,
        packet_type::DEBUG => SoupPacket::Debug(payload),
        packet_type::UNSEQUENCED_DATA | packet_type::LOGIN_REQUEST => {
            SoupPacket::UnsequencedData(payload)
        }
        packet_type::CLIENT_HEARTBEAT => SoupPacket::ClientHeartbeat,
        packet_type::LOGOUT_REQUEST => SoupPacket::LogoutRequest,
        _ => unreachable!("scan_frame already rejected unknown types"),
    };

    Ok(Some((packet, frame.total)))
}

/// Build a Login Request packet.
///
/// Field widths are fixed by the spec: username 6, password 10, requested session 10,
/// requested sequence number 20. Username and password are space padded on the right; the
/// sequence number is ASCII digits padded on the *left* with spaces.
///
/// An all-blank `session` means "the currently active session", and sequence `0` means
/// "start from the most recently generated message" rather than from the beginning.
pub fn encode_login_request(
    username: &str,
    password: &str,
    session: &str,
    requested_sequence: u64,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(49);
    w.be_u16(1 + 46) // type byte + 46 payload bytes
        .u8(packet_type::LOGIN_REQUEST)
        .space_padded(username, 6)
        .space_padded(password, 10)
        .space_padded(session, 10)
        .ascii_numeric_right(requested_sequence, 20);
    w.into_vec()
}

/// Build an Unsequenced Data packet wrapping one higher-level message (an OUCH order).
pub fn encode_unsequenced(payload: &[u8]) -> Vec<u8> {
    let mut w = Writer::with_capacity(3 + payload.len());
    w.be_u16((payload.len() + 1) as u16)
        .u8(packet_type::UNSEQUENCED_DATA)
        .raw(payload);
    w.into_vec()
}

/// Build a Sequenced Data packet. Only a Soup *server* sends these; included so a capture
/// replayer can regenerate a recorded stream.
pub fn encode_sequenced(payload: &[u8]) -> Vec<u8> {
    let mut w = Writer::with_capacity(3 + payload.len());
    w.be_u16((payload.len() + 1) as u16)
        .u8(packet_type::SEQUENCED_DATA)
        .raw(payload);
    w.into_vec()
}

pub fn encode_client_heartbeat() -> [u8; 3] {
    [0, 1, packet_type::CLIENT_HEARTBEAT]
}

pub fn encode_logout_request() -> [u8; 3] {
    [0, 1, packet_type::LOGOUT_REQUEST]
}

pub fn encode_login_accepted(session: &str, next_sequence: u64) -> Vec<u8> {
    let mut w = Writer::with_capacity(33);
    w.be_u16(1 + 30)
        .u8(packet_type::LOGIN_ACCEPTED)
        .space_padded(session, 10)
        .ascii_numeric_right(next_sequence, 20);
    w.into_vec()
}

/// Connection parameters for a Soup session.
#[derive(Debug, Clone)]
pub struct SoupConfig {
    /// `host:port` of the Soup server. There is no default: an ITCH or OUCH endpoint is
    /// assigned per subscriber, so guessing one would only produce a confusing timeout.
    pub addr: String,
    pub username: String,
    pub password: String,
    /// Session to resume, or blank for the currently active session.
    pub session: String,
    /// Next sequence number wanted. 1 replays the session from the start; 0 starts at the
    /// most recently generated message.
    pub requested_sequence: u64,
    /// How often to send a Client Heartbeat. The spec requires at least once per second.
    pub heartbeat_interval: Duration,
    /// Declare the link dead after this long with no inbound data or heartbeat.
    pub inactivity_timeout: Duration,
    /// How long to wait for the Login Accepted / Rejected response.
    pub login_timeout: Duration,
}

impl SoupConfig {
    pub fn new(
        addr: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            addr: addr.into(),
            username: username.into(),
            password: password.into(),
            session: String::new(),
            requested_sequence: 1,
            heartbeat_interval: Duration::from_millis(900),
            inactivity_timeout: Duration::from_secs(15),
            login_timeout: Duration::from_secs(30),
        }
    }

    /// Reject a configuration that cannot possibly connect, before any socket is opened.
    pub fn validate(&self) -> Result<()> {
        if self.addr.trim().is_empty() {
            return Err(NasdaqError::NotConfigured(
                "SoupBinTCP address is empty; set the endpoint assigned by Nasdaq".into(),
            ));
        }
        if self.username.trim().is_empty() {
            return Err(NasdaqError::NotConfigured(
                "SoupBinTCP username is empty".into(),
            ));
        }
        if self.username.len() > 6 {
            return Err(NasdaqError::NotConfigured(format!(
                "SoupBinTCP username {:?} exceeds the 6-byte field",
                self.username
            )));
        }
        if self.password.len() > 10 {
            return Err(NasdaqError::NotConfigured(
                "SoupBinTCP password exceeds the 10-byte field".into(),
            ));
        }
        if self.session.len() > 10 {
            return Err(NasdaqError::NotConfigured(
                "SoupBinTCP session id exceeds the 10-byte field".into(),
            ));
        }
        if self.heartbeat_interval >= Duration::from_secs(1) {
            return Err(NasdaqError::NotConfigured(
                "heartbeat interval must be under 1s; the spec requires traffic every second"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// What a live session hands back to the caller.
#[derive(Debug, PartialEq, Eq)]
pub enum SoupEvent<'a> {
    /// A sequenced higher-level message, with the sequence number it was assigned.
    Message { sequence: u64, payload: &'a [u8] },
    /// The server has nothing to send. Useful as a liveness tick.
    Heartbeat,
    /// Free-form server text.
    Debug(&'a [u8]),
    /// The session is over; reconnect with a blank session id for the next one.
    EndOfSession,
}

/// A logged-in SoupBinTCP client session.
pub struct SoupSession {
    stream: TcpStream,
    buf: Vec<u8>,
    /// Bytes at the front of `buf` already consumed.
    consumed: usize,
    session_id: String,
    next_sequence: u64,
    heartbeat_interval: Duration,
    inactivity_timeout: Duration,
    last_inbound: Instant,
}

impl SoupSession {
    /// Connect and complete the login handshake.
    pub async fn connect(cfg: &SoupConfig) -> Result<Self> {
        cfg.validate()?;

        let mut stream = TcpStream::connect(&cfg.addr).await?;
        // Order entry and market data are both latency sensitive and message oriented;
        // Nagle would coalesce a heartbeat with the next order.
        stream.set_nodelay(true)?;

        let login = encode_login_request(
            &cfg.username,
            &cfg.password,
            &cfg.session,
            cfg.requested_sequence,
        );
        stream.write_all(&login).await?;
        stream.flush().await?;

        let mut session = Self {
            stream,
            buf: Vec::with_capacity(64 * 1024),
            consumed: 0,
            session_id: String::new(),
            next_sequence: cfg.requested_sequence.max(1),
            heartbeat_interval: cfg.heartbeat_interval,
            inactivity_timeout: cfg.inactivity_timeout,
            last_inbound: Instant::now(),
        };

        // The spec guarantees Login Accepted or Login Rejected is the first non-debug
        // packet, so anything else here is a protocol violation worth failing on.
        let deadline = Instant::now() + cfg.login_timeout;
        loop {
            let Some(frame) = session.next_frame()? else {
                if Instant::now() >= deadline {
                    return Err(NasdaqError::Timeout("SoupBinTCP login response"));
                }
                session.fill().await?;
                continue;
            };
            let (start, len) = session.payload_bounds(frame);
            session.consumed += frame.total;

            match frame.ty {
                packet_type::LOGIN_ACCEPTED => {
                    let (sid, next_sequence) =
                        parse_login_accepted(&session.buf[start..start + len])?;
                    session.session_id = sid.to_string();
                    session.next_sequence = next_sequence;
                    return Ok(session);
                }
                packet_type::LOGIN_REJECTED => {
                    let ch = session.buf.get(start).copied().unwrap_or(b'?') as char;
                    return Err(NasdaqError::LoginRejected(LoginRejectReason::parse(ch)));
                }
                packet_type::DEBUG => {
                    tracing::debug!(
                        target: "nasdaq::soup",
                        text = %String::from_utf8_lossy(&session.buf[start..start + len]),
                        "soup debug packet during login"
                    );
                }
                other => {
                    return Err(NasdaqError::Protocol(format!(
                        "expected Login Accepted/Rejected, got packet type {:?}",
                        other as char
                    )))
                }
            }
        }
    }

    /// The session id the server put us in. Pass this back on reconnect to resume.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Sequence number of the next message the server will send. Persist this so a
    /// reconnect resumes exactly where the socket died.
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Send a client-originated message (an OUCH order) as Unsequenced Data.
    ///
    /// These are not guaranteed across a socket failure; OUCH's design assumption is that
    /// the caller may resend any pending inbound message benignly after a reconnect.
    pub async fn send(&mut self, payload: &[u8]) -> Result<()> {
        let pkt = encode_unsequenced(payload);
        self.stream.write_all(&pkt).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Ask the server to close the connection.
    pub async fn logout(&mut self) -> Result<()> {
        self.stream.write_all(&encode_logout_request()).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Await the next session event, sending heartbeats and enforcing the inactivity
    /// timeout while waiting.
    ///
    /// The returned event borrows the read buffer, so it must be consumed before the next
    /// call — which is what keeps the decode path allocation free.
    pub async fn next_event(&mut self) -> Result<SoupEvent<'_>> {
        let mut hb = tokio::time::interval(self.heartbeat_interval);
        hb.set_missed_tick_behavior(MissedTickBehavior::Delay);
        hb.tick().await; // the first tick completes immediately

        loop {
            // Anything already buffered wins before touching the socket.
            if let Some(frame) = self.next_frame()? {
                let (start, len) = self.payload_bounds(frame);
                self.consumed += frame.total;

                match frame.ty {
                    packet_type::SEQUENCED_DATA => {
                        let sequence = self.next_sequence;
                        self.next_sequence += 1;
                        return Ok(SoupEvent::Message {
                            sequence,
                            payload: &self.buf[start..start + len],
                        });
                    }
                    packet_type::SERVER_HEARTBEAT => return Ok(SoupEvent::Heartbeat),
                    packet_type::END_OF_SESSION => return Ok(SoupEvent::EndOfSession),
                    packet_type::DEBUG => {
                        return Ok(SoupEvent::Debug(&self.buf[start..start + len]))
                    }
                    packet_type::LOGIN_ACCEPTED | packet_type::LOGIN_REJECTED => {
                        return Err(NasdaqError::Protocol(
                            "unexpected login packet on an established session".into(),
                        ))
                    }
                    // A client never receives these from a Soup server; skip rather than
                    // fail, so a loopback or echo port cannot kill a live session.
                    _ => continue,
                }
            }

            tokio::select! {
                biased;
                read = read_more(&mut self.stream, &mut self.buf) => {
                    match read? {
                        0 => return Err(NasdaqError::SessionEnded(
                            "server closed the TCP connection".into(),
                        )),
                        _ => self.last_inbound = Instant::now(),
                    }
                }
                _ = hb.tick() => {
                    self.stream.write_all(&encode_client_heartbeat()).await?;
                    self.stream.flush().await?;
                    if self.last_inbound.elapsed() > self.inactivity_timeout {
                        return Err(NasdaqError::SessionEnded(format!(
                            "no inbound traffic for {:?}; link presumed down",
                            self.last_inbound.elapsed()
                        )));
                    }
                }
            }
        }
    }

    /// Read at least once into the buffer.
    async fn fill(&mut self) -> Result<()> {
        let n = read_more(&mut self.stream, &mut self.buf).await?;
        if n == 0 {
            return Err(NasdaqError::SessionEnded(
                "server closed the TCP connection during login".into(),
            ));
        }
        self.last_inbound = Instant::now();
        Ok(())
    }

    /// Frame the next packet in the unconsumed portion of the buffer, compacting first.
    fn next_frame(&mut self) -> Result<Option<Frame>> {
        // Compact only when it is worth it, so the common case is a pointer bump.
        if self.consumed > 0 && (self.consumed == self.buf.len() || self.consumed > 32 * 1024) {
            self.buf.drain(..self.consumed);
            self.consumed = 0;
        }
        if self.consumed >= self.buf.len() {
            return Ok(None);
        }
        scan_frame(&self.buf[self.consumed..])
    }

    /// Absolute `(start, len)` of a frame's payload inside `self.buf`.
    #[inline]
    fn payload_bounds(&self, frame: Frame) -> (usize, usize) {
        (self.consumed + frame.payload_start, frame.payload_len)
    }
}

async fn read_more(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Result<usize> {
    let mut chunk = [0u8; 16 * 1024];
    let n = stream.read(&mut chunk).await?;
    buf.extend_from_slice(&chunk[..n]);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_request_is_forty_nine_bytes_with_the_documented_layout() {
        let pkt = encode_login_request("USER01", "SECRET", "", 1);
        assert_eq!(pkt.len(), 49);
        assert_eq!(u16::from_be_bytes([pkt[0], pkt[1]]), 47);
        assert_eq!(pkt[2], b'L');
        assert_eq!(&pkt[3..9], b"USER01");
        assert_eq!(&pkt[9..19], b"SECRET    ");
        assert_eq!(&pkt[19..29], b"          ");
        assert_eq!(&pkt[29..49], b"                   1");
    }

    #[test]
    fn login_accepted_round_trips() {
        let pkt = encode_login_accepted("SESS01", 4_294_967_296);
        let (decoded, consumed) = decode_packet(&pkt).unwrap().unwrap();
        assert_eq!(consumed, pkt.len());
        assert_eq!(
            decoded,
            SoupPacket::LoginAccepted {
                session: "SESS01",
                next_sequence: 4_294_967_296
            }
        );
    }

    #[test]
    fn sequenced_data_carries_the_payload_verbatim() {
        let payload = b"\x41\x00\x01\x00\x02";
        let pkt = encode_sequenced(payload);
        let (decoded, _) = decode_packet(&pkt).unwrap().unwrap();
        assert_eq!(decoded, SoupPacket::SequencedData(payload));
    }

    #[test]
    fn a_partial_packet_yields_none_rather_than_a_garbage_read() {
        let pkt = encode_sequenced(b"0123456789");
        for cut in 0..pkt.len() {
            assert_eq!(
                decode_packet(&pkt[..cut]).unwrap(),
                None,
                "truncated at {cut} should not decode"
            );
        }
        assert!(decode_packet(&pkt).unwrap().is_some());
    }

    #[test]
    fn several_packets_in_one_tcp_segment_decode_one_at_a_time() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&encode_sequenced(b"aaa"));
        stream.extend_from_slice(&encode_client_heartbeat());
        stream.extend_from_slice(&encode_sequenced(b"bb"));

        let mut off = 0;
        let (p1, c1) = decode_packet(&stream[off..]).unwrap().unwrap();
        assert_eq!(p1, SoupPacket::SequencedData(b"aaa"));
        off += c1;
        let (p2, c2) = decode_packet(&stream[off..]).unwrap().unwrap();
        assert_eq!(p2, SoupPacket::ClientHeartbeat);
        off += c2;
        let (p3, c3) = decode_packet(&stream[off..]).unwrap().unwrap();
        assert_eq!(p3, SoupPacket::SequencedData(b"bb"));
        off += c3;
        assert_eq!(off, stream.len());
    }

    #[test]
    fn heartbeats_and_logout_are_three_byte_packets() {
        assert_eq!(encode_client_heartbeat(), [0, 1, b'R']);
        assert_eq!(encode_logout_request(), [0, 1, b'O']);
        let hb = encode_client_heartbeat();
        let (p, n) = decode_packet(&hb).unwrap().unwrap();
        assert_eq!((p, n), (SoupPacket::ClientHeartbeat, 3));
    }

    #[test]
    fn login_reject_codes_are_named() {
        for (code, expect) in [
            (b'A', LoginRejectReason::NotAuthorized),
            (b'S', LoginRejectReason::SessionNotAvailable),
        ] {
            let pkt = [0u8, 2, b'J', code];
            let (p, _) = decode_packet(&pkt).unwrap().unwrap();
            assert_eq!(p, SoupPacket::LoginRejected(expect));
        }
    }

    #[test]
    fn a_zero_length_packet_is_a_protocol_error() {
        assert!(decode_packet(&[0, 0, 0]).is_err());
    }

    #[test]
    fn an_unknown_packet_type_is_rejected_not_skipped() {
        let pkt = [0u8, 1, b'!'];
        assert!(decode_packet(&pkt).is_err());
    }

    #[test]
    fn config_validation_rejects_unusable_settings_before_connecting() {
        let mut cfg = SoupConfig::new("", "user", "pass");
        assert!(cfg.validate().is_err(), "empty address");

        cfg = SoupConfig::new("host:1234", "TOOLONGUSER", "pass");
        assert!(cfg.validate().is_err(), "username longer than 6 bytes");

        cfg = SoupConfig::new("host:1234", "user", "pass");
        cfg.heartbeat_interval = Duration::from_secs(2);
        assert!(cfg.validate().is_err(), "heartbeat slower than 1s");

        cfg = SoupConfig::new("host:1234", "user", "pass");
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn a_sequence_of_zero_means_start_from_the_latest_message() {
        let pkt = encode_login_request("u", "p", "", 0);
        assert_eq!(&pkt[29..49], b"                   0");
    }
}
