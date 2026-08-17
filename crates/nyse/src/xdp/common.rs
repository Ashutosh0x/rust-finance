//! Control and referential messages shared by every NYSE proprietary data feed.
//!
//! These are defined in the Pillar Common Client Specification rather than in any one
//! feed's specification, and they carry the state a feed handler cannot work without:
//!
//! | Type | Message                  | Why it matters                                          |
//! |------|--------------------------|---------------------------------------------------------|
//! | 1    | Sequence Number Reset    | The only authoritative restart signal on the channel.    |
//! | 2    | Source Time Reference    | Supplies the *seconds* half of every data timestamp.     |
//! | 3    | Symbol Index Mapping     | Symbol, price scale, lot size for a numeric symbol index.|
//! | 32   | Symbol Clear             | Drop all state for a symbol; a full refresh follows.     |
//! | 34   | Security Status          | Halts, short-sale restriction, session transitions.      |
//! | 35   | Refresh Header           | Frames a refresh (snapshot) sequence.                    |
//! | 31   | Message Unavailable      | The requested range can no longer be recovered.          |
//!
//! Types 2 and 3 are the two that break a handler silently if skipped. High-volume feeds
//! such as Integrated publish only `SourceTimeNS` on each data message — the seconds come
//! from the most recent Source Time Reference for that matching-engine partition — and data
//! messages carry no symbol at all, only the index established by type 3.

use exchange_core::wire::Cursor;
use exchange_core::{Price, WireResult};

use super::packet::expect_min_size;

/// Message type codes.
pub mod msg_type {
    pub const SEQUENCE_NUMBER_RESET: u16 = 1;
    pub const SOURCE_TIME_REFERENCE: u16 = 2;
    pub const SYMBOL_INDEX_MAPPING: u16 = 3;
    pub const RETRANSMISSION_REQUEST: u16 = 10;
    pub const REQUEST_RESPONSE: u16 = 11;
    pub const HEARTBEAT_RESPONSE: u16 = 12;
    pub const SYMBOL_INDEX_MAPPING_REQUEST: u16 = 13;
    pub const REFRESH_REQUEST: u16 = 15;
    pub const MESSAGE_UNAVAILABLE: u16 = 31;
    pub const SYMBOL_CLEAR: u16 = 32;
    pub const SECURITY_STATUS: u16 = 34;
    pub const REFRESH_HEADER: u16 = 35;
}

/// Type 1 — sequence numbering restarted on this channel.
///
/// Always arrives in a packet of its own with `SeqNum = 1`. `DeliveryFlag` is 12 at system
/// startup and 10 during a publisher failover; either way the client must reset its
/// watermark and discard outstanding gaps, because the numbers now mean something new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceNumberReset {
    pub source_time_secs: u32,
    pub source_time_nanos: u32,
    pub product_id: u8,
    pub channel_id: u8,
}

impl SequenceNumberReset {
    pub const SIZE: usize = 14;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::SEQUENCE_NUMBER_RESET, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_secs: c.le_u32()?,
            source_time_nanos: c.le_u32()?,
            product_id: c.u8()?,
            channel_id: c.u8()?,
        })
    }
}

/// Type 2 — the seconds half of the matching-engine timestamp, published once a second per
/// partition.
///
/// Data messages on Integrated and BBO carry only `SourceTimeNS`; concatenating it with the
/// most recent `SourceTime` for the same partition produces the full 8-byte event time.
/// Without this, every data timestamp is a bare nanosecond offset with no epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceTimeReference {
    /// Matching-engine partition this applies to.
    pub id: u32,
    /// Reserved for future use.
    pub symbol_seq_num: u32,
    pub source_time_secs: u32,
}

impl SourceTimeReference {
    pub const SIZE: usize = 16;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::SOURCE_TIME_REFERENCE, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            id: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            source_time_secs: c.le_u32()?,
        })
    }
}

