//! XDP packet and message framing, shared by every NYSE proprietary data feed.
//!
//! From the Pillar Common Client Specification:
//!
//! * every packet is a 16-byte header followed by one or more messages, except heartbeats
//!   which carry none;
//! * the maximum packet is 1400 bytes, so no message can exceed 1384;
//! * a message never straddles a packet boundary;
//! * binary fields are **little endian**, ASCII strings are left aligned and null padded;
//! * message fields align on 1-byte boundaries — there is no padding to remove.
//!
//! The single most important rule for a feed handler here is: **never hard-code a message
//! size**. Each message begins with its own `MsgSize`, and NYSE explicitly reserves the
//! right to append fields to existing message types. Walking the packet by `MsgSize` means
//! an unmodified handler keeps working when a longer variant goes live; walking it by a
//! compiled-in constant means it desynchronises mid-packet on release day. Everything below
//! validates against the documented size but advances by the declared one.

use exchange_core::wire::Cursor;
use exchange_core::{WireError, WireResult};

/// Bytes in the packet header.
pub const PACKET_HEADER_LEN: usize = 16;
/// Bytes in each message header.
pub const MESSAGE_HEADER_LEN: usize = 4;
/// Documented maximum packet size.
pub const MAX_PACKET_LEN: usize = 1400;

/// `DeliveryFlag` values from the Packet Header table.
pub mod delivery_flag {
    /// Packet contains no messages; `SeqNum` is the next expected sequence.
    pub const HEARTBEAT: u8 = 1;
    /// Publisher failover; treat the accompanying Sequence Number Reset as authoritative.
    pub const FAILOVER: u8 = 10;
    /// Ordinary real-time message.
    pub const ORIGINAL: u8 = 11;
    /// Carries a Sequence Number Reset message.
    pub const SEQUENCE_RESET: u8 = 12;
    pub const RETRANSMISSION_ONLY: u8 = 13;
    pub const RETRANSMISSION_PART: u8 = 15;
    pub const REFRESH_ONLY: u8 = 17;
    pub const REFRESH_START: u8 = 18;
    pub const REFRESH_PART: u8 = 19;
    pub const REFRESH_END: u8 = 20;
    pub const MESSAGE_UNAVAILABLE: u8 = 21;
}

/// How a packet reached us, derived from `DeliveryFlag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Heartbeat,
    Failover,
    Original,
    SequenceReset,
    /// Response to a Retransmission Request.
    Retransmission {
        last_in_sequence: bool,
    },
    /// Part of a Refresh (snapshot) sequence.
    Refresh {
        first_in_sequence: bool,
        last_in_sequence: bool,
    },
    /// The requested messages are no longer available.
    MessageUnavailable,
    Unknown(u8),
}

impl Delivery {
    pub const fn from_flag(flag: u8) -> Self {
        match flag {
            delivery_flag::HEARTBEAT => Self::Heartbeat,
            delivery_flag::FAILOVER => Self::Failover,
            delivery_flag::ORIGINAL => Self::Original,
            delivery_flag::SEQUENCE_RESET => Self::SequenceReset,
            delivery_flag::RETRANSMISSION_ONLY => Self::Retransmission {
                last_in_sequence: true,
            },
            delivery_flag::RETRANSMISSION_PART => Self::Retransmission {
                last_in_sequence: false,
            },
            delivery_flag::REFRESH_ONLY => Self::Refresh {
                first_in_sequence: true,
                last_in_sequence: true,
            },
            delivery_flag::REFRESH_START => Self::Refresh {
                first_in_sequence: true,
                last_in_sequence: false,
            },
            delivery_flag::REFRESH_PART => Self::Refresh {
                first_in_sequence: false,
                last_in_sequence: false,
            },
            delivery_flag::REFRESH_END => Self::Refresh {
                first_in_sequence: false,
                last_in_sequence: true,
            },
            delivery_flag::MESSAGE_UNAVAILABLE => Self::MessageUnavailable,
            other => Self::Unknown(other),
        }
    }

