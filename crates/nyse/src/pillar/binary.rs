//! NYSE Pillar Binary Gateway — the native order-entry protocol for NYSE, NYSE American,
//! NYSE Arca, NYSE National and NYSE Texas equities.
//!
//! Structure, from the outside in:
//!
//! ```text
//!   SeqMsg (type 0x0905, ≥32 bytes)
//!   ├─ msghdr    MsgHeader   type + total length
//!   ├─ seqmsg    SeqMsgId    stream id (8) + sequence (8)   ← globally unique, forever
//!   ├─ reserved  u32
//!   ├─ timestamp Timestamp   nanoseconds since the UNIX epoch
//!   └─ payload   MsgHeader   the application message starts here with its own header
//! ```
//!
//! Every message begins with a 4-byte `MsgHeader` of `{type: u16, length: u16}` where
//! `length` counts the header itself plus everything after it — including optional add-ons,
//! which is what makes several message types variable length.
//!
//! Wire conventions differ from Nasdaq in every respect: **little endian**, `zchar(n)`
//! strings NUL padded, prices as unsigned 64-bit at a fixed scale of 8 (`123000000` is
//! `$1.23`), and timestamps as nanoseconds since the UNIX epoch rather than since midnight.
//!
//! Order attributes are not fields but a 64-bit bitfield, `BitfieldOrderInstructions`.
//! Side, order type, time in force, capacity, routing and a dozen other instructions are
//! packed into it, so [`OrderInstructions`] exists to make that packing checkable rather
//! than a hand-rolled shift at each call site.

use exchange_core::wire::{Cursor, Writer};
use exchange_core::{Mpid4, Price, UserData8, WireError, WireResult};

const PROTOCOL: &str = "Pillar Binary";

/// Application message type codes.
pub mod msg_type {
    /// Envelope carrying every sequenced application message.
    pub const SEQ_MSG: u16 = 0x0905;
    /// Sequenced filler; advances the stream sequence with no business meaning.
    pub const SEQUENCED_FILLER: u16 = 0x0282;
    /// New Order Single, and Cancel/Replace when `OrigClOrdID` is non-zero.
    pub const NEW_ORDER: u16 = 0x0240;
    pub const ORDER_CANCEL_REQUEST: u16 = 0x0280;
    pub const ORDER_ACK: u16 = 0x0260;
    pub const EXECUTION_REPORT: u16 = 0x0290;
    pub const TRADE_BUST_CORRECT: u16 = 0x0292;
    /// Optional add-on appended to an order or its acknowledgement.
    pub const OPTIONAL_ORDER_ADD_ON: u16 = 0x0241;
}

/// `MsgHeader` — the 4 bytes every Pillar message starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgHeader {
    pub msg_type: u16,
    /// Total message length including this header and any add-ons.
    pub length: u16,
}

impl MsgHeader {
    pub const SIZE: usize = 4;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        let mut c = Cursor::new(bytes);
        Ok(Self {
            msg_type: c.le_u16()?,
            length: c.le_u16()?,
        })
    }

    fn write(&self, w: &mut Writer) {
        w.le_u16(self.msg_type).le_u16(self.length);
    }
}

/// `SeqMsgId` — stream identifier plus sequence number, globally unique across all firms
/// and indefinitely unique across time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqMsgId {
    pub stream_id: u64,
    /// Starts at 1 on each stream.
    pub sequence: u64,
}

/// `SeqMsg` — the envelope around every application message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqMsg {
    pub id: SeqMsgId,
    /// Nanoseconds since the UNIX epoch.
    pub timestamp: u64,
    /// The application payload, starting with its own `MsgHeader`.
    pub payload: Vec<u8>,
}

impl SeqMsg {
    /// Bytes before the payload.
    pub const PAYLOAD_OFFSET: usize = 32;

    pub fn new(id: SeqMsgId, timestamp: u64, payload: Vec<u8>) -> Self {
        Self {
            id,
            timestamp,
            payload,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let total = Self::PAYLOAD_OFFSET + self.payload.len();
        let mut w = Writer::with_capacity(total);
        MsgHeader {
            msg_type: msg_type::SEQ_MSG,
            length: total as u16,
        }
        .write(&mut w);
        w.le_u64(self.id.stream_id)
            .le_u64(self.id.sequence)
            .le_u32(0) // reserved
            .le_u64(self.timestamp)
            .raw(&self.payload);
        w.into_vec()
    }

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        let header = MsgHeader::parse(bytes)?;
        if header.msg_type != msg_type::SEQ_MSG {
            return Err(WireError::UnknownMessageType {
                protocol: PROTOCOL,
                got: header.msg_type,
                got_ascii: '?',
            });
        }
        let declared = header.length as usize;
        if declared < Self::PAYLOAD_OFFSET || bytes.len() < declared {
            return Err(WireError::LengthMismatch {
                protocol: PROTOCOL,
                msg_type: header.msg_type,
                declared,
                expected: Self::PAYLOAD_OFFSET,
            });
        }
        let mut c = Cursor::at(bytes, 4);
        let stream_id = c.le_u64()?;
        let sequence = c.le_u64()?;
        c.skip(4)?; // reserved
        let timestamp = c.le_u64()?;
        Ok(Self {
            id: SeqMsgId {
                stream_id,
                sequence,
            },
            timestamp,
            payload: bytes[Self::PAYLOAD_OFFSET..declared].to_vec(),
        })
    }

    /// The payload's message type, without copying the payload.
    pub fn payload_type(&self) -> WireResult<u16> {
        MsgHeader::parse(&self.payload).map(|h| h.msg_type)
    }
}

// ─── BitfieldOrderInstructions ──────────────────────────────────────────────

/// Bit positions and widths within `BitfieldOrderInstructions`, straight from the spec's
/// data-structure table.
mod bits {
    pub const SUB_ID_INDICATOR: (u32, u32) = (12, 1);
    pub const SPECIAL_ORD_TYPE: (u32, u32) = (13, 4);
    pub const LOCATE_REQD: (u32, u32) = (17, 1);
    pub const RETAIL_INDICATOR: (u32, u32) = (18, 1);
    pub const ATTRIBUTED_QUOTE: (u32, u32) = (19, 3);
    pub const ORDER_CAPACITY: (u32, u32) = (22, 3);
    pub const INTEREST_TYPE: (u32, u32) = (25, 3);
    pub const TRADING_SESSION_ID: (u32, u32) = (28, 3);
    pub const TIME_IN_FORCE: (u32, u32) = (31, 3);
    pub const PROACTIVE_IF_LOCKED: (u32, u32) = (34, 3);
    pub const SELF_TRADE_TYPE: (u32, u32) = (37, 3);
    pub const CANCEL_INSTEAD_OF_REPRICE: (u32, u32) = (40, 4);
    pub const ROUTING_INST: (u32, u32) = (44, 4);
    pub const EXTENDED_EXEC_INST: (u32, u32) = (48, 4);
    pub const EXEC_INST: (u32, u32) = (52, 4);
    pub const ORD_TYPE: (u32, u32) = (56, 4);
    pub const SIDE: (u32, u32) = (60, 4);
}

