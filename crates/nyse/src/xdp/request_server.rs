//! Pillar Request Server client — retransmission, refresh and symbol-index recovery.
//!
//! The Request Server is a TCP endpoint. Requests are sent inside an ordinary XDP packet
//! header with `DeliveryFlag = 11` and a client-assigned sequence number; the *response*
//! comes back two ways at once:
//!
//! * a Request Response message (type 11) on the same TCP connection, immediately,
//!   accepting or rejecting the request; and
//! * if accepted, the requested data on the relevant Retransmission or Refresh **multicast**
//!   channel — not on the TCP socket.
//!
//! Two operational limits matter more than the protocol details. First, requests are quota'd
//! per day and per sequence range, and exceeding either gets a reject rather than data
//! (`Status` 3, 4 and 5). Second, intraday connections must answer the server's heartbeat
//! within five seconds or be disconnected — so the heartbeat responder is not optional
//! housekeeping, it is what keeps recovery available when it is finally needed.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::{NyseError, Result};
use exchange_core::wire::{Cursor, Writer};

use super::common::msg_type;
use super::packet::{delivery_flag, PACKET_HEADER_LEN};

/// Outcome of a request, from the Request Response message's `Status` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    /// `0` — accepted; the data will follow on the multicast channel.
    Accepted,
    /// `1` — invalid Source ID.
    InvalidSourceId,
    /// `3` — the sequence range exceeds the per-request maximum.
    RangeTooWide,
    /// `4` — daily retransmission request quota exhausted.
    TooManyRequestsToday,
    /// `5` — daily refresh request quota exhausted.
    TooManyRefreshesToday,
    /// `6` — the request's own sequence number is too old to be honoured.
    RequestTooOld,
    /// `7` — invalid Channel ID.
    InvalidChannelId,
    /// `8` — invalid Product ID.
    InvalidProductId,
    /// `9` — invalid message type, or type and size disagree.
    InvalidMessage,
    Unknown(char),
}

impl RequestStatus {
    const fn parse(ch: char) -> Self {
        match ch {
            '0' => Self::Accepted,
            '1' => Self::InvalidSourceId,
            '3' => Self::RangeTooWide,
            '4' => Self::TooManyRequestsToday,
            '5' => Self::TooManyRefreshesToday,
            '6' => Self::RequestTooOld,
            '7' => Self::InvalidChannelId,
            '8' => Self::InvalidProductId,
            '9' => Self::InvalidMessage,
            other => Self::Unknown(other),
        }
    }

    pub const fn is_accepted(self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// True when retrying the same request cannot possibly help. Quota and range rejections
    /// need a different strategy (wait for tomorrow, or split the range), not a retry loop.
    pub const fn is_permanent(self) -> bool {
        matches!(
            self,
            Self::InvalidSourceId
                | Self::InvalidChannelId
                | Self::InvalidProductId
                | Self::InvalidMessage
                | Self::TooManyRequestsToday
                | Self::TooManyRefreshesToday
        )
    }
}

impl std::fmt::Display for RequestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Accepted => "accepted",
            Self::InvalidSourceId => "invalid source id",
            Self::RangeTooWide => "sequence range too wide",
            Self::TooManyRequestsToday => "daily retransmission quota exhausted",
            Self::TooManyRefreshesToday => "daily refresh quota exhausted",
            Self::RequestTooOld => "request sequence number too old",
            Self::InvalidChannelId => "invalid channel id",
            Self::InvalidProductId => "invalid product id",
            Self::InvalidMessage => "invalid message type or size",
            Self::Unknown(_) => "unknown status",
        };
        f.write_str(s)
    }
}

/// Type 11 — the server's answer to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestResponse {
    /// The sequence number the client put on its request, for correlation.
    pub request_seq_num: u32,
    /// Range being retransmitted; zero for refresh and symbol-mapping responses.
    pub begin_seq_num: u32,
    pub end_seq_num: u32,
    pub source_id: String,
    pub product_id: u8,
    pub channel_id: u8,
    pub status: RequestStatus,
}