    /// True for packets that belong to a recovery stream rather than the live feed.
    pub const fn is_recovery(self) -> bool {
        matches!(self, Self::Retransmission { .. } | Self::Refresh { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketHeader {
    /// Total packet size in bytes, including this 16-byte header.
    pub packet_size: u16,
    pub delivery: Delivery,
    pub raw_delivery_flag: u8,
    /// Number of messages in this packet.
    pub message_count: u8,
    /// Sequence number of the *first* message; the rest are implicit.
    pub sequence: u32,
    /// Publication time: seconds since the UNIX epoch.
    pub send_time_secs: u32,
    /// Nanosecond offset within `send_time_secs`.
    pub send_time_nanos: u32,
}

impl PacketHeader {
    pub fn parse(buf: &[u8]) -> WireResult<Self> {
        let mut c = Cursor::new(buf);
        let packet_size = c.le_u16()?;
        let raw_delivery_flag = c.u8()?;
        let message_count = c.u8()?;
        Ok(Self {
            packet_size,
            delivery: Delivery::from_flag(raw_delivery_flag),
            raw_delivery_flag,
            message_count,
            sequence: c.le_u32()?,
            send_time_secs: c.le_u32()?,
            send_time_nanos: c.le_u32()?,
        })
    }

    /// Publication time as nanoseconds since the UNIX epoch.
    pub const fn send_time_epoch_nanos(&self) -> u64 {
        self.send_time_secs as u64 * 1_000_000_000 + self.send_time_nanos as u64
    }

    /// A heartbeat has `DeliveryFlag == 1` and no messages, and does **not** consume a
    /// sequence number — its `SeqNum` is the next one expected.
    pub const fn is_heartbeat(&self) -> bool {
        matches!(self.delivery, Delivery::Heartbeat) || self.message_count == 0
    }
}

/// A parsed packet: header plus a cursor over its message bodies.
#[derive(Debug, Clone)]
pub struct Packet<'a> {
    pub header: PacketHeader,
    body: &'a [u8],
}

impl<'a> Packet<'a> {
    pub fn parse(datagram: &'a [u8]) -> WireResult<Self> {
        if datagram.len() < PACKET_HEADER_LEN {
            return Err(WireError::Truncated {
                at: 0,
                need: PACKET_HEADER_LEN,
                have: datagram.len(),
            });
        }
        let header = PacketHeader::parse(datagram)?;

        // Trust the smaller of the declared size and what actually arrived: a datagram
        // shorter than `PktSize` was truncated in flight, and reading past it would mix in
        // whatever was left in the receive buffer.
        let declared = header.packet_size as usize;
        let end = declared.min(datagram.len()).max(PACKET_HEADER_LEN);

        Ok(Self {
            header,
            body: &datagram[PACKET_HEADER_LEN..end],
        })
    }

    /// Iterate the messages, each as `(msg_type, full message bytes including its header)`.
    pub fn messages(&self) -> MessageIter<'a> {
        MessageIter {
            body: self.body,
            offset: 0,
            remaining: self.header.message_count,
            index: 0,
        }
    }

    /// Decode every message strictly, failing on the first malformed one.
    pub fn validate(&self) -> WireResult<Vec<RawMessage<'a>>> {
        let mut out = Vec::with_capacity(self.header.message_count as usize);
        let mut offset = 0usize;
        for i in 0..self.header.message_count {
            if offset + MESSAGE_HEADER_LEN > self.body.len() {
                return Err(WireError::Truncated {
                    at: offset,
                    need: MESSAGE_HEADER_LEN,
                    have: self.body.len().saturating_sub(offset),
                });
            }
            let size = u16::from_le_bytes([self.body[offset], self.body[offset + 1]]) as usize;
            let msg_type = u16::from_le_bytes([self.body[offset + 2], self.body[offset + 3]]);
            if size < MESSAGE_HEADER_LEN || offset + size > self.body.len() {
                return Err(WireError::LengthMismatch {
                    protocol: "XDP",
                    msg_type,
                    declared: size,
                    expected: self.body.len() - offset,
                });
            }
            out.push(RawMessage {
                msg_type,
                index: i,
                bytes: &self.body[offset..offset + size],
            });
            offset += size;
        }
        Ok(out)
    }
}

/// One message inside a packet, still undecoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawMessage<'a> {
    pub msg_type: u16,
    /// Position within the packet; add to the packet's `SeqNum` for this message's sequence.
    pub index: u8,
    /// The complete message, starting at its own `MsgSize` field.
    pub bytes: &'a [u8],
}

impl RawMessage<'_> {
    /// Declared size, which is what the next message's offset is computed from.
    pub fn declared_size(&self) -> usize {
        self.bytes.len()
    }
}

