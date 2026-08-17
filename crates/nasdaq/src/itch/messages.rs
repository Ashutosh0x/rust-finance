//! Nasdaq TotalView-ITCH 5.0 message decoder.
//!
//! Implements every message type in the published specification (Nasdaq TotalView-ITCH 5.0,
//! revision of April 28 2023, which added the Direct Listing with Capital Raise message and
//! the `P` = Paused imbalance direction).
//!
//! Wire conventions, from the spec's Data Types section:
//!
//! * all integer fields are **big endian**, unsigned unless noted;
//! * alpha fields are ASCII, left justified, right padded with spaces;
//! * `Price(n)` is an integer with `n` implied decimal places;
//! * timestamps are **6 bytes, nanoseconds since midnight** (Eastern), so they must be
//!   rebased onto a session date before they mean anything in UTC;
//! * every message carries `Stock Locate` at offset 1, "at the same position in all
//!   messages to support efficient filtering" — [`peek_stock_locate`] exploits that to
//!   filter before decoding.
//!
//! Decoding is zero-copy: `&str` fields borrow from the caller's receive buffer.

use exchange_core::wire::Cursor;
use exchange_core::{Price, WireError, WireResult};

const PROTOCOL: &str = "ITCH 5.0";

/// Fields common to the front of every ITCH message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Day-unique instrument handle, assigned in the Stock Directory spin. 0 for messages
    /// that are not instrument specific.
    pub stock_locate: u16,
    /// Nasdaq internal tracking number. Opaque; carried through for support tickets.
    pub tracking_number: u16,
    /// Nanoseconds since midnight US/Eastern.
    pub timestamp: u64,
}

impl Header {
    fn read(c: &mut Cursor<'_>) -> WireResult<Self> {
        Ok(Self {
            stock_locate: c.be_u16()?,
            tracking_number: c.be_u16()?,
            timestamp: c.be_u48()?,
        })
    }
}

// ─── System event ───────────────────────────────────────────────────────────

/// `S` — market/feed-handler lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEventCode {
    /// `O` — start of messages; first message of the trading day.
    StartOfMessages,
    /// `S` — start of system hours; Nasdaq is accepting orders.
    StartOfSystemHours,
    /// `Q` — start of market hours; market-hours orders are executable.
    StartOfMarketHours,
    /// `M` — end of market hours.
    EndOfMarketHours,
    /// `E` — end of system hours; no new orders. Breaks and deletes may still follow.
    EndOfSystemHours,
    /// `C` — end of messages; last message of the day.
    EndOfMessages,
}