/// `Side`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy = 1,
    Sell = 2,
    SellShort = 3,
    SellShortExempt = 4,
    /// NYSE Texas only.
    Cross = 5,
    CrossShort = 6,
    CrossShortExempt = 7,
}

impl Side {
    pub const fn from_code(v: u64) -> Option<Self> {
        Some(match v {
            1 => Self::Buy,
            2 => Self::Sell,
            3 => Self::SellShort,
            4 => Self::SellShortExempt,
            5 => Self::Cross,
            6 => Self::CrossShort,
            7 => Self::CrossShortExempt,
            _ => return None,
        })
    }

    /// True for the sides that require `LocateReqd = 0`.
    pub const fn is_short(self) -> bool {
        matches!(
            self,
            Self::SellShort | Self::SellShortExempt | Self::CrossShort | Self::CrossShortExempt
        )
    }
}

/// `OrdType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdType {
    Market = 1,
    Limit = 2,
    InsideLimit = 3,
    Pegged = 4,
}

impl OrdType {
    pub const fn from_code(v: u64) -> Option<Self> {
        Some(match v {
            1 => Self::Market,
            2 => Self::Limit,
            3 => Self::InsideLimit,
            4 => Self::Pegged,
            _ => return None,
        })
    }
}

/// `TimeInForce`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInForce {
    Day = 1,
    Ioc = 2,
    AtTheOpening = 3,
    OnClose = 4,
}

impl TimeInForce {
    pub const fn from_code(v: u64) -> Option<Self> {
        Some(match v {
            1 => Self::Day,
            2 => Self::Ioc,
            3 => Self::AtTheOpening,
            4 => Self::OnClose,
            _ => return None,
        })
    }
}

/// `OrderCapacity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderCapacity {
    Agency = 1,
    Principal = 2,
    RisklessPrincipal = 3,
    /// NYSE Floor Broker only.
    ErrorAccount = 4,
}

impl OrderCapacity {
    pub const fn from_code(v: u64) -> Option<Self> {
        Some(match v {
            1 => Self::Agency,
            2 => Self::Principal,
            3 => Self::RisklessPrincipal,
            4 => Self::ErrorAccount,
            _ => return None,
        })
    }
}

/// `TradingSessionID`. Combinations are distinct values, not a bit mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingSession {
    Overnight = 0,
    Early = 1,
    Core = 2,
    Late = 3,
    EarlyAndCore = 4,
    CoreAndLate = 5,
    EarlyCoreAndLate = 6,
    OvernightEarlyCoreAndLate = 7,
}

impl TradingSession {
    pub const fn from_code(v: u64) -> Option<Self> {
        Some(match v {
            0 => Self::Overnight,
            1 => Self::Early,
            2 => Self::Core,
            3 => Self::Late,
            4 => Self::EarlyAndCore,
            5 => Self::CoreAndLate,
            6 => Self::EarlyCoreAndLate,
            7 => Self::OvernightEarlyCoreAndLate,
            _ => return None,
        })
    }
}

/// `SelfTradeType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfTradeType {
    /// Use the session configuration's setting for this username.
    SessionDefault = 0,
    None = 1,
    CancelNewest = 2,
    CancelOldest = 3,
    CancelBoth = 4,
    CancelDecrement = 5,
}

/// `RoutingInst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingInst {
    None = 0,
    NonRoutable = 1,
    Routable = 2,
    DirectedPrimaryOnly = 3,
    DirectedAndRoutable = 4,
    PrimaryUntil0945 = 5,
    PrimaryAfter1555 = 6,
    PrimaryUntil0945AndAfter1555 = 7,
    /// Requires a non-zero `MinQty`.
    MinimumFill = 8,
    RouteToAts = 10,
}

/// `ExecInst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecInst {
    None = 0,
    TrackingOrder = 3,
    /// Intermarket sweep order.
    Iso = 4,
    PrimaryPeg = 5,
    MarketPeg = 6,
    MidpointLiquidity = 7,
    NonDisplayed = 8,
    TradeAtIso = 9,
    LastSalePeg = 10,
}

/// `ExtendedExecInst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedExecInst {
    None = 0,
    /// Add liquidity only.
    Alo = 1,
    NoRouteToIoi = 3,
    RetailType1 = 5,
    RetailType2 = 6,
    RetailProvider = 7,
    ImbalanceOffset = 8,
    DiscretionaryPeg = 9,
    DarkPrimaryPeg = 10,
    IssuerDirectOffering = 14,
}

/// The order-instruction bitfield, decomposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderInstructions {
    pub side: Side,
    pub ord_type: OrdType,
    pub time_in_force: TimeInForce,
    pub capacity: OrderCapacity,
    pub trading_session: TradingSession,
    pub exec_inst: ExecInst,
    pub extended_exec_inst: ExtendedExecInst,
    pub routing_inst: RoutingInst,
    pub self_trade_type: SelfTradeType,
    /// Must be false; short sides are rejected when set.
    pub locate_reqd: bool,
    pub retail_indicator: bool,
    /// 0 not attributed, 1 attributed to market data, 2 include in broker volume, 3 both.
    pub attributed_quote: u8,
    /// 0 use MPSubID for self-trade prevention, 1 evaluate at MPID level only.
    pub sub_id_indicator: bool,
    pub special_ord_type: u8,
    pub interest_type: u8,
    pub proactive_if_locked: u8,
    /// 0 default, 1 cancel instead of repricing for LULD, 3 for any reason.
    pub cancel_instead_of_reprice: u8,
}

impl Default for OrderInstructions {
    /// A plain agency day limit buy for the core session — the least surprising order.
    fn default() -> Self {
        Self {
            side: Side::Buy,
            ord_type: OrdType::Limit,
            time_in_force: TimeInForce::Day,
            capacity: OrderCapacity::Agency,
            trading_session: TradingSession::Core,
            exec_inst: ExecInst::None,
            extended_exec_inst: ExtendedExecInst::None,
            routing_inst: RoutingInst::None,
            self_trade_type: SelfTradeType::SessionDefault,
            locate_reqd: false,
            retail_indicator: false,
            attributed_quote: 0,
            sub_id_indicator: false,
            special_ord_type: 0,
            interest_type: 0,
            proactive_if_locked: 0,
            cancel_instead_of_reprice: 0,
        }
    }
}