pub struct MessageIter<'a> {
    body: &'a [u8],
    offset: usize,
    remaining: u8,
    index: u8,
}

impl<'a> Iterator for MessageIter<'a> {
    type Item = RawMessage<'a>;

    fn next(&mut self) -> Option<RawMessage<'a>> {
        if self.remaining == 0 || self.offset + MESSAGE_HEADER_LEN > self.body.len() {
            return None;
        }
        let size =
            u16::from_le_bytes([self.body[self.offset], self.body[self.offset + 1]]) as usize;
        let msg_type = u16::from_le_bytes([self.body[self.offset + 2], self.body[self.offset + 3]]);
        if size < MESSAGE_HEADER_LEN || self.offset + size > self.body.len() {
            self.remaining = 0;
            return None;
        }
        let bytes = &self.body[self.offset..self.offset + size];
        self.offset += size;
        self.remaining -= 1;
        let index = self.index;
        self.index += 1;
        Some(RawMessage {
            msg_type,
            index,
            bytes,
        })
    }
}

/// Read a message header from the front of a message body.
pub fn message_header(bytes: &[u8]) -> WireResult<(u16, u16)> {
    let mut c = Cursor::new(bytes);
    let size = c.le_u16()?;
    let ty = c.le_u16()?;
    Ok((size, ty))
}

/// Check a message against its documented minimum size.
///
/// NYSE may lengthen a message type in a future release, so this asserts "at least",
/// never "exactly" — the opposite of the ITCH rule, and for the opposite reason.
pub fn expect_min_size(bytes: &[u8], msg_type: u16, minimum: usize) -> WireResult<()> {
    if bytes.len() < minimum {
        return Err(WireError::LengthMismatch {
            protocol: "XDP",
            msg_type,
            declared: bytes.len(),
            expected: minimum,
        });
    }
    Ok(())
}

/// Build an XDP packet. Used by the request-server client (which sends its requests inside
/// an ordinary packet header) and by tests and capture tooling.
pub fn encode_packet(
    delivery_flag: u8,
    sequence: u32,
    send_time_secs: u32,
    send_time_nanos: u32,
    messages: &[&[u8]],
) -> Vec<u8> {
    use exchange_core::wire::Writer;
    let total = PACKET_HEADER_LEN + messages.iter().map(|m| m.len()).sum::<usize>();
    let mut w = Writer::with_capacity(total);
    w.le_u16(total as u16)
        .u8(delivery_flag)
        .u8(messages.len() as u8)
        .le_u32(sequence)
        .le_u32(send_time_secs)
        .le_u32(send_time_nanos);
    for m in messages {
        w.raw(m);
    }
    w.into_vec()
}