impl SystemEventCode {
    fn parse(ch: char) -> WireResult<Self> {
        Ok(match ch {
            'O' => Self::StartOfMessages,
            'S' => Self::StartOfSystemHours,
            'Q' => Self::StartOfMarketHours,
            'M' => Self::EndOfMarketHours,
            'E' => Self::EndOfSystemHours,
            'C' => Self::EndOfMessages,
            other => {
                return Err(WireError::InvalidEnum {
                    protocol: PROTOCOL,
                    field: "Event Code",
                    value: other,
                })
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemEvent {
    pub header: Header,
    pub event: SystemEventCode,
}

// ─── Stock related ──────────────────────────────────────────────────────────

/// `R` — one per active symbol at start of day. Establishes the locate-code mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockDirectory<'a> {
    pub header: Header,
    pub stock: &'a str,
    /// Listing market/tier: `Q`/`G`/`S` Nasdaq tiers, `N`/`A`/`P` NYSE family, `Z` Cboe BZX,
    /// `V` IEX, space when unavailable.
    pub market_category: char,
    /// Nasdaq continued-listing compliance flag (`N` normal, `D` deficient, …).
    pub financial_status: char,
    pub round_lot_size: u32,
    /// `Y` when Nasdaq only accepts round lots in this issue.
    pub round_lots_only: bool,
    pub issue_classification: char,
    pub issue_subtype: &'a str,
    /// `P` live/production, `T` test, `D` demo.
    pub authenticity: char,
    /// SEC Rule 203(b)(3) threshold list: `Y`, `N`, or space.
    pub short_sale_threshold: char,
    pub ipo_flag: char,
    /// LULD price-band tier: `1`, `2` or space.
    pub luld_reference_price_tier: char,
    pub etp_flag: char,
    pub etp_leverage_factor: u32,
    pub inverse_etp: bool,
}

/// `H` — halt / pause / resume for one symbol, across all US equity markets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockTradingAction<'a> {
    pub header: Header,
    pub stock: &'a str,
    pub trading_state: TradingStateCode,
    pub reason: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingStateCode {
    /// `H` — halted across all US equity markets / SROs.
    Halted,
    /// `P` — paused across all US equity markets (Nasdaq-listed only).
    Paused,
    /// `Q` — quotation-only period for a cross-SRO halt or pause.
    QuotationOnly,
    /// `T` — trading on Nasdaq.
    Trading,
}

impl TradingStateCode {
    fn parse(ch: char) -> WireResult<Self> {
        Ok(match ch {
            'H' => Self::Halted,
            'P' => Self::Paused,
            'Q' => Self::QuotationOnly,
            'T' => Self::Trading,
            other => {
                return Err(WireError::InvalidEnum {
                    protocol: PROTOCOL,
                    field: "Trading State",
                    value: other,
                })
            }
        })
    }
}

/// `Y` — Reg SHO Rule 201 short-sale price-test state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegShoRestriction<'a> {
    pub header: Header,
    pub stock: &'a str,
    pub action: RegShoAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegShoAction {
    /// `0` — no price test in place.
    None,
    /// `1` — restriction in effect from an intraday 10% price drop.
    IntradayDrop,
    /// `2` — restriction remains in effect (day two).
    Continued,
}

impl RegShoAction {
    fn parse(ch: char) -> WireResult<Self> {
        Ok(match ch {
            '0' => Self::None,
            '1' => Self::IntradayDrop,
            '2' => Self::Continued,
            other => {
                return Err(WireError::InvalidEnum {
                    protocol: PROTOCOL,
                    field: "Reg SHO Action",
                    value: other,
                })
            }
        })
    }
}

/// `L` — a market participant's registration state in one issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketParticipantPosition<'a> {
    pub header: Header,
    pub mpid: &'a str,
    pub stock: &'a str,
    /// `Y` when the firm qualifies as a Primary Market Maker.
    pub primary_market_maker: bool,
    /// Regulation M status: `N` normal, `P` passive, `S` syndicate, `R` pre-syndicate,
    /// `L` penalty.
    pub market_maker_mode: char,
    /// `A` active, `E` excused/withdrawn, `W` withdrawn, `S` suspended, `D` deleted.
    pub market_participant_state: char,
}

/// `V` — the day's market-wide circuit-breaker trigger levels (`Price(8)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MwcbDeclineLevel {
    pub header: Header,
    pub level1: Price,
    pub level2: Price,
    pub level3: Price,
}

/// `W` — a market-wide circuit-breaker level was breached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MwcbStatus {
    pub header: Header,
    /// `1`, `2` or `3`.
    pub breached_level: char,
}

/// `K` — anticipated IPO quotation release time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpoQuotingPeriodUpdate<'a> {
    pub header: Header,
    pub stock: &'a str,
    /// Seconds since midnight. Zero when the release is cancelled or postponed.
    pub release_time_secs: u32,
    /// `A` anticipated release time, `C` cancelled/postponed.
    pub release_qualifier: char,
    pub ipo_price: Price,
}

/// `J` — reopening auction collars after a LULD trading pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuldAuctionCollar<'a> {
    pub header: Header,
    pub stock: &'a str,
    pub reference_price: Price,
    pub upper_collar: Price,
    pub lower_collar: Price,
    /// Number of extensions granted to the reopening auction.
    pub collar_extension: u32,
}

/// `h` — venue-specific operational halt. Unlike `H`, the instrument keeps trading
/// elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationalHalt<'a> {
    pub header: Header,
    pub stock: &'a str,
    /// `Q` Nasdaq, `B` BX, `X` PSX.
    pub market_code: char,
    /// `H` halted on this market, `T` resumed.
    pub halted: bool,
}