#[inline]
const fn mask(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

#[inline]
fn put(bitfield: &mut u64, field: (u32, u32), value: u64) {
    let (offset, width) = field;
    *bitfield &= !(mask(width) << offset);
    *bitfield |= (value & mask(width)) << offset;
}

#[inline]
const fn get(bitfield: u64, field: (u32, u32)) -> u64 {
    let (offset, width) = field;
    (bitfield >> offset) & mask(width)
}

impl OrderInstructions {
    /// Pack into the 64-bit wire representation.
    pub fn to_bits(self) -> u64 {
        let mut b = 0u64;
        put(&mut b, bits::SIDE, self.side as u64);
        put(&mut b, bits::ORD_TYPE, self.ord_type as u64);
        put(&mut b, bits::TIME_IN_FORCE, self.time_in_force as u64);
        put(&mut b, bits::ORDER_CAPACITY, self.capacity as u64);
        put(
            &mut b,
            bits::TRADING_SESSION_ID,
            self.trading_session as u64,
        );
        put(&mut b, bits::EXEC_INST, self.exec_inst as u64);
        put(
            &mut b,
            bits::EXTENDED_EXEC_INST,
            self.extended_exec_inst as u64,
        );
        put(&mut b, bits::ROUTING_INST, self.routing_inst as u64);
        put(&mut b, bits::SELF_TRADE_TYPE, self.self_trade_type as u64);
        put(&mut b, bits::LOCATE_REQD, self.locate_reqd as u64);
        put(&mut b, bits::RETAIL_INDICATOR, self.retail_indicator as u64);
        put(&mut b, bits::ATTRIBUTED_QUOTE, self.attributed_quote as u64);
        put(&mut b, bits::SUB_ID_INDICATOR, self.sub_id_indicator as u64);
        put(&mut b, bits::SPECIAL_ORD_TYPE, self.special_ord_type as u64);
        put(&mut b, bits::INTEREST_TYPE, self.interest_type as u64);
        put(
            &mut b,
            bits::PROACTIVE_IF_LOCKED,
            self.proactive_if_locked as u64,
        );
        put(
            &mut b,
            bits::CANCEL_INSTEAD_OF_REPRICE,
            self.cancel_instead_of_reprice as u64,
        );
        b
    }

    /// Unpack from the wire representation.
    ///
    /// Enum fields that carry a value the specification does not define are an error rather
    /// than a silent default: an unrecognised side or order type on an acknowledgement means
    /// the message is not what this code thinks it is.
    pub fn from_bits(b: u64) -> WireResult<Self> {
        let side = Side::from_code(get(b, bits::SIDE)).ok_or(WireError::InvalidEnum {
            protocol: PROTOCOL,
            field: "Side",
            value: '?',
        })?;
        let ord_type =
            OrdType::from_code(get(b, bits::ORD_TYPE)).ok_or(WireError::InvalidEnum {
                protocol: PROTOCOL,
                field: "OrdType",
                value: '?',
            })?;
        let time_in_force =
            TimeInForce::from_code(get(b, bits::TIME_IN_FORCE)).ok_or(WireError::InvalidEnum {
                protocol: PROTOCOL,
                field: "TimeInForce",
                value: '?',
            })?;
        let capacity = OrderCapacity::from_code(get(b, bits::ORDER_CAPACITY)).ok_or(
            WireError::InvalidEnum {
                protocol: PROTOCOL,
                field: "OrderCapacity",
                value: '?',
            },
        )?;
        let trading_session = TradingSession::from_code(get(b, bits::TRADING_SESSION_ID)).ok_or(
            WireError::InvalidEnum {
                protocol: PROTOCOL,
                field: "TradingSessionID",
                value: '?',
            },
        )?;

        Ok(Self {
            side,
            ord_type,
            time_in_force,
            capacity,
            trading_session,
            exec_inst: match get(b, bits::EXEC_INST) {
                0 => ExecInst::None,
                3 => ExecInst::TrackingOrder,
                4 => ExecInst::Iso,
                5 => ExecInst::PrimaryPeg,
                6 => ExecInst::MarketPeg,
                7 => ExecInst::MidpointLiquidity,
                8 => ExecInst::NonDisplayed,
                9 => ExecInst::TradeAtIso,
                10 => ExecInst::LastSalePeg,
                _ => ExecInst::None,
            },
            extended_exec_inst: match get(b, bits::EXTENDED_EXEC_INST) {
                1 => ExtendedExecInst::Alo,
                3 => ExtendedExecInst::NoRouteToIoi,
                5 => ExtendedExecInst::RetailType1,
                6 => ExtendedExecInst::RetailType2,
                7 => ExtendedExecInst::RetailProvider,
                8 => ExtendedExecInst::ImbalanceOffset,
                9 => ExtendedExecInst::DiscretionaryPeg,
                10 => ExtendedExecInst::DarkPrimaryPeg,
                14 => ExtendedExecInst::IssuerDirectOffering,
                _ => ExtendedExecInst::None,
            },
            routing_inst: match get(b, bits::ROUTING_INST) {
                1 => RoutingInst::NonRoutable,
                2 => RoutingInst::Routable,
                3 => RoutingInst::DirectedPrimaryOnly,
                4 => RoutingInst::DirectedAndRoutable,
                5 => RoutingInst::PrimaryUntil0945,
                6 => RoutingInst::PrimaryAfter1555,
                7 => RoutingInst::PrimaryUntil0945AndAfter1555,
                8 => RoutingInst::MinimumFill,
                10 => RoutingInst::RouteToAts,
                _ => RoutingInst::None,
            },
            self_trade_type: match get(b, bits::SELF_TRADE_TYPE) {
                1 => SelfTradeType::None,
                2 => SelfTradeType::CancelNewest,
                3 => SelfTradeType::CancelOldest,
                4 => SelfTradeType::CancelBoth,
                5 => SelfTradeType::CancelDecrement,
                _ => SelfTradeType::SessionDefault,
            },
            locate_reqd: get(b, bits::LOCATE_REQD) == 1,
            retail_indicator: get(b, bits::RETAIL_INDICATOR) == 1,
            attributed_quote: get(b, bits::ATTRIBUTED_QUOTE) as u8,
            sub_id_indicator: get(b, bits::SUB_ID_INDICATOR) == 1,
            special_ord_type: get(b, bits::SPECIAL_ORD_TYPE) as u8,
            interest_type: get(b, bits::INTEREST_TYPE) as u8,
            proactive_if_locked: get(b, bits::PROACTIVE_IF_LOCKED) as u8,
            cancel_instead_of_reprice: get(b, bits::CANCEL_INSTEAD_OF_REPRICE) as u8,
        })
    }

    /// Catch instruction combinations Pillar rejects, before spending a round trip.
    pub fn validate(&self) -> Result<(), String> {
        if self.side.is_short() && self.locate_reqd {
            return Err(
                "short sides must be entered with LocateReqd = 0; Pillar rejects otherwise".into(),
            );
        }
        if matches!(self.routing_inst, RoutingInst::MinimumFill) {
            // MinQty lives on the message, not the bitfield, so the message-level validator
            // owns the numeric half of this rule.
        }
        if self.attributed_quote > 3 {
            return Err(format!(
                "AttributedQuote is a 3-bit field with values 0..=3, got {}",
                self.attributed_quote
            ));
        }
        Ok(())
    }
}

// ─── Firm → Exchange ────────────────────────────────────────────────────────

/// New Order Single, or Cancel/Replace when `orig_cl_ord_id` is non-zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOrder {
    pub symbol_id: u32,
    /// Firm identifier. On a Cancel/Replace this must match the original order's MPID.
    pub mpid: Mpid4,
    /// Integer market-maker identifier agreed with the exchange; 0 when not applicable.
    pub mmid: u32,
    /// Single character identifying the desk within the firm.
    pub mp_sub_id: char,
    /// Firm-assigned order id, unique per (username + MPID) for the whole trading day.
    pub cl_ord_id: u64,
    /// 0 for a new order; the previous `ClOrdID` for a Cancel/Replace.
    pub orig_cl_ord_id: u64,
    pub instructions: OrderInstructions,
    pub price: Price,
    pub order_qty: u32,
    /// 0 for no minimum; otherwise must not exceed `order_qty`.
    pub min_qty: u32,
    /// Up to 8 printable ASCII characters, excluding `, ; | @ < > &` and quotes.
    pub user_data: UserData8,
}