/// Type 3 — referential data for one symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolIndexMapping {
    pub symbol_index: u32,
    /// NYSE-format ticker, null terminated within an 11-byte field.
    pub symbol: String,
    /// 1 NYSE, 3 Arca Equities, 4 Arca Options, 5 Bonds, 8 American Options,
    /// 9 American Equities, 10 National, 11 NYSE Texas (formerly Chicago).
    pub market_id: u16,
    /// Matching-engine instance; needed to disambiguate 4-byte trade ids across markets.
    pub system_id: u8,
    /// Listing market: `A`, `L`, `N`, `P`, `Q`, `V`, `Z`.
    pub exchange_code: char,
    /// Decimal places for every price field on this symbol. **Required** to interpret any
    /// price on the feed.
    pub price_scale_code: u8,
    /// `C` common, `E` ETF, `P` preferred, `T` test, …
    pub security_type: char,
    pub lot_size: u16,
    pub prev_close_price: Price,
    pub prev_close_volume: u32,
    /// 0 all penny, 1 penny/nickel, 5 nickel/dime.
    pub price_resolution: u8,
    /// `Y` when round lots are accepted.
    pub round_lot: char,
    /// Minimum price variation, in hundredths of a cent. Usually 1 ($0.0001).
    pub mpv: u16,
    /// Unit of trade in shares: 1, 10, 50 or 100.
    pub unit_of_trade: u16,
}

impl SymbolIndexMapping {
    pub const SIZE: usize = 44;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::SYMBOL_INDEX_MAPPING, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        let symbol_index = c.le_u32()?;
        let symbol = c.nul_padded(11, "Symbol")?.to_string();
        c.skip(1)?; // Reserved
        let market_id = c.le_u16()?;
        let system_id = c.u8()?;
        let exchange_code = c.ascii_char()?;
        let price_scale_code = c.u8()?;
        let security_type = c.ascii_char()?;
        let lot_size = c.le_u16()?;
        let prev_close_raw = c.le_i32()?;
        let prev_close_volume = c.le_u32()?;
        let price_resolution = c.u8()?;
        let round_lot = c.ascii_char()?;
        let mpv = c.le_u16()?;
        let unit_of_trade = c.le_u16()?;

        Ok(Self {
            symbol_index,
            symbol,
            market_id,
            system_id,
            exchange_code,
            price_scale_code,
            security_type,
            lot_size,
            prev_close_price: Price::from_xdp(prev_close_raw, price_scale_code),
            prev_close_volume,
            price_resolution,
            round_lot,
            mpv,
            unit_of_trade,
        })
    }

    /// True for symbols NYSE marks as test issues, which must never reach a strategy.
    pub const fn is_test_symbol(&self) -> bool {
        self.security_type == 'T'
    }

    /// Minimum price variation as a [`Price`]. `MPV` is in hundredths of a cent, i.e. four
    /// implied decimal places.
    pub fn minimum_price_variation(&self) -> Price {
        Price::from_scaled(self.mpv as i64, 4)
    }
}

/// Type 32 — clear all state for one symbol; a full refresh follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolClear {
    pub source_time_secs: u32,
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    /// Symbol sequence number the next message for this symbol will carry.
    pub next_source_seq_num: u32,
}

impl SymbolClear {
    pub const SIZE: usize = 20;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::SYMBOL_CLEAR, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_secs: c.le_u32()?,
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            next_source_seq_num: c.le_u32()?,
        })
    }
}

/// The `Security Status` field, which multiplexes four different state machines onto one
/// byte. Which one a given value belongs to is decided purely by the character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityStatusCode {
    /// `4` — trading halt.
    TradingHalt,
    /// `5` — resume.
    Resume,
    /// `A` — short-sale restriction activated (day 1).
    SsrActivated,
    /// `C` — short-sale restriction continued (day 2).
    SsrContinued,
    /// `D` — short-sale restriction deactivated.
    SsrDeactivated,
    /// `P` — pre-opening.
    PreOpening,
    /// `B` — the Pillar gateways have begun accepting orders (market state may still be
    /// pre-opening).
    BeginAcceptingOrders,
    /// `E` — early session.
    EarlySession,
    /// `O` — core session.
    CoreSession,
    /// `L` — late session (non-NYSE markets only).
    LateSession,
    /// `X` — closed.
    Closed,
    /// `I` — halt resume price indication.
    HaltResumePriceIndication,
    /// `G` — pre-opening price indication.
    PreOpeningPriceIndication,
    Unknown(char),
}