// ─── Order book ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuySell {
    Buy,
    Sell,
}

impl BuySell {
    fn parse(ch: char) -> WireResult<Self> {
        Ok(match ch {
            'B' => Self::Buy,
            'S' => Self::Sell,
            other => {
                return Err(WireError::InvalidEnum {
                    protocol: PROTOCOL,
                    field: "Buy/Sell Indicator",
                    value: other,
                })
            }
        })
    }
}

/// `A` / `F` — a new displayed order joined the book. `attribution` is `Some` only for the
/// `F` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddOrder<'a> {
    pub header: Header,
    pub order_reference_number: u64,
    pub side: BuySell,
    pub shares: u32,
    pub stock: &'a str,
    pub price: Price,
    pub attribution: Option<&'a str>,
}

/// `E` — a resting order executed at its display price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderExecuted {
    pub header: Header,
    pub order_reference_number: u64,
    pub executed_shares: u32,
    pub match_number: u64,
}

/// `C` — a resting order executed away from its display price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderExecutedWithPrice {
    pub header: Header,
    pub order_reference_number: u64,
    pub executed_shares: u32,
    pub match_number: u64,
    /// `N` executions are rolled into a later bulk print; counting them would double count
    /// cross volume.
    pub printable: bool,
    pub execution_price: Price,
}

/// `X` — partial cancel. Displayed size shrinks; time priority is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderCancel {
    pub header: Header,
    pub order_reference_number: u64,
    pub cancelled_shares: u32,
}

/// `D` — the order left the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderDelete {
    pub header: Header,
    pub order_reference_number: u64,
}

/// `U` — cancel/replace. Side, symbol and MPID are *not* restated; carry them from the
/// original Add Order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderReplace {
    pub header: Header,
    pub original_order_reference_number: u64,
    pub new_order_reference_number: u64,
    pub shares: u32,
    pub price: Price,
}

// ─── Trades ─────────────────────────────────────────────────────────────────

/// `P` — a match involving non-displayable order types. No book impact.
///
/// Two historical quirks are preserved verbatim from the spec: `order_reference_number` has
/// been published as zero since 2010-12-06, and `side` has been forced to `B` regardless of
/// the resting side since 2014-07-14. Neither field should be relied on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeNonCross<'a> {
    pub header: Header,
    pub order_reference_number: u64,
    pub side: BuySell,
    pub shares: u32,
    pub stock: &'a str,
    pub price: Price,
    pub match_number: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossType {
    /// `O` — Nasdaq Opening Cross.
    Opening,
    /// `C` — Nasdaq Closing Cross.
    Closing,
    /// `H` — IPO / halt / pause cross.
    HaltIpo,
    /// `A` — Extended Trading Close (NOII only).
    ExtendedTradingClose,
}

impl CrossType {
    fn parse(ch: char) -> WireResult<Self> {
        Ok(match ch {
            'O' => Self::Opening,
            'C' => Self::Closing,
            'H' => Self::HaltIpo,
            'A' => Self::ExtendedTradingClose,
            other => {
                return Err(WireError::InvalidEnum {
                    protocol: PROTOCOL,
                    field: "Cross Type",
                    value: other,
                })
            }
        })
    }
}

/// `Q` — bulk print for one symbol's cross. Shares may be zero when there was too little
/// interest to cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossTrade<'a> {
    pub header: Header,
    pub shares: u64,
    pub stock: &'a str,
    pub cross_price: Price,
    pub match_number: u64,
    pub cross_type: CrossType,
}

/// `B` — an execution was broken. Final; a broken trade is never reinstated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokenTrade {
    pub header: Header,
    pub match_number: u64,
}

// ─── Auction / retail ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImbalanceDirection {
    Buy,
    Sell,
    None,
    /// `O` — insufficient orders to calculate.
    InsufficientOrders,
    /// `P` — paused (added in the 2023 revision).
    Paused,
}