impl RequestResponse {
    pub const SIZE: usize = 29;

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < Self::SIZE {
            return Err(NyseError::Protocol(format!(
                "Request Response is {} bytes, expected at least {}",
                bytes.len(),
                Self::SIZE
            )));
        }
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            request_seq_num: c.le_u32()?,
            begin_seq_num: c.le_u32()?,
            end_seq_num: c.le_u32()?,
            source_id: c.nul_padded(10, "SourceID")?.to_string(),
            product_id: c.u8()?,
            channel_id: c.u8()?,
            status: RequestStatus::parse(c.ascii_char()?),
        })
    }

    pub fn encode(&self, status: char) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::REQUEST_RESPONSE)
            .le_u32(self.request_seq_num)
            .le_u32(self.begin_seq_num)
            .le_u32(self.end_seq_num)
            .nul_padded(&self.source_id, 10)
            .u8(self.product_id)
            .u8(self.channel_id)
            .ascii_char(status);
        w.into_vec()
    }
}

/// Type 10 — retransmission request.
pub fn encode_retransmission_request(
    begin_seq_num: u32,
    end_seq_num: u32,
    source_id: &str,
    product_id: u8,
    channel_id: u8,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(24);
    w.le_u16(24)
        .le_u16(msg_type::RETRANSMISSION_REQUEST)
        .le_u32(begin_seq_num)
        .le_u32(end_seq_num)
        .nul_padded(source_id, 10)
        .u8(product_id)
        .u8(channel_id);
    w.into_vec()
}

/// Type 15 — refresh request. `symbol_index` of 0 refreshes every symbol in the channel.
pub fn encode_refresh_request(
    symbol_index: u32,
    source_id: &str,
    product_id: u8,
    channel_id: u8,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(20);
    w.le_u16(20)
        .le_u16(msg_type::REFRESH_REQUEST)
        .le_u32(symbol_index)
        .nul_padded(source_id, 10)
        .u8(product_id)
        .u8(channel_id);
    w.into_vec()
}

/// Type 13 — symbol index mapping request. `symbol_index` of 0 requests every symbol.
pub fn encode_symbol_index_mapping_request(
    symbol_index: u32,
    source_id: &str,
    product_id: u8,
    channel_id: u8,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(21);
    w.le_u16(21)
        .le_u16(msg_type::SYMBOL_INDEX_MAPPING_REQUEST)
        .le_u32(symbol_index)
        .nul_padded(source_id, 10)
        .u8(product_id)
        .u8(channel_id)
        .u8(0); // RetransmitMethod: 0 = deliver via UDP
    w.into_vec()
}

/// Type 12 — heartbeat response. Must be sent within five seconds of the server's heartbeat.
pub fn encode_heartbeat_response(source_id: &str) -> Vec<u8> {
    let mut w = Writer::with_capacity(14);
    w.le_u16(14)
        .le_u16(msg_type::HEARTBEAT_RESPONSE)
        .nul_padded(source_id, 10);
    w.into_vec()
}

/// Request Server connection settings.
#[derive(Debug, Clone)]
pub struct RequestServerConfig {
    /// `host:port` of the Request Server, as assigned by NYSE.
    pub addr: String,
    /// Client identifier, at most 10 characters, NUL padded on the wire.
    pub source_id: String,
    pub product_id: u8,
    pub channel_id: u8,
    /// Must be under the server's 5-second disconnect threshold.
    pub heartbeat_response_timeout: Duration,
    pub response_timeout: Duration,
}