impl SecurityStatusCode {
    pub const fn parse(ch: char) -> Self {
        match ch {
            '4' => Self::TradingHalt,
            '5' => Self::Resume,
            'A' => Self::SsrActivated,
            'C' => Self::SsrContinued,
            'D' => Self::SsrDeactivated,
            'P' => Self::PreOpening,
            'B' => Self::BeginAcceptingOrders,
            'E' => Self::EarlySession,
            'O' => Self::CoreSession,
            'L' => Self::LateSession,
            'X' => Self::Closed,
            'I' => Self::HaltResumePriceIndication,
            'G' => Self::PreOpeningPriceIndication,
            other => Self::Unknown(other),
        }
    }

    /// True when this value carries indication prices in the `Price 1`/`Price 2` fields.
    pub const fn is_price_indication(self) -> bool {
        matches!(
            self,
            Self::HaltResumePriceIndication | Self::PreOpeningPriceIndication
        )
    }
}

/// `Halt Condition`. `~` means the security is not halted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltCondition {
    NotHalted,
    NewsDissemination,
    OrderImbalance,
    NewsPending,
    /// `M` — LULD pause.
    LuldPause,
    EquipmentChangeover,
    AdditionalInformationRequested,
    RegulatoryConcern,
    MergerEffective,
    EtfComponentPricesUnavailable,
    CorporateAction,
    NewSecurityOffering,
    IntradayIndicativeValueUnavailable,
    /// `1`/`2`/`3` — market-wide circuit breaker halt at the named level.
    MarketWideCircuitBreaker(u8),
    Unknown(char),
}

impl HaltCondition {
    pub const fn parse(ch: char) -> Self {
        match ch {
            '~' => Self::NotHalted,
            'D' => Self::NewsDissemination,
            'I' => Self::OrderImbalance,
            'P' => Self::NewsPending,
            'M' => Self::LuldPause,
            'X' => Self::EquipmentChangeover,
            'A' => Self::AdditionalInformationRequested,
            'C' => Self::RegulatoryConcern,
            'E' => Self::MergerEffective,
            'F' => Self::EtfComponentPricesUnavailable,
            'N' => Self::CorporateAction,
            'O' => Self::NewSecurityOffering,
            'V' => Self::IntradayIndicativeValueUnavailable,
            '1' => Self::MarketWideCircuitBreaker(1),
            '2' => Self::MarketWideCircuitBreaker(2),
            '3' => Self::MarketWideCircuitBreaker(3),
            other => Self::Unknown(other),
        }
    }

    pub const fn is_halted(self) -> bool {
        !matches!(self, Self::NotHalted)
    }
}

/// Type 34 — security status change.
///
/// Prices in this message are *not* scaled here: the caller must apply the symbol's
/// `PriceScaleCode` from the Symbol Index Mapping message, which is why the raw numerators
/// are exposed alongside the decoded fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityStatus {
    pub source_time_secs: u32,
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub status: SecurityStatusCode,
    pub halt_condition: HaltCondition,
    /// SSR triggering trade price when `status` is `SsrActivated`; indication low price
    /// when it is a price indication. Raw numerator — apply the symbol's price scale.
    pub price1_raw: i32,
    /// Indication high price for a price indication. Raw numerator.
    pub price2_raw: i32,
    pub ssr_triggering_exchange_id: char,
    pub ssr_triggering_volume: u32,
    /// `HHMMSSmmm`, or 0.
    pub time: u32,
    /// `~` no restriction in effect, `E` in effect.
    pub ssr_state: char,
    /// `P`, `E`, `O`, `L`, `X`.
    pub market_state: char,
}