impl NewOrder {
    /// Minimum length; add-ons make the message longer.
    pub const MIN_LEN: usize = 65;

    /// Maximum order quantity accepted on any NYSE Group equities market.
    pub const MAX_ORDER_QTY: u32 = 999_999_999;

    pub fn is_replace(&self) -> bool {
        self.orig_cl_ord_id != 0
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::MIN_LEN);
        MsgHeader {
            msg_type: msg_type::NEW_ORDER,
            length: Self::MIN_LEN as u16,
        }
        .write(&mut w);
        w.le_u32(self.symbol_id)
            .raw(self.mpid.as_bytes())
            .le_u32(self.mmid)
            .ascii_char(self.mp_sub_id)
            .le_u64(self.cl_ord_id)
            .le_u64(self.orig_cl_ord_id)
            .le_u64(self.instructions.to_bits())
            .le_u64(self.price.to_pillar())
            .le_u32(self.order_qty)
            .le_u32(self.min_qty)
            .raw(self.user_data.as_bytes());
        w.into_vec()
    }

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        let header = MsgHeader::parse(bytes)?;
        if header.msg_type != msg_type::NEW_ORDER {
            return Err(WireError::UnknownMessageType {
                protocol: PROTOCOL,
                got: header.msg_type,
                got_ascii: '?',
            });
        }
        if bytes.len() < Self::MIN_LEN {
            return Err(WireError::LengthMismatch {
                protocol: PROTOCOL,
                msg_type: header.msg_type,
                declared: bytes.len(),
                expected: Self::MIN_LEN,
            });
        }
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            symbol_id: c.le_u32()?,
            mpid: Mpid4::from_wire(c.take(4)?),
            mmid: c.le_u32()?,
            mp_sub_id: c.ascii_char()?,
            cl_ord_id: c.le_u64()?,
            orig_cl_ord_id: c.le_u64()?,
            instructions: OrderInstructions::from_bits(c.le_u64()?)?,
            price: Price::from_pillar(c.le_u64()?),
            order_qty: c.le_u32()?,
            min_qty: c.le_u32()?,
            user_data: UserData8::from_wire(c.take(8)?),
        })
    }

    /// Reject an order Pillar would reject.
    pub fn validate(&self) -> Result<(), String> {
        self.instructions.validate()?;

        if self.order_qty == 0 || self.order_qty > Self::MAX_ORDER_QTY {
            return Err(format!(
                "OrderQty must be 1..={}, got {}",
                Self::MAX_ORDER_QTY,
                self.order_qty
            ));
        }
        if self.min_qty > self.order_qty {
            return Err(format!(
                "MinQty {} exceeds OrderQty {}",
                self.min_qty, self.order_qty
            ));
        }
        if matches!(self.instructions.routing_inst, RoutingInst::MinimumFill) && self.min_qty == 0 {
            return Err("RoutingInst = Minimum Fill requires a non-zero MinQty".into());
        }
        if self.mpid.len() > 4 {
            return Err(format!("MPID {:?} exceeds the 4-byte field", self.mpid));
        }
        if self.user_data.len() > 8 {
            return Err(format!(
                "UserData {:?} exceeds the 8-byte field",
                self.user_data
            ));
        }
        if let Some(bad) = self.user_data.as_str().chars().find(|c| {
            !c.is_ascii_graphic()
                || matches!(c, ',' | ';' | '|' | '@' | '<' | '>' | '&' | '"' | '\'')
        }) {
            return Err(format!(
                "UserData contains the disallowed character {bad:?}"
            ));
        }
        if self.cl_ord_id == 0 {
            return Err("ClOrdID must be non-zero; 0 marks a message as a new order".into());
        }
        // A limit order at zero would be accepted as a price of $0.00 rather than rejected
        // as an obvious mistake, so catch it here.
        if matches!(
            self.instructions.ord_type,
            OrdType::Limit | OrdType::InsideLimit
        ) && self.price.is_zero()
        {
            return Err("limit orders require a non-zero price".into());
        }
        Ok(())
    }
}

/// Order Cancel Request — cancels a single targeted order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelRequest {
    pub symbol_id: u32,
    /// Must match the MPID of the order being cancelled.
    pub mpid: Mpid4,
    pub cl_ord_id: u64,
    pub orig_cl_ord_id: u64,
}