impl ImbalanceDirection {
    fn parse(ch: char) -> WireResult<Self> {
        Ok(match ch {
            'B' => Self::Buy,
            'S' => Self::Sell,
            'N' => Self::None,
            'O' => Self::InsufficientOrders,
            'P' => Self::Paused,
            other => {
                return Err(WireError::InvalidEnum {
                    protocol: PROTOCOL,
                    field: "Imbalance Direction",
                    value: other,
                })
            }
        })
    }
}

/// `I` — Net Order Imbalance Indicator, published into the opening/closing crosses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetOrderImbalance<'a> {
    pub header: Header,
    pub paired_shares: u64,
    pub imbalance_shares: u64,
    pub imbalance_direction: ImbalanceDirection,
    pub stock: &'a str,
    /// Hypothetical clearing price for cross orders only.
    pub far_price: Price,
    /// Hypothetical clearing price including continuous orders.
    pub near_price: Price,
    pub current_reference_price: Price,
    pub cross_type: CrossType,
    /// Deviation band of near price from the reference price (`L`, `1`..`9`, `A`..`C`).
    pub price_variation_indicator: char,
}

/// `N` — Retail Price Improvement indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetailPriceImprovement<'a> {
    pub header: Header,
    pub stock: &'a str,
    /// `B` bid, `S` offer, `A` both, `N` none.
    pub interest_flag: char,
}

/// `O` — Direct Listing with Capital Raise price discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectListingCapitalRaise<'a> {
    pub header: Header,
    pub stock: &'a str,
    pub open_eligible: bool,
    /// 20% below the registration statement's lower price.
    pub minimum_allowable_price: Price,
    /// 80% above the registration statement's highest price.
    pub maximum_allowable_price: Price,
    pub near_execution_price: Price,
    /// Nanoseconds since midnight at which the near execution price was set.
    pub near_execution_time: u64,
    pub lower_price_collar: Price,
    pub upper_price_collar: Price,
}