impl SecurityStatus {
    pub const SIZE: usize = 46;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::SECURITY_STATUS, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        let source_time_secs = c.le_u32()?;
        let source_time_nanos = c.le_u32()?;
        let symbol_index = c.le_u32()?;
        let symbol_seq_num = c.le_u32()?;
        let status = SecurityStatusCode::parse(c.ascii_char()?);
        let halt_condition = HaltCondition::parse(c.ascii_char()?);
        c.skip(4)?; // Reserved
        let price1_raw = c.le_i32()?;
        let price2_raw = c.le_i32()?;
        let ssr_triggering_exchange_id = c.ascii_char()?;
        let ssr_triggering_volume = c.le_u32()?;
        let time = c.le_u32()?;
        let ssr_state = c.ascii_char()?;
        let market_state = c.ascii_char()?;

        Ok(Self {
            source_time_secs,
            source_time_nanos,
            symbol_index,
            symbol_seq_num,
            status,
            halt_condition,
            price1_raw,
            price2_raw,
            ssr_triggering_exchange_id,
            ssr_triggering_volume,
            time,
            ssr_state,
            market_state,
        })
    }

    /// True when a short-sale restriction is currently in force for this symbol.
    pub const fn short_sale_restricted(&self) -> bool {
        self.ssr_state == 'E'
    }
}

/// Type 35 — header of a refresh (snapshot) sequence.
///
/// `last_seq_num` is the value to resume the real-time channel from once the snapshot has
/// been applied: the Pillar guidance is explicitly to use the `LastSeqNum` in this header
/// for symbol-based recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshHeader {
    pub source_time_secs: u32,
    pub source_time_nanos: u32,
    /// Sequence number of the last real-time message reflected in this snapshot.
    pub last_seq_num: u32,
    pub symbol_index: u32,
}

impl RefreshHeader {
    pub const SIZE: usize = 20;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::REFRESH_HEADER, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_secs: c.le_u32()?,
            source_time_nanos: c.le_u32()?,
            last_seq_num: c.le_u32()?,
            symbol_index: c.le_u32()?,
        })
    }
}

/// Type 31 — the requested messages are no longer available.
///
/// This ends the retransmission path: the only remaining recovery is a full refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageUnavailable {
    pub begin_seq_num: u32,
    pub end_seq_num: u32,
}

impl MessageUnavailable {
    pub const SIZE: usize = 12;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::MESSAGE_UNAVAILABLE, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            begin_seq_num: c.le_u32()?,
            end_seq_num: c.le_u32()?,
        })
    }
}

/// Any control message.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlMessage {
    SequenceNumberReset(SequenceNumberReset),
    SourceTimeReference(SourceTimeReference),
    SymbolIndexMapping(Box<SymbolIndexMapping>),
    SymbolClear(SymbolClear),
    SecurityStatus(SecurityStatus),
    RefreshHeader(RefreshHeader),
    MessageUnavailable(MessageUnavailable),
}

/// Decode a control message, or `None` if `msg_type` is not one.
pub fn decode_control(msg_type: u16, bytes: &[u8]) -> WireResult<Option<ControlMessage>> {
    Ok(Some(match msg_type {
        msg_type::SEQUENCE_NUMBER_RESET => {
            ControlMessage::SequenceNumberReset(SequenceNumberReset::parse(bytes)?)
        }
        msg_type::SOURCE_TIME_REFERENCE => {
            ControlMessage::SourceTimeReference(SourceTimeReference::parse(bytes)?)
        }
        msg_type::SYMBOL_INDEX_MAPPING => {
            ControlMessage::SymbolIndexMapping(Box::new(SymbolIndexMapping::parse(bytes)?))
        }
        msg_type::SYMBOL_CLEAR => ControlMessage::SymbolClear(SymbolClear::parse(bytes)?),
        msg_type::SECURITY_STATUS => ControlMessage::SecurityStatus(SecurityStatus::parse(bytes)?),
        msg_type::REFRESH_HEADER => ControlMessage::RefreshHeader(RefreshHeader::parse(bytes)?),
        msg_type::MESSAGE_UNAVAILABLE => {
            ControlMessage::MessageUnavailable(MessageUnavailable::parse(bytes)?)
        }
        _ => return Ok(None),
    }))
}