impl RequestServerConfig {
    pub fn new(
        addr: impl Into<String>,
        source_id: impl Into<String>,
        product_id: u8,
        channel_id: u8,
    ) -> Self {
        Self {
            addr: addr.into(),
            source_id: source_id.into(),
            product_id,
            channel_id,
            heartbeat_response_timeout: Duration::from_secs(2),
            response_timeout: Duration::from_secs(5),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.addr.trim().is_empty() {
            return Err(NyseError::NotConfigured(
                "Request Server address is empty; recovery would be unavailable".into(),
            ));
        }
        if self.source_id.trim().is_empty() {
            return Err(NyseError::NotConfigured(
                "Request Server source id is empty; the server rejects with Status 1".into(),
            ));
        }
        if self.source_id.len() > 10 {
            return Err(NyseError::NotConfigured(format!(
                "source id {:?} exceeds the 10-byte field",
                self.source_id
            )));
        }
        if self.heartbeat_response_timeout >= Duration::from_secs(5) {
            return Err(NyseError::NotConfigured(
                "heartbeat response must be under 5s or the server disconnects".into(),
            ));
        }
        Ok(())
    }
}

/// A connected Request Server client.
pub struct RequestServerClient {
    stream: TcpStream,
    config: RequestServerConfig,
    /// Client-side sequence number stamped on each outgoing request packet.
    next_request_seq: u32,
    buf: Vec<u8>,
}

impl RequestServerClient {
    pub async fn connect(config: RequestServerConfig) -> Result<Self> {
        config.validate()?;
        let stream = TcpStream::connect(&config.addr).await?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            config,
            next_request_seq: 1,
            buf: Vec::with_capacity(4096),
        })
    }

    /// Request retransmission of `[begin, end]` on this channel.
    pub async fn request_retransmission(
        &mut self,
        begin: u32,
        end: u32,
    ) -> Result<RequestResponse> {
        let msg = encode_retransmission_request(
            begin,
            end,
            &self.config.source_id,
            self.config.product_id,
            self.config.channel_id,
        );
        self.send(&msg).await?;
        self.await_response().await
    }

    /// Request a full refresh of one symbol, or of every symbol when `symbol_index` is 0.
    pub async fn request_refresh(&mut self, symbol_index: u32) -> Result<RequestResponse> {
        let msg = encode_refresh_request(
            symbol_index,
            &self.config.source_id,
            self.config.product_id,
            self.config.channel_id,
        );
        self.send(&msg).await?;
        self.await_response().await
    }

    /// Request the Symbol Index Mapping spin, which is what makes prices interpretable.
    pub async fn request_symbol_index_mapping(
        &mut self,
        symbol_index: u32,
    ) -> Result<RequestResponse> {
        let msg = encode_symbol_index_mapping_request(
            symbol_index,
            &self.config.source_id,
            self.config.product_id,
            self.config.channel_id,
        );
        self.send(&msg).await?;
        self.await_response().await
    }

    /// Answer a server heartbeat. Failing to do so within five seconds costs the connection.
    pub async fn send_heartbeat_response(&mut self) -> Result<()> {
        let msg = encode_heartbeat_response(&self.config.source_id);
        self.send(&msg).await
    }

    /// Wrap a request message in a packet header and write it.
    async fn send(&mut self, message: &[u8]) -> Result<()> {
        let seq = self.next_request_seq;
        self.next_request_seq = self.next_request_seq.wrapping_add(1).max(1);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();

        let packet = super::packet::encode_packet(
            delivery_flag::ORIGINAL,
            seq,
            now.as_secs() as u32,
            now.subsec_nanos(),
            &[message],
        );
        self.stream.write_all(&packet).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Read until a Request Response arrives, answering heartbeats meanwhile.
    async fn await_response(&mut self) -> Result<RequestResponse> {
        let deadline = tokio::time::Instant::now() + self.config.response_timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(NyseError::Timeout("Request Server response"));
            }

            let mut chunk = [0u8; 2048];
            let n = tokio::time::timeout_at(deadline, self.stream.read(&mut chunk))
                .await
                .map_err(|_| NyseError::Timeout("Request Server response"))??;
            if n == 0 {
                return Err(NyseError::SessionEnded(
                    "Request Server closed the connection".into(),
                ));
            }
            self.buf.extend_from_slice(&chunk[..n]);

            while let Some((msg_type_id, body, consumed)) = next_message(&self.buf) {
                let body = body.to_vec();
                self.buf.drain(..consumed);
                match msg_type_id {
                    msg_type::REQUEST_RESPONSE => return RequestResponse::parse(&body),
                    msg_type::HEARTBEAT_RESPONSE => {
                        // The server's heartbeat uses the same type; answer it.
                        self.send_heartbeat_response().await?;
                    }
                    other => tracing::debug!(
                        target: "nyse::request_server",
                        msg_type = other,
                        "ignoring unexpected message on the request socket"
                    ),
                }
            }
        }
    }
}