impl CancelRequest {
    pub const LEN: usize = 28;

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::LEN);
        MsgHeader {
            msg_type: msg_type::ORDER_CANCEL_REQUEST,
            length: Self::LEN as u16,
        }
        .write(&mut w);
        w.le_u32(self.symbol_id)
            .raw(self.mpid.as_bytes())
            .le_u64(self.cl_ord_id)
            .le_u64(self.orig_cl_ord_id);
        w.into_vec()
    }

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        let header = MsgHeader::parse(bytes)?;
        if header.msg_type != msg_type::ORDER_CANCEL_REQUEST || bytes.len() < Self::LEN {
            return Err(WireError::LengthMismatch {
                protocol: PROTOCOL,
                msg_type: header.msg_type,
                declared: bytes.len(),
                expected: Self::LEN,
            });
        }
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            symbol_id: c.le_u32()?,
            mpid: Mpid4::from_wire(c.take(4)?),
            cl_ord_id: c.le_u64()?,
            orig_cl_ord_id: c.le_u64()?,
        })
    }
}

// ─── Exchange → Firm ────────────────────────────────────────────────────────

/// `AckType` on an Order Acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckType {
    NewInterest,
    OrderPriorityUpdateNewOrderId,
    OrderPriorityUpdateSameOrderId,
    BulkCancelAck,
    PendingCancel,
    PendingReplace,
    PendingModify,
    Replaced,
    Modified,
    EligibleForCross,
    Canceled,
    DoneForDay,
    BillableCancelAddingLiquidity,
    BillableCancelRemovingLiquidity,
    BillableCancelSubDollarAdding,
    BillableCancelSubDollarRemoving,
    /// A value the specification does not define yet; carried through rather than dropped.
    Unknown(u8),
}

impl AckType {
    pub const fn from_code(v: u8) -> Self {
        match v {
            1 => Self::NewInterest,
            2 => Self::OrderPriorityUpdateNewOrderId,
            3 => Self::OrderPriorityUpdateSameOrderId,
            4 => Self::BulkCancelAck,
            5 => Self::PendingCancel,
            6 => Self::PendingReplace,
            7 => Self::PendingModify,
            8 => Self::Replaced,
            9 => Self::Modified,
            10 => Self::EligibleForCross,
            11 => Self::Canceled,
            12 => Self::DoneForDay,
            13 => Self::BillableCancelAddingLiquidity,
            14 => Self::BillableCancelRemovingLiquidity,
            15 => Self::BillableCancelSubDollarAdding,
            16 => Self::BillableCancelSubDollarRemoving,
            other => Self::Unknown(other),
        }
    }

    pub const fn code(self) -> u8 {
        match self {
            Self::NewInterest => 1,
            Self::OrderPriorityUpdateNewOrderId => 2,
            Self::OrderPriorityUpdateSameOrderId => 3,
            Self::BulkCancelAck => 4,
            Self::PendingCancel => 5,
            Self::PendingReplace => 6,
            Self::PendingModify => 7,
            Self::Replaced => 8,
            Self::Modified => 9,
            Self::EligibleForCross => 10,
            Self::Canceled => 11,
            Self::DoneForDay => 12,
            Self::BillableCancelAddingLiquidity => 13,
            Self::BillableCancelRemovingLiquidity => 14,
            Self::BillableCancelSubDollarAdding => 15,
            Self::BillableCancelSubDollarRemoving => 16,
            Self::Unknown(v) => v,
        }
    }

    /// True when the order is no longer working after this acknowledgement.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Canceled
                | Self::DoneForDay
                | Self::BillableCancelAddingLiquidity
                | Self::BillableCancelRemovingLiquidity
                | Self::BillableCancelSubDollarAdding
                | Self::BillableCancelSubDollarRemoving
        )
    }
}

/// Order and Cancel/Replace Acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderAck {
    /// Exchange application time, nanoseconds since the UNIX epoch.
    pub transact_time: u64,
    pub symbol_id: u32,
    pub mpid: Mpid4,
    pub mmid: u32,
    pub mp_sub_id: char,
    pub cl_ord_id: u64,
    pub orig_cl_ord_id: u64,
    pub instructions: OrderInstructions,
    pub price: Price,
    pub order_qty: u32,
    pub min_qty: u32,
    /// Exchange-assigned order id. This is the same value published on the Integrated feed
    /// as `OrderID`, which is what lets a firm find its own order in the public book.
    pub order_id: u64,
    pub leaves_qty: u32,
    pub working_price: Price,
    /// 1 when the working price differs from the displayed price.
    pub working_away_from_display: u8,
    pub pre_liquidity_indicator: Mpid4,
    /// Matching-engine reason code qualifying this event.
    pub reason_code: u16,
    pub ack_type: AckType,
    /// Bit 0 set when the inbound message was throttled.
    pub flow_indicator: u8,
}

impl OrderAck {
    pub const MIN_LEN: usize = 102;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        let header = MsgHeader::parse(bytes)?;
        if header.msg_type != msg_type::ORDER_ACK {
            return Err(WireError::UnknownMessageType {
                protocol: PROTOCOL,
                got: header.msg_type,
                got_ascii: '?',
            });
        }
        if bytes.len() < 94 {
            return Err(WireError::LengthMismatch {
                protocol: PROTOCOL,
                msg_type: header.msg_type,
                declared: bytes.len(),
                expected: 94,
            });
        }
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            transact_time: c.le_u64()?,
            symbol_id: c.le_u32()?,
            mpid: Mpid4::from_wire(c.take(4)?),
            mmid: c.le_u32()?,
            mp_sub_id: c.ascii_char()?,
            cl_ord_id: c.le_u64()?,
            orig_cl_ord_id: c.le_u64()?,
            instructions: OrderInstructions::from_bits(c.le_u64()?)?,
            price: Price::from_pillar(c.le_u64()?),
            order_qty: c.le_u32()?,
            min_qty: c.le_u32()?,
            order_id: c.le_u64()?,
            leaves_qty: c.le_u32()?,
            working_price: Price::from_pillar(c.le_u64()?),
            working_away_from_display: c.u8()?,
            pre_liquidity_indicator: Mpid4::from_wire(c.take(4)?),
            reason_code: c.le_u16()?,
            ack_type: AckType::from_code(c.u8()?),
            flow_indicator: c.u8()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::MIN_LEN);
        MsgHeader {
            msg_type: msg_type::ORDER_ACK,
            length: Self::MIN_LEN as u16,
        }
        .write(&mut w);
        w.le_u64(self.transact_time)
            .le_u32(self.symbol_id)
            .raw(self.mpid.as_bytes())
            .le_u32(self.mmid)
            .ascii_char(self.mp_sub_id)
            .le_u64(self.cl_ord_id)
            .le_u64(self.orig_cl_ord_id)
            .le_u64(self.instructions.to_bits())
            .le_u64(self.price.to_pillar())
            .le_u32(self.order_qty)
            .le_u32(self.min_qty)
            .le_u64(self.order_id)
            .le_u32(self.leaves_qty)
            .le_u64(self.working_price.to_pillar())
            .u8(self.working_away_from_display)
            .raw(self.pre_liquidity_indicator.as_bytes())
            .le_u16(self.reason_code)
            .u8(self.ack_type.code())
            .u8(self.flow_indicator);
        // Pad out to the documented minimum length; the tail is add-on space.
        let mut out = w.into_vec();
        out.resize(Self::MIN_LEN, 0);
        out[2..4].copy_from_slice(&(Self::MIN_LEN as u16).to_le_bytes());
        out
    }

    /// True when the inbound message that produced this ack was throttled.
    pub const fn was_throttled(&self) -> bool {
        self.flow_indicator & 1 == 1
    }
}