// ── Encoders (tests, capture tooling, and the request-server client) ─────────

use exchange_core::wire::Writer;

fn message_header(w: &mut Writer, size: u16, ty: u16) {
    w.le_u16(size).le_u16(ty);
}

pub fn encode_sequence_number_reset(
    source_time_secs: u32,
    source_time_nanos: u32,
    product_id: u8,
    channel_id: u8,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(SequenceNumberReset::SIZE);
    message_header(
        &mut w,
        SequenceNumberReset::SIZE as u16,
        msg_type::SEQUENCE_NUMBER_RESET,
    );
    w.le_u32(source_time_secs)
        .le_u32(source_time_nanos)
        .u8(product_id)
        .u8(channel_id);
    w.into_vec()
}

pub fn encode_source_time_reference(
    id: u32,
    symbol_seq_num: u32,
    source_time_secs: u32,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(SourceTimeReference::SIZE);
    message_header(
        &mut w,
        SourceTimeReference::SIZE as u16,
        msg_type::SOURCE_TIME_REFERENCE,
    );
    w.le_u32(id).le_u32(symbol_seq_num).le_u32(source_time_secs);
    w.into_vec()
}

pub fn encode_symbol_index_mapping(m: &SymbolIndexMapping) -> Vec<u8> {
    let mut w = Writer::with_capacity(SymbolIndexMapping::SIZE);
    message_header(
        &mut w,
        SymbolIndexMapping::SIZE as u16,
        msg_type::SYMBOL_INDEX_MAPPING,
    );
    w.le_u32(m.symbol_index)
        .nul_padded(&m.symbol, 11)
        .u8(0) // Reserved
        .le_u16(m.market_id)
        .u8(m.system_id)
        .ascii_char(m.exchange_code)
        .u8(m.price_scale_code)
        .ascii_char(m.security_type)
        .le_u16(m.lot_size)
        .le_u32(m.prev_close_price.to_xdp(m.price_scale_code) as u32)
        .le_u32(m.prev_close_volume)
        .u8(m.price_resolution)
        .ascii_char(m.round_lot)
        .le_u16(m.mpv)
        .le_u16(m.unit_of_trade)
        .le_u16(0); // Reserved
    w.into_vec()
}

pub fn encode_symbol_clear(m: &SymbolClear) -> Vec<u8> {
    let mut w = Writer::with_capacity(SymbolClear::SIZE);
    message_header(&mut w, SymbolClear::SIZE as u16, msg_type::SYMBOL_CLEAR);
    w.le_u32(m.source_time_secs)
        .le_u32(m.source_time_nanos)
        .le_u32(m.symbol_index)
        .le_u32(m.next_source_seq_num);
    w.into_vec()
}

pub fn encode_security_status(m: &SecurityStatus, status: char, halt: char) -> Vec<u8> {
    let mut w = Writer::with_capacity(SecurityStatus::SIZE);
    message_header(
        &mut w,
        SecurityStatus::SIZE as u16,
        msg_type::SECURITY_STATUS,
    );
    w.le_u32(m.source_time_secs)
        .le_u32(m.source_time_nanos)
        .le_u32(m.symbol_index)
        .le_u32(m.symbol_seq_num)
        .ascii_char(status)
        .ascii_char(halt)
        .le_u32(0) // Reserved
        .le_u32(m.price1_raw as u32)
        .le_u32(m.price2_raw as u32)
        .ascii_char(m.ssr_triggering_exchange_id)
        .le_u32(m.ssr_triggering_volume)
        .le_u32(m.time)
        .ascii_char(m.ssr_state)
        .ascii_char(m.market_state)
        .u8(0); // SessionState, unused
    w.into_vec()
}