/// Pull the first complete message out of a buffer of packet-framed request-server traffic.
///
/// Returns `(msg_type, message bytes, bytes consumed from the buffer)`.
fn next_message(buf: &[u8]) -> Option<(u16, &[u8], usize)> {
    if buf.len() < PACKET_HEADER_LEN + 4 {
        return None;
    }
    let packet_size = u16::from_le_bytes([buf[0], buf[1]]) as usize;
    if packet_size < PACKET_HEADER_LEN || buf.len() < packet_size {
        return None;
    }
    let body = &buf[PACKET_HEADER_LEN..packet_size];
    if body.len() < 4 {
        return None;
    }
    let msg_size = u16::from_le_bytes([body[0], body[1]]) as usize;
    let msg_type_id = u16::from_le_bytes([body[2], body[3]]);
    if msg_size < 4 || msg_size > body.len() {
        return None;
    }
    Some((msg_type_id, &body[..msg_size], packet_size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retransmission_request_matches_the_documented_layout() {
        let m = encode_retransmission_request(100, 200, "ABCDEFGHI", 7, 3);
        assert_eq!(m.len(), 24);
        assert_eq!(u16::from_le_bytes([m[0], m[1]]), 24);
        assert_eq!(u16::from_le_bytes([m[2], m[3]]), 10);
        assert_eq!(u32::from_le_bytes(m[4..8].try_into().unwrap()), 100);
        assert_eq!(u32::from_le_bytes(m[8..12].try_into().unwrap()), 200);
        // Source ID is NUL padded, not space padded.
        assert_eq!(&m[12..22], b"ABCDEFGHI\0");
        assert_eq!(m[22], 7);
        assert_eq!(m[23], 3);
    }

    #[test]
    fn refresh_and_symbol_mapping_requests_have_their_documented_sizes() {
        assert_eq!(encode_refresh_request(0, "SRC", 1, 1).len(), 20);
        assert_eq!(
            encode_symbol_index_mapping_request(0, "SRC", 1, 1).len(),
            21
        );
        assert_eq!(encode_heartbeat_response("SRC").len(), 14);
    }

    #[test]
    fn a_symbol_index_of_zero_requests_every_symbol() {
        let m = encode_refresh_request(0, "SRC", 1, 1);
        assert_eq!(u32::from_le_bytes(m[4..8].try_into().unwrap()), 0);
        let m = encode_refresh_request(4242, "SRC", 1, 1);
        assert_eq!(u32::from_le_bytes(m[4..8].try_into().unwrap()), 4242);
    }

    #[test]
    fn symbol_mapping_request_asks_for_udp_delivery() {
        let m = encode_symbol_index_mapping_request(0, "SRC", 1, 1);
        assert_eq!(m[20], 0, "RetransmitMethod 0 = deliver via UDP");
    }

    #[test]
    fn request_response_round_trips() {
        let r = RequestResponse {
            request_seq_num: 5,
            begin_seq_num: 100,
            end_seq_num: 200,
            source_id: "SRC".into(),
            product_id: 7,
            channel_id: 3,
            status: RequestStatus::Accepted,
        };
        let bytes = r.encode('0');
        assert_eq!(bytes.len(), RequestResponse::SIZE);
        assert_eq!(RequestResponse::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn reject_statuses_are_named_and_classified() {
        for (code, expect, permanent) in [
            ('0', RequestStatus::Accepted, false),
            ('1', RequestStatus::InvalidSourceId, true),
            ('3', RequestStatus::RangeTooWide, false),
            ('4', RequestStatus::TooManyRequestsToday, true),
            ('5', RequestStatus::TooManyRefreshesToday, true),
            ('6', RequestStatus::RequestTooOld, false),
            ('7', RequestStatus::InvalidChannelId, true),
            ('8', RequestStatus::InvalidProductId, true),
            ('9', RequestStatus::InvalidMessage, true),
        ] {
            let r = RequestResponse {
                request_seq_num: 1,
                begin_seq_num: 0,
                end_seq_num: 0,
                source_id: "S".into(),
                product_id: 1,
                channel_id: 1,
                status: expect,
            };
            let parsed = RequestResponse::parse(&r.encode(code)).unwrap();
            assert_eq!(parsed.status, expect, "status code {code}");
            assert_eq!(
                parsed.status.is_permanent(),
                permanent,
                "status code {code}"
            );
        }
        assert!(RequestStatus::Accepted.is_accepted());
        assert!(!RequestStatus::RangeTooWide.is_accepted());
    }

    #[test]
    fn framing_pulls_one_message_at_a_time_from_a_tcp_stream() {
        let r = RequestResponse {
            request_seq_num: 1,
            begin_seq_num: 10,
            end_seq_num: 20,
            source_id: "SRC".into(),
            product_id: 1,
            channel_id: 1,
            status: RequestStatus::Accepted,
        };
        let msg = r.encode('0');
        let pkt = super::super::packet::encode_packet(delivery_flag::ORIGINAL, 1, 0, 0, &[&msg]);

        let mut stream = pkt.clone();
        stream.extend_from_slice(&pkt);

        let (ty, body, consumed) = next_message(&stream).unwrap();
        assert_eq!(ty, msg_type::REQUEST_RESPONSE);
        assert_eq!(RequestResponse::parse(body).unwrap(), r);
        assert_eq!(consumed, pkt.len());

        let (ty2, _, _) = next_message(&stream[consumed..]).unwrap();
        assert_eq!(ty2, msg_type::REQUEST_RESPONSE);
    }

    #[test]
    fn framing_waits_for_a_complete_packet() {
        let msg = encode_heartbeat_response("SRC");
        let pkt = super::super::packet::encode_packet(delivery_flag::ORIGINAL, 1, 0, 0, &[&msg]);
        for cut in 0..pkt.len() {
            assert!(next_message(&pkt[..cut]).is_none(), "truncated at {cut}");
        }
        assert!(next_message(&pkt).is_some());
    }

    #[test]
    fn config_validation_catches_settings_that_would_lose_recovery() {
        let mut cfg = RequestServerConfig::new("", "SRC", 1, 1);
        assert!(cfg.validate().is_err(), "empty address");

        cfg = RequestServerConfig::new("host:1", "", 1, 1);
        assert!(cfg.validate().is_err(), "empty source id");

        cfg = RequestServerConfig::new("host:1", "TOOLONGSOURCE", 1, 1);
        assert!(cfg.validate().is_err(), "source id over 10 bytes");

        cfg = RequestServerConfig::new("host:1", "SRC", 1, 1);
        cfg.heartbeat_response_timeout = Duration::from_secs(6);
        assert!(
            cfg.validate().is_err(),
            "heartbeat slower than the 5s cutoff"
        );

        cfg = RequestServerConfig::new("host:1", "SRC", 1, 1);
        assert!(cfg.validate().is_ok());
    }
}