// ─── The union ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItchMessage<'a> {
    SystemEvent(SystemEvent),
    StockDirectory(StockDirectory<'a>),
    StockTradingAction(StockTradingAction<'a>),
    RegShoRestriction(RegShoRestriction<'a>),
    MarketParticipantPosition(MarketParticipantPosition<'a>),
    MwcbDeclineLevel(MwcbDeclineLevel),
    MwcbStatus(MwcbStatus),
    IpoQuotingPeriodUpdate(IpoQuotingPeriodUpdate<'a>),
    LuldAuctionCollar(LuldAuctionCollar<'a>),
    OperationalHalt(OperationalHalt<'a>),
    AddOrder(AddOrder<'a>),
    OrderExecuted(OrderExecuted),
    OrderExecutedWithPrice(OrderExecutedWithPrice),
    OrderCancel(OrderCancel),
    OrderDelete(OrderDelete),
    OrderReplace(OrderReplace),
    TradeNonCross(TradeNonCross<'a>),
    CrossTrade(CrossTrade<'a>),
    BrokenTrade(BrokenTrade),
    NetOrderImbalance(NetOrderImbalance<'a>),
    RetailPriceImprovement(RetailPriceImprovement<'a>),
    DirectListingCapitalRaise(DirectListingCapitalRaise<'a>),
}

impl ItchMessage<'_> {
    /// The one-byte type code this message decoded from.
    pub const fn message_type(&self) -> u8 {
        match self {
            Self::SystemEvent(_) => b'S',
            Self::StockDirectory(_) => b'R',
            Self::StockTradingAction(_) => b'H',
            Self::RegShoRestriction(_) => b'Y',
            Self::MarketParticipantPosition(_) => b'L',
            Self::MwcbDeclineLevel(_) => b'V',
            Self::MwcbStatus(_) => b'W',
            Self::IpoQuotingPeriodUpdate(_) => b'K',
            Self::LuldAuctionCollar(_) => b'J',
            Self::OperationalHalt(_) => b'h',
            Self::AddOrder(a) => {
                if a.attribution.is_some() {
                    b'F'
                } else {
                    b'A'
                }
            }
            Self::OrderExecuted(_) => b'E',
            Self::OrderExecutedWithPrice(_) => b'C',
            Self::OrderCancel(_) => b'X',
            Self::OrderDelete(_) => b'D',
            Self::OrderReplace(_) => b'U',
            Self::TradeNonCross(_) => b'P',
            Self::CrossTrade(_) => b'Q',
            Self::BrokenTrade(_) => b'B',
            Self::NetOrderImbalance(_) => b'I',
            Self::RetailPriceImprovement(_) => b'N',
            Self::DirectListingCapitalRaise(_) => b'O',
        }
    }

    pub const fn header(&self) -> Header {
        match self {
            Self::SystemEvent(m) => m.header,
            Self::StockDirectory(m) => m.header,
            Self::StockTradingAction(m) => m.header,
            Self::RegShoRestriction(m) => m.header,
            Self::MarketParticipantPosition(m) => m.header,
            Self::MwcbDeclineLevel(m) => m.header,
            Self::MwcbStatus(m) => m.header,
            Self::IpoQuotingPeriodUpdate(m) => m.header,
            Self::LuldAuctionCollar(m) => m.header,
            Self::OperationalHalt(m) => m.header,
            Self::AddOrder(m) => m.header,
            Self::OrderExecuted(m) => m.header,
            Self::OrderExecutedWithPrice(m) => m.header,
            Self::OrderCancel(m) => m.header,
            Self::OrderDelete(m) => m.header,
            Self::OrderReplace(m) => m.header,
            Self::TradeNonCross(m) => m.header,
            Self::CrossTrade(m) => m.header,
            Self::BrokenTrade(m) => m.header,
            Self::NetOrderImbalance(m) => m.header,
            Self::RetailPriceImprovement(m) => m.header,
            Self::DirectListingCapitalRaise(m) => m.header,
        }
    }

    /// Ticker, where the message carries one inline.
    pub const fn stock(&self) -> Option<&str> {
        match self {
            Self::StockDirectory(m) => Some(m.stock),
            Self::StockTradingAction(m) => Some(m.stock),
            Self::RegShoRestriction(m) => Some(m.stock),
            Self::MarketParticipantPosition(m) => Some(m.stock),
            Self::IpoQuotingPeriodUpdate(m) => Some(m.stock),
            Self::LuldAuctionCollar(m) => Some(m.stock),
            Self::OperationalHalt(m) => Some(m.stock),
            Self::AddOrder(m) => Some(m.stock),
            Self::TradeNonCross(m) => Some(m.stock),
            Self::CrossTrade(m) => Some(m.stock),
            Self::NetOrderImbalance(m) => Some(m.stock),
            Self::RetailPriceImprovement(m) => Some(m.stock),
            Self::DirectListingCapitalRaise(m) => Some(m.stock),
            _ => None,
        }
    }
}

/// Fixed on-the-wire length of each message type, including the one-byte type code.
///
/// ITCH is self-describing only by type, not by length: the surrounding session layer
/// (SoupBinTCP or MoldUDP64) delimits messages, so these lengths are used to validate what
/// the session layer handed over rather than to walk a stream.
pub const fn message_length(msg_type: u8) -> Option<usize> {
    Some(match msg_type {
        b'S' => 12,
        b'R' => 39,
        b'H' => 25,
        b'Y' => 20,
        b'L' => 26,
        b'V' => 35,
        b'W' => 12,
        b'K' => 28,
        b'J' => 35,
        b'h' => 21,
        b'A' => 36,
        b'F' => 40,
        b'E' => 31,
        b'C' => 36,
        b'X' => 23,
        b'D' => 19,
        b'U' => 35,
        b'P' => 44,
        b'Q' => 40,
        b'B' => 19,
        b'I' => 50,
        b'N' => 20,
        b'O' => 48,
        _ => return None,
    })
}

/// Read the stock locate code without decoding the message.
///
/// The spec guarantees it lives at offset 1 in every message type precisely so that a feed
/// handler can discard uninteresting instruments before paying for a full decode.
#[inline]
pub fn peek_stock_locate(buf: &[u8]) -> Option<u16> {
    buf.get(1..3).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

/// Decode one ITCH message from the front of `buf`.
///
/// `buf` must contain exactly one message as delimited by the session layer; a length that
/// disagrees with the specification is rejected rather than tolerated, because a wrong
/// length means the session framing itself is broken.
pub fn decode(buf: &[u8]) -> WireResult<ItchMessage<'_>> {
    let msg_type = *buf.first().ok_or(WireError::Truncated {
        at: 0,
        need: 1,
        have: 0,
    })?;

    let expected = message_length(msg_type).ok_or(WireError::UnknownMessageType {
        protocol: PROTOCOL,
        got: msg_type as u16,
        got_ascii: msg_type as char,
    })?;

    if buf.len() != expected {
        return Err(WireError::LengthMismatch {
            protocol: PROTOCOL,
            msg_type: msg_type as u16,
            declared: buf.len(),
            expected,
        });
    }

    let mut c = Cursor::at(buf, 1);
    let header = Header::read(&mut c)?;

    Ok(match msg_type {
        b'S' => ItchMessage::SystemEvent(SystemEvent {
            header,
            event: SystemEventCode::parse(c.ascii_char()?)?,
        }),

        b'R' => ItchMessage::StockDirectory(StockDirectory {
            header,
            stock: c.space_padded(8, "Stock")?,
            market_category: c.ascii_char()?,
            financial_status: c.ascii_char()?,
            round_lot_size: c.be_u32()?,
            round_lots_only: c.ascii_char()? == 'Y',
            issue_classification: c.ascii_char()?,
            issue_subtype: c.space_padded(2, "Issue Sub-Type")?,
            authenticity: c.ascii_char()?,
            short_sale_threshold: c.ascii_char()?,
            ipo_flag: c.ascii_char()?,
            luld_reference_price_tier: c.ascii_char()?,
            etp_flag: c.ascii_char()?,
            etp_leverage_factor: c.be_u32()?,
            inverse_etp: c.ascii_char()? == 'Y',
        }),

        b'H' => {
            let stock = c.space_padded(8, "Stock")?;
            let trading_state = TradingStateCode::parse(c.ascii_char()?)?;
            c.skip(1)?; // Reserved
            ItchMessage::StockTradingAction(StockTradingAction {
                header,
                stock,
                trading_state,
                reason: c.space_padded(4, "Reason")?,
            })
        }

        b'Y' => ItchMessage::RegShoRestriction(RegShoRestriction {
            header,
            stock: c.space_padded(8, "Stock")?,
            action: RegShoAction::parse(c.ascii_char()?)?,
        }),

        b'L' => ItchMessage::MarketParticipantPosition(MarketParticipantPosition {
            header,
            mpid: c.space_padded(4, "MPID")?,
            stock: c.space_padded(8, "Stock")?,
            primary_market_maker: c.ascii_char()? == 'Y',
            market_maker_mode: c.ascii_char()?,
            market_participant_state: c.ascii_char()?,
        }),

        b'V' => ItchMessage::MwcbDeclineLevel(MwcbDeclineLevel {
            header,
            level1: Price::from_price8(c.be_u64()?),
            level2: Price::from_price8(c.be_u64()?),
            level3: Price::from_price8(c.be_u64()?),
        }),

        b'W' => ItchMessage::MwcbStatus(MwcbStatus {
            header,
            breached_level: c.ascii_char()?,
        }),

        b'K' => ItchMessage::IpoQuotingPeriodUpdate(IpoQuotingPeriodUpdate {
            header,
            stock: c.space_padded(8, "Stock")?,
            release_time_secs: c.be_u32()?,
            release_qualifier: c.ascii_char()?,
            ipo_price: Price::from_price4(c.be_u32()?),
        }),

        b'J' => ItchMessage::LuldAuctionCollar(LuldAuctionCollar {
            header,
            stock: c.space_padded(8, "Stock")?,
            reference_price: Price::from_price4(c.be_u32()?),
            upper_collar: Price::from_price4(c.be_u32()?),
            lower_collar: Price::from_price4(c.be_u32()?),
            collar_extension: c.be_u32()?,
        }),

        b'h' => ItchMessage::OperationalHalt(OperationalHalt {
            header,
            stock: c.space_padded(8, "Stock")?,
            market_code: c.ascii_char()?,
            halted: c.ascii_char()? == 'H',
        }),

        b'A' | b'F' => {
            let order_reference_number = c.be_u64()?;
            let side = BuySell::parse(c.ascii_char()?)?;
            let shares = c.be_u32()?;
            let stock = c.space_padded(8, "Stock")?;
            let price = Price::from_price4(c.be_u32()?);
            let attribution = if msg_type == b'F' {
                Some(c.space_padded(4, "Attribution")?)
            } else {
                None
            };
            ItchMessage::AddOrder(AddOrder {
                header,
                order_reference_number,
                side,
                shares,
                stock,
                price,
                attribution,
            })
        }

        b'E' => ItchMessage::OrderExecuted(OrderExecuted {
            header,
            order_reference_number: c.be_u64()?,
            executed_shares: c.be_u32()?,
            match_number: c.be_u64()?,
        }),

        b'C' => ItchMessage::OrderExecutedWithPrice(OrderExecutedWithPrice {
            header,
            order_reference_number: c.be_u64()?,
            executed_shares: c.be_u32()?,
            match_number: c.be_u64()?,
            printable: c.ascii_char()? == 'Y',
            execution_price: Price::from_price4(c.be_u32()?),
        }),

        b'X' => ItchMessage::OrderCancel(OrderCancel {
            header,
            order_reference_number: c.be_u64()?,
            cancelled_shares: c.be_u32()?,
        }),

        b'D' => ItchMessage::OrderDelete(OrderDelete {
            header,
            order_reference_number: c.be_u64()?,
        }),

        b'U' => ItchMessage::OrderReplace(OrderReplace {
            header,
            original_order_reference_number: c.be_u64()?,
            new_order_reference_number: c.be_u64()?,
            shares: c.be_u32()?,
            price: Price::from_price4(c.be_u32()?),
        }),

        b'P' => ItchMessage::TradeNonCross(TradeNonCross {
            header,
            order_reference_number: c.be_u64()?,
            side: BuySell::parse(c.ascii_char()?)?,
            shares: c.be_u32()?,
            stock: c.space_padded(8, "Stock")?,
            price: Price::from_price4(c.be_u32()?),
            match_number: c.be_u64()?,
        }),

        b'Q' => ItchMessage::CrossTrade(CrossTrade {
            header,
            shares: c.be_u64()?,
            stock: c.space_padded(8, "Stock")?,
            cross_price: Price::from_price4(c.be_u32()?),
            match_number: c.be_u64()?,
            cross_type: CrossType::parse(c.ascii_char()?)?,
        }),

        b'B' => ItchMessage::BrokenTrade(BrokenTrade {
            header,
            match_number: c.be_u64()?,
        }),

        b'I' => ItchMessage::NetOrderImbalance(NetOrderImbalance {
            header,
            paired_shares: c.be_u64()?,
            imbalance_shares: c.be_u64()?,
            imbalance_direction: ImbalanceDirection::parse(c.ascii_char()?)?,
            stock: c.space_padded(8, "Stock")?,
            far_price: Price::from_price4(c.be_u32()?),
            near_price: Price::from_price4(c.be_u32()?),
            current_reference_price: Price::from_price4(c.be_u32()?),
            cross_type: CrossType::parse(c.ascii_char()?)?,
            price_variation_indicator: c.ascii_char()?,
        }),

        b'N' => ItchMessage::RetailPriceImprovement(RetailPriceImprovement {
            header,
            stock: c.space_padded(8, "Stock")?,
            interest_flag: c.ascii_char()?,
        }),

        b'O' => ItchMessage::DirectListingCapitalRaise(DirectListingCapitalRaise {
            header,
            stock: c.space_padded(8, "Stock")?,
            open_eligible: c.ascii_char()? == 'Y',
            minimum_allowable_price: Price::from_price4(c.be_u32()?),
            maximum_allowable_price: Price::from_price4(c.be_u32()?),
            near_execution_price: Price::from_price4(c.be_u32()?),
            near_execution_time: c.be_u64()?,
            lower_price_collar: Price::from_price4(c.be_u32()?),
            upper_price_collar: Price::from_price4(c.be_u32()?),
        }),

        _ => unreachable!("message_length() already rejected unknown types"),
    })
}