/// `ParticipantType` on an Execution Report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticipantType {
    Customer = 1,
    MarketMaker = 2,
    Dmm = 3,
    Slp = 4,
    FloorBroker = 5,
    Unknown = 0,
}

impl ParticipantType {
    pub const fn from_code(v: u8) -> Self {
        match v {
            1 => Self::Customer,
            2 => Self::MarketMaker,
            3 => Self::Dmm,
            4 => Self::Slp,
            5 => Self::FloorBroker,
            _ => Self::Unknown,
        }
    }
}

/// Execution Report — a partial or complete fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReport {
    pub transact_time: u64,
    pub symbol_id: u32,
    pub mpid: Mpid4,
    pub order_id: u64,
    pub cl_ord_id: u64,
    /// Shared by both sides of the trade, and published on the Integrated feed as the low
    /// four bytes of `TradeID`.
    pub deal_id: u64,
    pub last_px: Price,
    pub leaves_qty: u32,
    pub cum_qty: u32,
    pub last_qty: u32,
    /// Determines the fee or rebate for this fill.
    pub liquidity_indicator: Mpid4,
    /// Bits 0..3: executed trading session.
    pub execution_details: u8,
    pub locate_reqd: u8,
    pub participant_type: ParticipantType,
    pub reason_code: u16,
    pub user_data: UserData8,
}

impl ExecutionReport {
    pub const MIN_LEN: usize = 84;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        let header = MsgHeader::parse(bytes)?;
        if header.msg_type != msg_type::EXECUTION_REPORT {
            return Err(WireError::UnknownMessageType {
                protocol: PROTOCOL,
                got: header.msg_type,
                got_ascii: '?',
            });
        }
        if bytes.len() < Self::MIN_LEN {
            return Err(WireError::LengthMismatch {
                protocol: PROTOCOL,
                msg_type: header.msg_type,
                declared: bytes.len(),
                expected: Self::MIN_LEN,
            });
        }
        let mut c = Cursor::at(bytes, 4);
        let transact_time = c.le_u64()?;
        let symbol_id = c.le_u32()?;
        let mpid = Mpid4::from_wire(c.take(4)?);
        let order_id = c.le_u64()?;
        let cl_ord_id = c.le_u64()?;
        let deal_id = c.le_u64()?;
        let last_px = Price::from_pillar(c.le_u64()?);
        let leaves_qty = c.le_u32()?;
        let cum_qty = c.le_u32()?;
        let last_qty = c.le_u32()?;
        let liquidity_indicator = Mpid4::from_wire(c.take(4)?);
        let execution_details = c.u8()?;
        c.skip(3)?; // Reserved
        let locate_reqd = c.u8()?;
        let participant_type = ParticipantType::from_code(c.u8()?);
        let reason_code = c.le_u16()?;
        let user_data = UserData8::from_wire(c.take(8)?);

        Ok(Self {
            transact_time,
            symbol_id,
            mpid,
            order_id,
            cl_ord_id,
            deal_id,
            last_px,
            leaves_qty,
            cum_qty,
            last_qty,
            liquidity_indicator,
            execution_details,
            locate_reqd,
            participant_type,
            reason_code,
            user_data,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::MIN_LEN);
        MsgHeader {
            msg_type: msg_type::EXECUTION_REPORT,
            length: Self::MIN_LEN as u16,
        }
        .write(&mut w);
        w.le_u64(self.transact_time)
            .le_u32(self.symbol_id)
            .raw(self.mpid.as_bytes())
            .le_u64(self.order_id)
            .le_u64(self.cl_ord_id)
            .le_u64(self.deal_id)
            .le_u64(self.last_px.to_pillar())
            .le_u32(self.leaves_qty)
            .le_u32(self.cum_qty)
            .le_u32(self.last_qty)
            .raw(self.liquidity_indicator.as_bytes())
            .u8(self.execution_details)
            .nul_padded("", 3)
            .u8(self.locate_reqd)
            .u8(self.participant_type as u8)
            .le_u16(self.reason_code)
            .raw(self.user_data.as_bytes());
        w.into_vec()
    }

    /// True when this fill completes the order.
    pub const fn is_final_fill(&self) -> bool {
        self.leaves_qty == 0
    }
}

/// Any message the gateway can send to a firm that this crate decodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outbound {
    OrderAck(Box<OrderAck>),
    ExecutionReport(Box<ExecutionReport>),
}

impl Outbound {
    /// Decode an application payload by its `MsgHeader` type.
    pub fn decode(payload: &[u8]) -> WireResult<Option<Self>> {
        let header = MsgHeader::parse(payload)?;
        Ok(Some(match header.msg_type {
            msg_type::ORDER_ACK => Self::OrderAck(Box::new(OrderAck::parse(payload)?)),
            msg_type::EXECUTION_REPORT => {
                Self::ExecutionReport(Box::new(ExecutionReport::parse(payload)?))
            }
            _ => return Ok(None),
        }))
    }

    /// The firm-assigned order id this message concerns.
    pub fn cl_ord_id(&self) -> u64 {
        match self {
            Self::OrderAck(a) => a.cl_ord_id,
            Self::ExecutionReport(e) => e.cl_ord_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> NewOrder {
        NewOrder {
            symbol_id: 4242,
            mpid: Mpid4::new("ABCD"),
            mmid: 0,
            mp_sub_id: 'A',
            cl_ord_id: 1_000_001,
            orig_cl_ord_id: 0,
            instructions: OrderInstructions::default(),
            price: Price::from_f64(180.50),
            order_qty: 500,
            min_qty: 0,
            user_data: UserData8::new("DESK1"),
        }
    }

    #[test]
    fn seq_msg_envelope_round_trips_with_its_payload() {
        let payload = order().encode();
        let msg = SeqMsg::new(
            SeqMsgId {
                stream_id: 0x1122_3344_5566_7788,
                sequence: 42,
            },
            1_700_000_000_123_456_789,
            payload.clone(),
        );
        let bytes = msg.encode();
        assert_eq!(bytes.len(), SeqMsg::PAYLOAD_OFFSET + payload.len());
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), msg_type::SEQ_MSG);
        let back = SeqMsg::parse(&bytes).unwrap();
        assert_eq!(back, msg);
        assert_eq!(back.payload_type().unwrap(), msg_type::NEW_ORDER);
    }