/// Build a heartbeat packet advertising the next expected sequence number.
pub fn encode_heartbeat(next_sequence: u32, send_time_secs: u32, send_time_nanos: u32) -> Vec<u8> {
    encode_packet(
        delivery_flag::HEARTBEAT,
        next_sequence,
        send_time_secs,
        send_time_nanos,
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange_core::wire::Writer;

    fn msg(size: u16, ty: u16) -> Vec<u8> {
        let mut w = Writer::with_capacity(size as usize);
        w.le_u16(size).le_u16(ty);
        for i in 0..(size as usize - 4) {
            w.u8(i as u8);
        }
        w.into_vec()
    }

    #[test]
    fn packet_header_layout_is_little_endian_at_the_documented_offsets() {
        let pkt = encode_packet(delivery_flag::ORIGINAL, 12_345, 1_700_000_000, 500, &[]);
        assert_eq!(u16::from_le_bytes([pkt[0], pkt[1]]), 16);
        assert_eq!(pkt[2], 11);
        assert_eq!(pkt[3], 0);
        assert_eq!(u32::from_le_bytes(pkt[4..8].try_into().unwrap()), 12_345);
        assert_eq!(
            u32::from_le_bytes(pkt[8..12].try_into().unwrap()),
            1_700_000_000
        );
        assert_eq!(u32::from_le_bytes(pkt[12..16].try_into().unwrap()), 500);
    }

    #[test]
    fn messages_are_walked_by_their_declared_size() {
        let a = msg(20, 100);
        let b = msg(35, 101);
        let c = msg(12, 102);
        let pkt = encode_packet(delivery_flag::ORIGINAL, 1, 0, 0, &[&a, &b, &c]);
        let parsed = Packet::parse(&pkt).unwrap();
        let types: Vec<u16> = parsed.messages().map(|m| m.msg_type).collect();
        assert_eq!(types, vec![100, 101, 102]);
        let sizes: Vec<usize> = parsed.messages().map(|m| m.declared_size()).collect();
        assert_eq!(sizes, vec![20, 35, 12]);
    }

    #[test]
    fn a_longer_than_documented_message_does_not_desynchronise_the_packet() {
        // Simulate a future release that appends four bytes to message type 100.
        let long100 = msg(43, 100); // documented size today is 39
        let next = msg(25, 102);
        let pkt = encode_packet(delivery_flag::ORIGINAL, 1, 0, 0, &[&long100, &next]);
        let parsed = Packet::parse(&pkt).unwrap();
        let msgs = parsed.validate().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].msg_type, 100);
        assert_eq!(msgs[0].declared_size(), 43);
        assert_eq!(msgs[1].msg_type, 102, "second message still found");
    }

    #[test]
    fn a_heartbeat_carries_no_messages() {
        let pkt = encode_heartbeat(777, 1, 2);
        let parsed = Packet::parse(&pkt).unwrap();
        assert!(parsed.header.is_heartbeat());
        assert_eq!(parsed.header.sequence, 777);
        assert_eq!(parsed.messages().count(), 0);
        assert_eq!(parsed.header.delivery, Delivery::Heartbeat);
    }

    #[test]
    fn delivery_flags_map_to_named_states() {
        assert_eq!(Delivery::from_flag(11), Delivery::Original);
        assert_eq!(Delivery::from_flag(12), Delivery::SequenceReset);
        assert_eq!(Delivery::from_flag(10), Delivery::Failover);
        assert_eq!(
            Delivery::from_flag(18),
            Delivery::Refresh {
                first_in_sequence: true,
                last_in_sequence: false
            }
        );
        assert_eq!(
            Delivery::from_flag(20),
            Delivery::Refresh {
                first_in_sequence: false,
                last_in_sequence: true
            }
        );
        assert!(Delivery::from_flag(15).is_recovery());
        assert!(!Delivery::from_flag(11).is_recovery());
        assert_eq!(Delivery::from_flag(99), Delivery::Unknown(99));
    }

    #[test]
    fn a_truncated_datagram_does_not_read_past_what_arrived() {
        let a = msg(20, 100);
        let full = encode_packet(delivery_flag::ORIGINAL, 1, 0, 0, &[&a]);
        // Deliver only half the packet while the header still claims the full size.
        let short = &full[..full.len() - 6];
        let parsed = Packet::parse(short).unwrap();
        assert_eq!(parsed.header.packet_size as usize, full.len());
        assert!(
            parsed.validate().is_err(),
            "strict decode reports the truncation"
        );
        assert_eq!(
            parsed.messages().count(),
            0,
            "lenient decode yields nothing"
        );
    }

    #[test]
    fn a_message_claiming_a_size_past_the_packet_is_rejected() {
        let mut a = msg(20, 100);
        a[0] = 0xFF;
        a[1] = 0x00; // claim 255 bytes
        let pkt = encode_packet(delivery_flag::ORIGINAL, 1, 0, 0, &[&a]);
        let parsed = Packet::parse(&pkt).unwrap();
        assert!(parsed.validate().is_err());
    }

    #[test]
    fn a_message_size_below_the_header_length_is_rejected() {
        let mut a = msg(20, 100);
        a[0] = 2;
        a[1] = 0;
        let pkt = encode_packet(delivery_flag::ORIGINAL, 1, 0, 0, &[&a]);
        assert!(Packet::parse(&pkt).unwrap().validate().is_err());
    }

    #[test]
    fn send_time_combines_into_epoch_nanoseconds() {
        let pkt = encode_packet(delivery_flag::ORIGINAL, 1, 1_700_000_000, 123_456_789, &[]);
        let h = Packet::parse(&pkt).unwrap().header;
        assert_eq!(h.send_time_epoch_nanos(), 1_700_000_000_123_456_789);
    }

    #[test]
    fn expect_min_size_allows_growth_but_not_shrinkage() {
        assert!(expect_min_size(&[0u8; 39], 100, 39).is_ok());
        assert!(
            expect_min_size(&[0u8; 43], 100, 39).is_ok(),
            "future field appended"
        );
        assert!(expect_min_size(&[0u8; 38], 100, 39).is_err());
    }
}