pub fn encode_refresh_header(m: &RefreshHeader) -> Vec<u8> {
    let mut w = Writer::with_capacity(RefreshHeader::SIZE);
    message_header(&mut w, RefreshHeader::SIZE as u16, msg_type::REFRESH_HEADER);
    w.le_u32(m.source_time_secs)
        .le_u32(m.source_time_nanos)
        .le_u32(m.last_seq_num)
        .le_u32(m.symbol_index);
    w.into_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_mapping() -> SymbolIndexMapping {
        SymbolIndexMapping {
            symbol_index: 4242,
            symbol: "IBM".into(),
            market_id: 1,
            system_id: 3,
            exchange_code: 'N',
            price_scale_code: 4,
            security_type: 'C',
            lot_size: 100,
            prev_close_price: Price::from_xdp(1_805_000, 4),
            prev_close_volume: 3_210_000,
            price_resolution: 0,
            round_lot: 'Y',
            mpv: 1,
            unit_of_trade: 100,
        }
    }

    #[test]
    fn symbol_index_mapping_round_trips_at_forty_four_bytes() {
        let m = sample_mapping();
        let bytes = encode_symbol_index_mapping(&m);
        assert_eq!(bytes.len(), SymbolIndexMapping::SIZE);
        assert_eq!(SymbolIndexMapping::parse(&bytes).unwrap(), m);
    }

    #[test]
    fn price_scale_code_governs_how_prices_decode() {
        let mut m = sample_mapping();
        m.price_scale_code = 6;
        m.prev_close_price = Price::from_xdp(180_500_000, 6);
        let bytes = encode_symbol_index_mapping(&m);
        let back = SymbolIndexMapping::parse(&bytes).unwrap();
        assert_eq!(back.price_scale_code, 6);
        assert_eq!(back.prev_close_price.to_string(), "180.50");
    }

    #[test]
    fn mpv_is_hundredths_of_a_cent() {
        let mut m = sample_mapping();
        m.mpv = 1;
        assert_eq!(m.minimum_price_variation().to_string(), "0.0001");
        assert_eq!(m.minimum_price_variation().raw(), 100_000);
        // A tick-pilot stock quoted in nickels.
        m.mpv = 500;
        assert_eq!(m.minimum_price_variation().to_string(), "0.05");
    }

    #[test]
    fn test_symbols_are_identifiable() {
        let mut m = sample_mapping();
        assert!(!m.is_test_symbol());
        m.security_type = 'T';
        assert!(m.is_test_symbol());
    }

    #[test]
    fn symbol_is_read_up_to_its_null_terminator() {
        let mut m = sample_mapping();
        m.symbol = "BRK A".into();
        let bytes = encode_symbol_index_mapping(&m);
        assert_eq!(SymbolIndexMapping::parse(&bytes).unwrap().symbol, "BRK A");
    }

    #[test]
    fn sequence_number_reset_round_trips() {
        let bytes = encode_sequence_number_reset(1_700_000_000, 42, 7, 3);
        assert_eq!(bytes.len(), SequenceNumberReset::SIZE);
        let m = SequenceNumberReset::parse(&bytes).unwrap();
        assert_eq!(m.source_time_secs, 1_700_000_000);
        assert_eq!(m.product_id, 7);
        assert_eq!(m.channel_id, 3);
    }

    #[test]
    fn source_time_reference_round_trips() {
        let bytes = encode_source_time_reference(5, 0, 1_700_000_000);
        assert_eq!(bytes.len(), SourceTimeReference::SIZE);
        let m = SourceTimeReference::parse(&bytes).unwrap();
        assert_eq!(m.id, 5);
        assert_eq!(m.source_time_secs, 1_700_000_000);
    }

    #[test]
    fn symbol_clear_round_trips() {
        let m = SymbolClear {
            source_time_secs: 1,
            source_time_nanos: 2,
            symbol_index: 3,
            next_source_seq_num: 4,
        };
        let bytes = encode_symbol_clear(&m);
        assert_eq!(bytes.len(), SymbolClear::SIZE);
        assert_eq!(SymbolClear::parse(&bytes).unwrap(), m);
    }

    #[test]
    fn security_status_round_trips_at_forty_six_bytes() {
        let m = SecurityStatus {
            source_time_secs: 1_700_000_000,
            source_time_nanos: 1,
            symbol_index: 4242,
            symbol_seq_num: 99,
            status: SecurityStatusCode::TradingHalt,
            halt_condition: HaltCondition::LuldPause,
            price1_raw: 0,
            price2_raw: 0,
            ssr_triggering_exchange_id: ' ',
            ssr_triggering_volume: 0,
            time: 0,
            ssr_state: '~',
            market_state: 'O',
        };
        let bytes = encode_security_status(&m, '4', 'M');
        assert_eq!(bytes.len(), SecurityStatus::SIZE);
        assert_eq!(SecurityStatus::parse(&bytes).unwrap(), m);
    }

    #[test]
    fn security_status_multiplexes_four_state_machines_on_one_byte() {
        assert_eq!(
            SecurityStatusCode::parse('4'),
            SecurityStatusCode::TradingHalt
        );
        assert_eq!(
            SecurityStatusCode::parse('A'),
            SecurityStatusCode::SsrActivated
        );
        assert_eq!(
            SecurityStatusCode::parse('O'),
            SecurityStatusCode::CoreSession
        );
        assert_eq!(
            SecurityStatusCode::parse('G'),
            SecurityStatusCode::PreOpeningPriceIndication
        );
        assert!(SecurityStatusCode::parse('I').is_price_indication());
        assert!(!SecurityStatusCode::parse('4').is_price_indication());
    }

    #[test]
    fn short_sale_restriction_state_is_read_from_its_own_field() {
        let mut m = SecurityStatus {
            source_time_secs: 0,
            source_time_nanos: 0,
            symbol_index: 1,
            symbol_seq_num: 1,
            status: SecurityStatusCode::SsrActivated,
            halt_condition: HaltCondition::NotHalted,
            price1_raw: 1_234_500,
            price2_raw: 0,
            ssr_triggering_exchange_id: 'N',
            ssr_triggering_volume: 100,
            time: 93_015_000,
            ssr_state: 'E',
            market_state: 'O',
        };
        let bytes = encode_security_status(&m, 'A', '~');
        let back = SecurityStatus::parse(&bytes).unwrap();
        assert!(back.short_sale_restricted());
        assert_eq!(back.price1_raw, 1_234_500);
        assert!(!back.halt_condition.is_halted());

        m.ssr_state = '~';
        let bytes = encode_security_status(&m, 'D', '~');
        assert!(!SecurityStatus::parse(&bytes)
            .unwrap()
            .short_sale_restricted());
    }

    #[test]
    fn halt_conditions_include_market_wide_circuit_breakers() {
        assert_eq!(
            HaltCondition::parse('2'),
            HaltCondition::MarketWideCircuitBreaker(2)
        );
        assert!(HaltCondition::parse('2').is_halted());
        assert!(!HaltCondition::parse('~').is_halted());
    }

    #[test]
    fn refresh_header_carries_the_resume_point() {
        let m = RefreshHeader {
            source_time_secs: 1,
            source_time_nanos: 2,
            last_seq_num: 987_654,
            symbol_index: 4242,
        };
        let bytes = encode_refresh_header(&m);
        assert_eq!(bytes.len(), RefreshHeader::SIZE);
        assert_eq!(RefreshHeader::parse(&bytes).unwrap().last_seq_num, 987_654);
    }

    #[test]
    fn decode_control_ignores_data_message_types() {
        let bytes = encode_symbol_clear(&SymbolClear {
            source_time_secs: 1,
            source_time_nanos: 2,
            symbol_index: 3,
            next_source_seq_num: 4,
        });
        assert!(decode_control(msg_type::SYMBOL_CLEAR, &bytes)
            .unwrap()
            .is_some());
        assert!(decode_control(100, &bytes).unwrap().is_none());
    }

    #[test]
    fn a_short_control_message_is_rejected() {
        let bytes = encode_symbol_clear(&SymbolClear {
            source_time_secs: 1,
            source_time_nanos: 2,
            symbol_index: 3,
            next_source_seq_num: 4,
        });
        assert!(SymbolClear::parse(&bytes[..bytes.len() - 1]).is_err());
    }
}