    #[test]
    fn seq_msg_payload_starts_at_offset_thirty_two() {
        let msg = SeqMsg::new(
            SeqMsgId {
                stream_id: 1,
                sequence: 1,
            },
            0,
            vec![0xAA; 10],
        );
        let bytes = msg.encode();
        assert_eq!(&bytes[32..42], &[0xAA; 10]);
        assert_eq!(u64::from_le_bytes(bytes[4..12].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(bytes[12..20].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(bytes[24..32].try_into().unwrap()), 0);
    }

    #[test]
    fn new_order_is_sixty_five_bytes_at_the_documented_offsets() {
        let o = order();
        let b = o.encode();
        assert_eq!(b.len(), NewOrder::MIN_LEN);
        assert_eq!(u16::from_le_bytes([b[0], b[1]]), msg_type::NEW_ORDER);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 4242);
        assert_eq!(&b[8..12], b"ABCD");
        assert_eq!(u32::from_le_bytes(b[12..16].try_into().unwrap()), 0);
        assert_eq!(b[16], b'A');
        assert_eq!(u64::from_le_bytes(b[17..25].try_into().unwrap()), 1_000_001);
        assert_eq!(u64::from_le_bytes(b[25..33].try_into().unwrap()), 0);
        // Price at offset 41, scale 8: $180.50 → 18_050_000_000.
        assert_eq!(
            u64::from_le_bytes(b[41..49].try_into().unwrap()),
            18_050_000_000
        );
        assert_eq!(u32::from_le_bytes(b[49..53].try_into().unwrap()), 500);
        assert_eq!(u32::from_le_bytes(b[53..57].try_into().unwrap()), 0);
        assert_eq!(&b[57..62], b"DESK1");
    }

    #[test]
    fn new_order_round_trips() {
        let o = order();
        assert_eq!(NewOrder::parse(&o.encode()).unwrap(), o);
    }

    #[test]
    fn a_non_zero_orig_cl_ord_id_makes_it_a_cancel_replace() {
        let mut o = order();
        assert!(!o.is_replace());
        o.orig_cl_ord_id = 999;
        assert!(o.is_replace());
        assert_eq!(NewOrder::parse(&o.encode()).unwrap().orig_cl_ord_id, 999);
    }

    #[test]
    fn order_instructions_pack_at_the_documented_bit_offsets() {
        let i = OrderInstructions {
            side: Side::SellShort,
            ord_type: OrdType::Limit,
            time_in_force: TimeInForce::Ioc,
            capacity: OrderCapacity::Principal,
            trading_session: TradingSession::Core,
            ..Default::default()
        };
        let b = i.to_bits();
        assert_eq!((b >> 60) & 0xF, 3, "Side at bit 60");
        assert_eq!((b >> 56) & 0xF, 2, "OrdType at bit 56");
        assert_eq!((b >> 31) & 0x7, 2, "TimeInForce at bit 31");
        assert_eq!((b >> 22) & 0x7, 2, "OrderCapacity at bit 22");
        assert_eq!((b >> 28) & 0x7, 2, "TradingSessionID at bit 28");
        assert_eq!(OrderInstructions::from_bits(b).unwrap(), i);
    }

    #[test]
    fn every_instruction_field_survives_a_round_trip() {
        let i = OrderInstructions {
            side: Side::CrossShortExempt,
            ord_type: OrdType::Pegged,
            time_in_force: TimeInForce::OnClose,
            capacity: OrderCapacity::RisklessPrincipal,
            trading_session: TradingSession::EarlyCoreAndLate,
            exec_inst: ExecInst::MidpointLiquidity,
            extended_exec_inst: ExtendedExecInst::Alo,
            routing_inst: RoutingInst::RouteToAts,
            self_trade_type: SelfTradeType::CancelOldest,
            locate_reqd: false,
            retail_indicator: true,
            attributed_quote: 3,
            sub_id_indicator: true,
            special_ord_type: 4,
            interest_type: 5,
            proactive_if_locked: 2,
            cancel_instead_of_reprice: 3,
        };
        assert_eq!(OrderInstructions::from_bits(i.to_bits()).unwrap(), i);
    }

    #[test]
    fn the_reserved_low_twelve_bits_stay_zero() {
        let bits = OrderInstructions::default().to_bits();
        assert_eq!(bits & 0xFFF, 0, "bits 0..12 are reserved and must be 0");
    }

    #[test]
    fn an_undefined_side_is_an_error_not_a_default() {
        let mut bits = OrderInstructions::default().to_bits();
        put(&mut bits, bits::SIDE, 9);
        assert!(matches!(
            OrderInstructions::from_bits(bits),
            Err(WireError::InvalidEnum { field: "Side", .. })
        ));
    }

    #[test]
    fn short_sides_must_not_set_locate_reqd() {
        let mut i = OrderInstructions {
            side: Side::SellShort,
            locate_reqd: true,
            ..Default::default()
        };
        assert!(i.validate().is_err());
        i.locate_reqd = false;
        assert!(i.validate().is_ok());
        // A long sale is unaffected either way.
        assert!(OrderInstructions {
            side: Side::Sell,
            locate_reqd: false,
            ..Default::default()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn order_validation_catches_what_pillar_would_reject() {
        let mut o = order();
        o.order_qty = 0;
        assert!(o.validate().is_err(), "zero quantity");

        o = order();
        o.order_qty = 1_000_000_000;
        assert!(o.validate().is_err(), "quantity above 999,999,999");

        o = order();
        o.min_qty = 1_000;
        assert!(o.validate().is_err(), "MinQty above OrderQty");

        o = order();
        o.instructions.routing_inst = RoutingInst::MinimumFill;
        assert!(o.validate().is_err(), "minimum fill without MinQty");
        o.min_qty = 100;
        assert!(o.validate().is_ok());

        o = order();
        o.user_data = UserData8::new("A@B");
        assert!(o.validate().is_err(), "disallowed character in UserData");

        o = order();
        o.cl_ord_id = 0;
        assert!(o.validate().is_err(), "ClOrdID of zero");

        o = order();
        o.price = Price::ZERO;
        assert!(o.validate().is_err(), "limit order at zero");

        assert!(order().validate().is_ok());
    }

    #[test]
    fn market_orders_may_have_a_zero_price() {
        let mut o = order();
        o.instructions.ord_type = OrdType::Market;
        o.price = Price::ZERO;
        assert!(o.validate().is_ok());
    }

    #[test]
    fn cancel_request_is_twenty_eight_bytes() {
        let c = CancelRequest {
            symbol_id: 4242,
            mpid: Mpid4::new("ABCD"),
            cl_ord_id: 2,
            orig_cl_ord_id: 1,
        };
        let b = c.encode();
        assert_eq!(b.len(), CancelRequest::LEN);
        assert_eq!(
            u16::from_le_bytes([b[0], b[1]]),
            msg_type::ORDER_CANCEL_REQUEST
        );
        assert_eq!(u16::from_le_bytes([b[2], b[3]]), 28);
        assert_eq!(CancelRequest::parse(&b).unwrap(), c);
    }

    #[test]
    fn order_ack_round_trips_and_links_to_the_public_order_id() {
        let a = OrderAck {
            transact_time: 1_700_000_000_000_000_001,
            symbol_id: 4242,
            mpid: Mpid4::new("ABCD"),
            mmid: 0,
            mp_sub_id: 'A',
            cl_ord_id: 1_000_001,
            orig_cl_ord_id: 0,
            instructions: OrderInstructions::default(),
            price: Price::from_f64(180.50),
            order_qty: 500,
            min_qty: 0,
            order_id: 0x0102_0304_0506_0708,
            leaves_qty: 500,
            working_price: Price::from_f64(180.50),
            working_away_from_display: 0,
            pre_liquidity_indicator: Mpid4::new("0"),
            reason_code: 0,
            ack_type: AckType::NewInterest,
            flow_indicator: 0,
        };
        let bytes = a.encode();
        assert_eq!(bytes.len(), OrderAck::MIN_LEN);
        let back = OrderAck::parse(&bytes).unwrap();
        assert_eq!(back, a);
        assert_eq!(back.order_id, 0x0102_0304_0506_0708);
        assert!(!back.was_throttled());
    }

    #[test]
    fn a_throttled_flow_indicator_is_visible() {
        let mut a = OrderAck {
            transact_time: 1,
            symbol_id: 1,
            mpid: Mpid4::new("A"),
            mmid: 0,
            mp_sub_id: ' ',
            cl_ord_id: 1,
            orig_cl_ord_id: 0,
            instructions: OrderInstructions::default(),
            price: Price::from_f64(1.0),
            order_qty: 1,
            min_qty: 0,
            order_id: 1,
            leaves_qty: 1,
            working_price: Price::from_f64(1.0),
            working_away_from_display: 0,
            pre_liquidity_indicator: Mpid4::NUL,
            reason_code: 0,
            ack_type: AckType::NewInterest,
            flow_indicator: 1,
        };
        assert!(a.was_throttled());
        a.flow_indicator = 0;
        assert!(!a.was_throttled());
    }

    #[test]
    fn ack_types_identify_terminal_states() {
        assert!(AckType::Canceled.is_terminal());
        assert!(AckType::DoneForDay.is_terminal());
        assert!(!AckType::NewInterest.is_terminal());
        assert!(!AckType::PendingReplace.is_terminal());
        assert_eq!(AckType::from_code(8), AckType::Replaced);
        assert_eq!(AckType::from_code(200), AckType::Unknown(200));
    }

    #[test]
    fn execution_report_round_trips_and_reports_completion() {
        let e = ExecutionReport {
            transact_time: 1_700_000_000_000_000_002,
            symbol_id: 4242,
            mpid: Mpid4::new("ABCD"),
            order_id: 0x0102_0304_0506_0708,
            cl_ord_id: 1_000_001,
            deal_id: 0x00AA_BBCC_DDEE_FF00,
            last_px: Price::from_f64(180.50),
            leaves_qty: 0,
            cum_qty: 500,
            last_qty: 500,
            liquidity_indicator: Mpid4::new("R"),
            execution_details: 0,
            locate_reqd: 0,
            participant_type: ParticipantType::Customer,
            reason_code: 0,
            user_data: UserData8::new("DESK1"),
        };
        let bytes = e.encode();
        assert_eq!(bytes.len(), ExecutionReport::MIN_LEN);
        let back = ExecutionReport::parse(&bytes).unwrap();
        assert_eq!(back, e);
        assert!(back.is_final_fill());
    }

    #[test]
    fn a_partial_fill_is_not_final() {
        let mut e = ExecutionReport {
            transact_time: 1,
            symbol_id: 1,
            mpid: Mpid4::new("A"),
            order_id: 1,
            cl_ord_id: 1,
            deal_id: 1,
            last_px: Price::from_f64(1.0),
            leaves_qty: 100,
            cum_qty: 400,
            last_qty: 400,
            liquidity_indicator: Mpid4::new("A"),
            execution_details: 0,
            locate_reqd: 0,
            participant_type: ParticipantType::Customer,
            reason_code: 0,
            user_data: UserData8::NUL,
        };
        assert!(!e.is_final_fill());
        e.leaves_qty = 0;
        assert!(e.is_final_fill());
    }

    #[test]
    fn outbound_dispatches_on_the_payload_message_type() {
        let ack = OrderAck {
            transact_time: 1,
            symbol_id: 1,
            mpid: Mpid4::new("A"),
            mmid: 0,
            mp_sub_id: ' ',
            cl_ord_id: 77,
            orig_cl_ord_id: 0,
            instructions: OrderInstructions::default(),
            price: Price::from_f64(1.0),
            order_qty: 1,
            min_qty: 0,
            order_id: 1,
            leaves_qty: 1,
            working_price: Price::from_f64(1.0),
            working_away_from_display: 0,
            pre_liquidity_indicator: Mpid4::NUL,
            reason_code: 0,
            ack_type: AckType::NewInterest,
            flow_indicator: 0,
        };
        let decoded = Outbound::decode(&ack.encode()).unwrap().unwrap();
        assert_eq!(decoded.cl_ord_id(), 77);
        assert!(matches!(decoded, Outbound::OrderAck(_)));

        // An unrecognised application type is reported as such rather than misparsed.
        let mut junk = ack.encode();
        junk[0..2].copy_from_slice(&0x0999u16.to_le_bytes());
        assert!(Outbound::decode(&junk).unwrap().is_none());
    }

    #[test]
    fn pillar_price_scale_is_eight_not_four() {
        // The single most likely integration bug: using the ITCH scale here would enter an
        // order at 1/10,000th of the intended price.
        let o = NewOrder {
            price: Price::from_f64(1.23),
            ..order()
        };
        let b = o.encode();
        assert_eq!(
            u64::from_le_bytes(b[41..49].try_into().unwrap()),
            123_000_000
        );
    }
}
