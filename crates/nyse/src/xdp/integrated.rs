//! NYSE Pillar Integrated Feed — the order-by-order data messages.
//!
//! The Integrated Feed is the full depth of book: every displayed order, every change to
//! one, and every print. Message types 100–114 are the order and trade messages; the
//! referential and control messages that make them interpretable live in [`super::common`].
//!
//! Two properties of this feed shape every decoder below:
//!
//! * **Prices are raw numerators.** A price field means nothing until it is divided by
//!   `10^PriceScaleCode` from the symbol's Symbol Index Mapping message, and the scale
//!   differs per symbol. Each struct therefore exposes `*_raw` numerators, and scaling is a
//!   separate step performed once the directory is known.
//! * **Timestamps are nanoseconds only.** `SourceTimeNS` is an offset within a second whose
//!   value arrived in the last Source Time Reference message for this matching-engine
//!   partition. Messages 105, 106 and 113's siblings are the exceptions that carry the full
//!   `SourceTime` as well.

use exchange_core::wire::{Cursor, Writer};
use exchange_core::{FirmId5, WireResult};

use super::packet::expect_min_size;

/// Integrated Feed message type codes.
pub mod msg_type {
    pub const ADD_ORDER: u16 = 100;
    pub const MODIFY_ORDER: u16 = 101;
    pub const DELETE_ORDER: u16 = 102;
    pub const ORDER_EXECUTION: u16 = 103;
    pub const REPLACE_ORDER: u16 = 104;
    pub const IMBALANCE: u16 = 105;
    pub const ADD_ORDER_REFRESH: u16 = 106;
    pub const NON_DISPLAYED_TRADE: u16 = 110;
    pub const CROSS_TRADE: u16 = 111;
    pub const TRADE_CANCEL: u16 = 112;
    pub const CROSS_CORRECTION: u16 = 113;
    pub const RETAIL_PRICE_IMPROVEMENT: u16 = 114;
    pub const STOCK_SUMMARY: u16 = 223;
}

/// Side of an order, as published on this feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub const fn code(self) -> char {
        match self {
            Self::Buy => 'B',
            Self::Sell => 'S',
        }
    }

    /// Anything other than `B` is treated as a sell, matching the feed's two documented
    /// values without failing a whole packet on an unexpected byte.
    pub const fn parse(ch: char) -> Self {
        match ch {
            'B' => Self::Buy,
            _ => Self::Sell,
        }
    }
}

/// The four trade-condition bytes carried on execution and trade messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TradeConditions {
    /// Settlement: `@` regular, `C` cash, `N` next day.
    pub settlement: char,
    /// Trade-through exemption: `F` ISO, `O` opening, `5` reopening, `6` closing, `7` QCT.
    pub exemption: char,
    /// Extended hours / sequencing: `T`, `U`, `Z`.
    pub extended_hours: char,
    /// SRO detail: `I` odd lot, `V` contingent.
    pub sro_detail: char,
}

impl TradeConditions {
    fn read(c: &mut Cursor<'_>) -> WireResult<Self> {
        Ok(Self {
            settlement: c.ascii_char()?,
            exemption: c.ascii_char()?,
            extended_hours: c.ascii_char()?,
            sro_detail: c.ascii_char()?,
        })
    }

    fn write(&self, w: &mut Writer) {
        w.ascii_char(self.settlement)
            .ascii_char(self.exemption)
            .ascii_char(self.extended_hours)
            .ascii_char(self.sro_detail);
    }

    /// True for an auction print (opening, reopening or closing).
    pub const fn is_auction(&self) -> bool {
        matches!(self.exemption, 'O' | '5' | '6')
    }

    /// True for an odd-lot print.
    pub const fn is_odd_lot(&self) -> bool {
        self.sro_detail == 'I'
    }
}

/// Type 100 — a new displayed order joined the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddOrder {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    /// Matches the `OrderID` in the Pillar gateway Order Ack for the firm that entered it.
    pub order_id: u64,
    pub price_raw: i32,
    pub volume: u32,
    pub side: Side,
    /// Blank unless the order is attributed. Stored inline: this is the highest-volume
    /// message on the feed, and a heap allocation here was two thirds of its decode cost.
    pub firm_id: FirmId5,
}

impl AddOrder {
    pub const SIZE: usize = 39;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::ADD_ORDER, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            order_id: c.le_u64()?,
            price_raw: c.le_i32()?,
            volume: c.le_u32()?,
            side: Side::parse(c.ascii_char()?),
            firm_id: FirmId5::from_wire(c.take(5)?),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::ADD_ORDER)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u64(self.order_id)
            .le_u32(self.price_raw as u32)
            .le_u32(self.volume)
            .ascii_char(self.side.code())
            .raw(self.firm_id.as_bytes())
            .u8(0); // Reserved 1
        w.into_vec()
    }
}

/// Type 106 — an Add Order republished as part of a refresh (snapshot).
///
/// Identical content to type 100 plus the full `SourceTime`, because a snapshot is replayed
/// outside the normal Source Time Reference cadence and would otherwise have no seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddOrderRefresh {
    pub source_time_secs: u32,
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub order_id: u64,
    pub price_raw: i32,
    pub volume: u32,
    pub side: Side,
    pub firm_id: FirmId5,
}

impl AddOrderRefresh {
    pub const SIZE: usize = 43;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::ADD_ORDER_REFRESH, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_secs: c.le_u32()?,
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            order_id: c.le_u64()?,
            price_raw: c.le_i32()?,
            volume: c.le_u32()?,
            side: Side::parse(c.ascii_char()?),
            firm_id: FirmId5::from_wire(c.take(5)?),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::ADD_ORDER_REFRESH)
            .le_u32(self.source_time_secs)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u64(self.order_id)
            .le_u32(self.price_raw as u32)
            .le_u32(self.volume)
            .ascii_char(self.side.code())
            .raw(self.firm_id.as_bytes())
            .u8(0);
        w.into_vec()
    }
}

/// Type 101 — price or volume changed without a cancel/replace or an execution.
///
/// The fields carry the values *after* the change, not deltas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifyOrder {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub order_id: u64,
    pub price_raw: i32,
    pub volume: u32,
    /// 0 kept book position, 1 lost it. A price change always loses position; an unchanged
    /// price always keeps it.
    pub position_change: u8,
}

impl ModifyOrder {
    pub const SIZE: usize = 35;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::MODIFY_ORDER, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            order_id: c.le_u64()?,
            price_raw: c.le_i32()?,
            volume: c.le_u32()?,
            position_change: c.u8()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::MODIFY_ORDER)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u64(self.order_id)
            .le_u32(self.price_raw as u32)
            .le_u32(self.volume)
            .u8(self.position_change)
            .u8(0)
            .u8(0); // Reserved 1, 2
        w.into_vec()
    }

    pub const fn lost_position(&self) -> bool {
        self.position_change == 1
    }
}

/// Type 104 — cancel/replace. The old order must be removed and the new one added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplaceOrder {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub order_id: u64,
    pub new_order_id: u64,
    pub price_raw: i32,
    pub volume: u32,
}

impl ReplaceOrder {
    pub const SIZE: usize = 42;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::REPLACE_ORDER, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            order_id: c.le_u64()?,
            new_order_id: c.le_u64()?,
            price_raw: c.le_i32()?,
            volume: c.le_u32()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::REPLACE_ORDER)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u64(self.order_id)
            .le_u64(self.new_order_id)
            .le_u32(self.price_raw as u32)
            .le_u32(self.volume)
            .u8(0)
            .u8(0);
        w.into_vec()
    }
}

/// Type 102 — the order left the book for a reason other than a full execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteOrder {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub order_id: u64,
}

impl DeleteOrder {
    pub const SIZE: usize = 25;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::DELETE_ORDER, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            order_id: c.le_u64()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::DELETE_ORDER)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u64(self.order_id)
            .u8(0);
        w.into_vec()
    }
}

/// Type 103 — a resting order was partially or fully executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderExecution {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub order_id: u64,
    /// Matches the low 4 bytes of the `DealID` in the gateway Execution Report.
    pub trade_id: u32,
    pub price_raw: i32,
    pub volume: u32,
    /// 0 not printed to the SIP, 1 printed. Auction executions are always 0 so that the
    /// bulk Cross Trade print is not double counted.
    pub printable_flag: u8,
    pub conditions: TradeConditions,
}

impl OrderExecution {
    pub const SIZE: usize = 42;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::ORDER_EXECUTION, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        let source_time_nanos = c.le_u32()?;
        let symbol_index = c.le_u32()?;
        let symbol_seq_num = c.le_u32()?;
        let order_id = c.le_u64()?;
        let trade_id = c.le_u32()?;
        let price_raw = c.le_i32()?;
        let volume = c.le_u32()?;
        let printable_flag = c.u8()?;
        c.skip(1)?; // Reserved 1
        let conditions = TradeConditions::read(&mut c)?;
        Ok(Self {
            source_time_nanos,
            symbol_index,
            symbol_seq_num,
            order_id,
            trade_id,
            price_raw,
            volume,
            printable_flag,
            conditions,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::ORDER_EXECUTION)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u64(self.order_id)
            .le_u32(self.trade_id)
            .le_u32(self.price_raw as u32)
            .le_u32(self.volume)
            .u8(self.printable_flag)
            .u8(0);
        self.conditions.write(&mut w);
        w.into_vec()
    }

    pub const fn printed_to_sip(&self) -> bool {
        self.printable_flag == 1
    }
}

/// Type 110 — a match between two non-displayed orders. No book impact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonDisplayedTrade {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub trade_id: u32,
    pub price_raw: i32,
    pub volume: u32,
    pub printable_flag: u8,
    pub conditions: TradeConditions,
}

impl NonDisplayedTrade {
    pub const SIZE: usize = 33;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::NON_DISPLAYED_TRADE, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            trade_id: c.le_u32()?,
            price_raw: c.le_i32()?,
            volume: c.le_u32()?,
            printable_flag: c.u8()?,
            conditions: TradeConditions::read(&mut c)?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::NON_DISPLAYED_TRADE)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u32(self.trade_id)
            .le_u32(self.price_raw as u32)
            .le_u32(self.volume)
            .u8(self.printable_flag);
        self.conditions.write(&mut w);
        w.into_vec()
    }

    pub const fn printed_to_sip(&self) -> bool {
        self.printable_flag == 1
    }
}

/// Type 111 — the bulk print for one symbol's auction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossTrade {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub cross_id: u32,
    pub price_raw: i32,
    pub volume: u32,
    /// `E` early open, `O` open, `5` reopen, `6` close.
    pub cross_type: char,
}

impl CrossTrade {
    pub const SIZE: usize = 29;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::CROSS_TRADE, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            cross_id: c.le_u32()?,
            price_raw: c.le_i32()?,
            volume: c.le_u32()?,
            cross_type: c.ascii_char()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::CROSS_TRADE)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u32(self.cross_id)
            .le_u32(self.price_raw as u32)
            .le_u32(self.volume)
            .ascii_char(self.cross_type);
        w.into_vec()
    }
}

/// Type 112 — a previously reported execution or trade was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeCancel {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub trade_id: u32,
}

impl TradeCancel {
    pub const SIZE: usize = 20;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::TRADE_CANCEL, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            trade_id: c.le_u32()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::TRADE_CANCEL)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u32(self.trade_id);
        w.into_vec()
    }
}

/// Type 113 — a previously reported cross had the wrong volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossCorrection {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub cross_id: u32,
    /// The corrected volume, which replaces the original.
    pub volume: u32,
}

impl CrossCorrection {
    pub const SIZE: usize = 24;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::CROSS_CORRECTION, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            cross_id: c.le_u32()?,
            volume: c.le_u32()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::CROSS_CORRECTION)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u32(self.cross_id)
            .le_u32(self.volume);
        w.into_vec()
    }
}

/// Type 114 — hidden retail price improvement interest appeared or disappeared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetailPriceImprovement {
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    /// Space none, `A` bid, `B` offer, `C` both.
    pub rpi_indicator: char,
}

impl RetailPriceImprovement {
    pub const SIZE: usize = 17;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::RETAIL_PRICE_IMPROVEMENT, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            rpi_indicator: c.ascii_char()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::RETAIL_PRICE_IMPROVEMENT)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .ascii_char(self.rpi_indicator);
        w.into_vec()
    }
}

/// Type 105 — auction imbalance, published once a second during an auction.
///
/// Several fields are documented as NYSE-only or non-NYSE-only and are defaulted to 0 on the
/// other markets; the semantics of `PairedQty` and `TotalImbalanceQty` also differ (NYSE
/// computes them at the Reference Price, other markets at the Indicative Match Price).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Imbalance {
    pub source_time_secs: u32,
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub symbol_seq_num: u32,
    pub reference_price_raw: i32,
    pub paired_qty: u32,
    pub total_imbalance_qty: u32,
    /// Market-order imbalance at the indicative match price. 0 on NYSE.
    pub market_imbalance_qty: u32,
    /// Projected auction time as `hhmm`.
    pub auction_time: u16,
    /// `O` early open, `M` core open, `H` reopen, `C` close, `P` extreme closing imbalance
    /// (NYSE), `R` regulatory closing imbalance (NYSE).
    pub auction_type: char,
    /// `B`, `S`, or space for none.
    pub imbalance_side: char,
    pub continuous_book_clearing_price_raw: i32,
    pub auction_interest_clearing_price_raw: i32,
    pub ssr_filing_price_raw: i32,
    pub indicative_match_price_raw: i32,
    pub upper_collar_raw: i32,
    pub lower_collar_raw: i32,
    /// 0 will run, 1 will run with interest inside the collars, 2 will not run (imbalance
    /// through the collars), 3 will not run and transitions to the closing auction.
    pub auction_status: u8,
    /// 1 while the imbalance freeze is in effect.
    pub freeze_status: u8,
    pub num_extensions: u8,
    /// NYSE closing auction only.
    pub unpaired_qty: u32,
    pub unpaired_side: char,
    /// `Y` when the current imbalance is significant.
    pub significant_imbalance: char,
}

impl Imbalance {
    pub const SIZE: usize = 73;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::IMBALANCE, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_secs: c.le_u32()?,
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            symbol_seq_num: c.le_u32()?,
            reference_price_raw: c.le_i32()?,
            paired_qty: c.le_u32()?,
            total_imbalance_qty: c.le_u32()?,
            market_imbalance_qty: c.le_u32()?,
            auction_time: c.le_u16()?,
            auction_type: c.ascii_char()?,
            imbalance_side: c.ascii_char()?,
            continuous_book_clearing_price_raw: c.le_i32()?,
            auction_interest_clearing_price_raw: c.le_i32()?,
            ssr_filing_price_raw: c.le_i32()?,
            indicative_match_price_raw: c.le_i32()?,
            upper_collar_raw: c.le_i32()?,
            lower_collar_raw: c.le_i32()?,
            auction_status: c.u8()?,
            freeze_status: c.u8()?,
            num_extensions: c.u8()?,
            unpaired_qty: c.le_u32()?,
            unpaired_side: c.ascii_char()?,
            significant_imbalance: c.ascii_char()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::IMBALANCE)
            .le_u32(self.source_time_secs)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.symbol_seq_num)
            .le_u32(self.reference_price_raw as u32)
            .le_u32(self.paired_qty)
            .le_u32(self.total_imbalance_qty)
            .le_u32(self.market_imbalance_qty)
            .le_u16(self.auction_time)
            .ascii_char(self.auction_type)
            .ascii_char(self.imbalance_side)
            .le_u32(self.continuous_book_clearing_price_raw as u32)
            .le_u32(self.auction_interest_clearing_price_raw as u32)
            .le_u32(self.ssr_filing_price_raw as u32)
            .le_u32(self.indicative_match_price_raw as u32)
            .le_u32(self.upper_collar_raw as u32)
            .le_u32(self.lower_collar_raw as u32)
            .u8(self.auction_status)
            .u8(self.freeze_status)
            .u8(self.num_extensions)
            .le_u32(self.unpaired_qty)
            .ascii_char(self.unpaired_side)
            .ascii_char(self.significant_imbalance);
        w.into_vec()
    }

    /// True when the auction will not run as scheduled.
    pub const fn auction_will_run(&self) -> bool {
        self.auction_status <= 1
    }
}

/// Type 223 — per-symbol summary published every 60 seconds on a separate channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockSummary {
    pub source_time_secs: u32,
    pub source_time_nanos: u32,
    pub symbol_index: u32,
    pub high_price_raw: i32,
    pub low_price_raw: i32,
    pub open_price_raw: i32,
    pub close_price_raw: i32,
    pub total_volume: u32,
}

impl StockSummary {
    pub const SIZE: usize = 36;

    pub fn parse(bytes: &[u8]) -> WireResult<Self> {
        expect_min_size(bytes, msg_type::STOCK_SUMMARY, Self::SIZE)?;
        let mut c = Cursor::at(bytes, 4);
        Ok(Self {
            source_time_secs: c.le_u32()?,
            source_time_nanos: c.le_u32()?,
            symbol_index: c.le_u32()?,
            high_price_raw: c.le_i32()?,
            low_price_raw: c.le_i32()?,
            open_price_raw: c.le_i32()?,
            close_price_raw: c.le_i32()?,
            total_volume: c.le_u32()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::SIZE);
        w.le_u16(Self::SIZE as u16)
            .le_u16(msg_type::STOCK_SUMMARY)
            .le_u32(self.source_time_secs)
            .le_u32(self.source_time_nanos)
            .le_u32(self.symbol_index)
            .le_u32(self.high_price_raw as u32)
            .le_u32(self.low_price_raw as u32)
            .le_u32(self.open_price_raw as u32)
            .le_u32(self.close_price_raw as u32)
            .le_u32(self.total_volume);
        w.into_vec()
    }
}

/// Any Integrated Feed data message.
#[derive(Debug, Clone, PartialEq)]
pub enum IntegratedMessage {
    AddOrder(AddOrder),
    AddOrderRefresh(AddOrderRefresh),
    ModifyOrder(ModifyOrder),
    ReplaceOrder(ReplaceOrder),
    DeleteOrder(DeleteOrder),
    OrderExecution(OrderExecution),
    NonDisplayedTrade(NonDisplayedTrade),
    CrossTrade(CrossTrade),
    TradeCancel(TradeCancel),
    CrossCorrection(CrossCorrection),
    RetailPriceImprovement(RetailPriceImprovement),
    Imbalance(Box<Imbalance>),
    StockSummary(StockSummary),
}

impl IntegratedMessage {
    /// The symbol index every data message carries.
    pub const fn symbol_index(&self) -> u32 {
        match self {
            Self::AddOrder(m) => m.symbol_index,
            Self::AddOrderRefresh(m) => m.symbol_index,
            Self::ModifyOrder(m) => m.symbol_index,
            Self::ReplaceOrder(m) => m.symbol_index,
            Self::DeleteOrder(m) => m.symbol_index,
            Self::OrderExecution(m) => m.symbol_index,
            Self::NonDisplayedTrade(m) => m.symbol_index,
            Self::CrossTrade(m) => m.symbol_index,
            Self::TradeCancel(m) => m.symbol_index,
            Self::CrossCorrection(m) => m.symbol_index,
            Self::RetailPriceImprovement(m) => m.symbol_index,
            Self::Imbalance(m) => m.symbol_index,
            Self::StockSummary(m) => m.symbol_index,
        }
    }

    /// Per-symbol sequence number, where the message carries one. Stock Summary does not.
    pub const fn symbol_seq_num(&self) -> Option<u32> {
        Some(match self {
            Self::AddOrder(m) => m.symbol_seq_num,
            Self::AddOrderRefresh(m) => m.symbol_seq_num,
            Self::ModifyOrder(m) => m.symbol_seq_num,
            Self::ReplaceOrder(m) => m.symbol_seq_num,
            Self::DeleteOrder(m) => m.symbol_seq_num,
            Self::OrderExecution(m) => m.symbol_seq_num,
            Self::NonDisplayedTrade(m) => m.symbol_seq_num,
            Self::CrossTrade(m) => m.symbol_seq_num,
            Self::TradeCancel(m) => m.symbol_seq_num,
            Self::CrossCorrection(m) => m.symbol_seq_num,
            Self::RetailPriceImprovement(m) => m.symbol_seq_num,
            Self::Imbalance(m) => m.symbol_seq_num,
            Self::StockSummary(_) => return None,
        })
    }

    /// The nanosecond offset within the current Source Time Reference second.
    pub const fn source_time_nanos(&self) -> u32 {
        match self {
            Self::AddOrder(m) => m.source_time_nanos,
            Self::AddOrderRefresh(m) => m.source_time_nanos,
            Self::ModifyOrder(m) => m.source_time_nanos,
            Self::ReplaceOrder(m) => m.source_time_nanos,
            Self::DeleteOrder(m) => m.source_time_nanos,
            Self::OrderExecution(m) => m.source_time_nanos,
            Self::NonDisplayedTrade(m) => m.source_time_nanos,
            Self::CrossTrade(m) => m.source_time_nanos,
            Self::TradeCancel(m) => m.source_time_nanos,
            Self::CrossCorrection(m) => m.source_time_nanos,
            Self::RetailPriceImprovement(m) => m.source_time_nanos,
            Self::Imbalance(m) => m.source_time_nanos,
            Self::StockSummary(m) => m.source_time_nanos,
        }
    }

    /// Full seconds, for the message types that carry them inline (refresh and imbalance).
    pub const fn source_time_secs(&self) -> Option<u32> {
        match self {
            Self::AddOrderRefresh(m) => Some(m.source_time_secs),
            Self::Imbalance(m) => Some(m.source_time_secs),
            Self::StockSummary(m) => Some(m.source_time_secs),
            _ => None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::AddOrder(m) => m.encode(),
            Self::AddOrderRefresh(m) => m.encode(),
            Self::ModifyOrder(m) => m.encode(),
            Self::ReplaceOrder(m) => m.encode(),
            Self::DeleteOrder(m) => m.encode(),
            Self::OrderExecution(m) => m.encode(),
            Self::NonDisplayedTrade(m) => m.encode(),
            Self::CrossTrade(m) => m.encode(),
            Self::TradeCancel(m) => m.encode(),
            Self::CrossCorrection(m) => m.encode(),
            Self::RetailPriceImprovement(m) => m.encode(),
            Self::Imbalance(m) => m.encode(),
            Self::StockSummary(m) => m.encode(),
        }
    }
}

/// Decode an Integrated Feed data message, or `None` if `msg_type` is not one of them
/// (it may still be a control message — see [`super::common::decode_control`]).
pub fn decode(msg_type: u16, bytes: &[u8]) -> WireResult<Option<IntegratedMessage>> {
    Ok(Some(match msg_type {
        msg_type::ADD_ORDER => IntegratedMessage::AddOrder(AddOrder::parse(bytes)?),
        msg_type::ADD_ORDER_REFRESH => {
            IntegratedMessage::AddOrderRefresh(AddOrderRefresh::parse(bytes)?)
        }
        msg_type::MODIFY_ORDER => IntegratedMessage::ModifyOrder(ModifyOrder::parse(bytes)?),
        msg_type::REPLACE_ORDER => IntegratedMessage::ReplaceOrder(ReplaceOrder::parse(bytes)?),
        msg_type::DELETE_ORDER => IntegratedMessage::DeleteOrder(DeleteOrder::parse(bytes)?),
        msg_type::ORDER_EXECUTION => {
            IntegratedMessage::OrderExecution(OrderExecution::parse(bytes)?)
        }
        msg_type::NON_DISPLAYED_TRADE => {
            IntegratedMessage::NonDisplayedTrade(NonDisplayedTrade::parse(bytes)?)
        }
        msg_type::CROSS_TRADE => IntegratedMessage::CrossTrade(CrossTrade::parse(bytes)?),
        msg_type::TRADE_CANCEL => IntegratedMessage::TradeCancel(TradeCancel::parse(bytes)?),
        msg_type::CROSS_CORRECTION => {
            IntegratedMessage::CrossCorrection(CrossCorrection::parse(bytes)?)
        }
        msg_type::RETAIL_PRICE_IMPROVEMENT => {
            IntegratedMessage::RetailPriceImprovement(RetailPriceImprovement::parse(bytes)?)
        }
        msg_type::IMBALANCE => IntegratedMessage::Imbalance(Box::new(Imbalance::parse(bytes)?)),
        msg_type::STOCK_SUMMARY => IntegratedMessage::StockSummary(StockSummary::parse(bytes)?),
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conds() -> TradeConditions {
        TradeConditions {
            settlement: '@',
            exemption: ' ',
            extended_hours: ' ',
            sro_detail: ' ',
        }
    }

    fn all_messages() -> Vec<(IntegratedMessage, usize)> {
        vec![
            (
                IntegratedMessage::AddOrder(AddOrder {
                    source_time_nanos: 123_456_789,
                    symbol_index: 4242,
                    symbol_seq_num: 7,
                    order_id: 0x0102_0304_0506_0708,
                    price_raw: 1_805_000,
                    volume: 500,
                    side: Side::Buy,
                    firm_id: FirmId5::new("ABCD"),
                }),
                AddOrder::SIZE,
            ),
            (
                IntegratedMessage::AddOrderRefresh(AddOrderRefresh {
                    source_time_secs: 1_700_000_000,
                    source_time_nanos: 1,
                    symbol_index: 4242,
                    symbol_seq_num: 8,
                    order_id: 99,
                    price_raw: 1_805_100,
                    volume: 300,
                    side: Side::Sell,
                    firm_id: FirmId5::NUL,
                }),
                AddOrderRefresh::SIZE,
            ),
            (
                IntegratedMessage::ModifyOrder(ModifyOrder {
                    source_time_nanos: 2,
                    symbol_index: 4242,
                    symbol_seq_num: 9,
                    order_id: 99,
                    price_raw: 1_805_200,
                    volume: 200,
                    position_change: 1,
                }),
                ModifyOrder::SIZE,
            ),
            (
                IntegratedMessage::ReplaceOrder(ReplaceOrder {
                    source_time_nanos: 3,
                    symbol_index: 4242,
                    symbol_seq_num: 10,
                    order_id: 99,
                    new_order_id: 100,
                    price_raw: 1_805_300,
                    volume: 400,
                }),
                ReplaceOrder::SIZE,
            ),
            (
                IntegratedMessage::DeleteOrder(DeleteOrder {
                    source_time_nanos: 4,
                    symbol_index: 4242,
                    symbol_seq_num: 11,
                    order_id: 100,
                }),
                DeleteOrder::SIZE,
            ),
            (
                IntegratedMessage::OrderExecution(OrderExecution {
                    source_time_nanos: 5,
                    symbol_index: 4242,
                    symbol_seq_num: 12,
                    order_id: 100,
                    trade_id: 55,
                    price_raw: 1_805_000,
                    volume: 100,
                    printable_flag: 1,
                    conditions: conds(),
                }),
                OrderExecution::SIZE,
            ),
            (
                IntegratedMessage::NonDisplayedTrade(NonDisplayedTrade {
                    source_time_nanos: 6,
                    symbol_index: 4242,
                    symbol_seq_num: 13,
                    trade_id: 56,
                    price_raw: 1_805_050,
                    volume: 250,
                    printable_flag: 1,
                    conditions: conds(),
                }),
                NonDisplayedTrade::SIZE,
            ),
            (
                IntegratedMessage::CrossTrade(CrossTrade {
                    source_time_nanos: 7,
                    symbol_index: 4242,
                    symbol_seq_num: 14,
                    cross_id: 1,
                    price_raw: 1_805_000,
                    volume: 1_000_000,
                    cross_type: 'O',
                }),
                CrossTrade::SIZE,
            ),
            (
                IntegratedMessage::TradeCancel(TradeCancel {
                    source_time_nanos: 8,
                    symbol_index: 4242,
                    symbol_seq_num: 15,
                    trade_id: 56,
                }),
                TradeCancel::SIZE,
            ),
            (
                IntegratedMessage::CrossCorrection(CrossCorrection {
                    source_time_nanos: 9,
                    symbol_index: 4242,
                    symbol_seq_num: 16,
                    cross_id: 1,
                    volume: 900_000,
                }),
                CrossCorrection::SIZE,
            ),
            (
                IntegratedMessage::RetailPriceImprovement(RetailPriceImprovement {
                    source_time_nanos: 10,
                    symbol_index: 4242,
                    symbol_seq_num: 17,
                    rpi_indicator: 'C',
                }),
                RetailPriceImprovement::SIZE,
            ),
            (
                IntegratedMessage::Imbalance(Box::new(Imbalance {
                    source_time_secs: 1_700_000_000,
                    source_time_nanos: 11,
                    symbol_index: 4242,
                    symbol_seq_num: 18,
                    reference_price_raw: 1_805_000,
                    paired_qty: 500_000,
                    total_imbalance_qty: 120_000,
                    market_imbalance_qty: 0,
                    auction_time: 1600,
                    auction_type: 'C',
                    imbalance_side: 'B',
                    continuous_book_clearing_price_raw: 1_805_500,
                    auction_interest_clearing_price_raw: 1_805_200,
                    ssr_filing_price_raw: 0,
                    indicative_match_price_raw: 0,
                    upper_collar_raw: 0,
                    lower_collar_raw: 0,
                    auction_status: 0,
                    freeze_status: 1,
                    num_extensions: 0,
                    unpaired_qty: 20_000,
                    unpaired_side: 'B',
                    significant_imbalance: 'Y',
                })),
                Imbalance::SIZE,
            ),
            (
                IntegratedMessage::StockSummary(StockSummary {
                    source_time_secs: 1_700_000_000,
                    source_time_nanos: 12,
                    symbol_index: 4242,
                    high_price_raw: 1_810_000,
                    low_price_raw: 1_800_000,
                    open_price_raw: 1_805_000,
                    close_price_raw: 1_806_000,
                    total_volume: 4_000_000,
                }),
                StockSummary::SIZE,
            ),
        ]
    }

    #[test]
    fn every_message_round_trips_at_its_documented_size() {
        for (msg, size) in all_messages() {
            let bytes = msg.encode();
            assert_eq!(bytes.len(), size, "wire size for {msg:?}");
            let (declared, ty) = super::super::packet::message_header(&bytes).unwrap();
            assert_eq!(declared as usize, size, "MsgSize field for {msg:?}");
            let back = decode(ty, &bytes).unwrap().expect("known type");
            assert_eq!(back, msg);
        }
    }

    #[test]
    fn every_message_reports_its_symbol_index() {
        for (msg, _) in all_messages() {
            assert_eq!(msg.symbol_index(), 4242);
        }
    }

    #[test]
    fn only_stock_summary_lacks_a_symbol_sequence_number() {
        for (msg, _) in all_messages() {
            match msg {
                IntegratedMessage::StockSummary(_) => assert!(msg.symbol_seq_num().is_none()),
                _ => assert!(msg.symbol_seq_num().is_some(), "{msg:?}"),
            }
        }
    }

    #[test]
    fn refresh_and_imbalance_carry_full_seconds_but_data_messages_do_not() {
        for (msg, _) in all_messages() {
            match msg {
                IntegratedMessage::AddOrderRefresh(_)
                | IntegratedMessage::Imbalance(_)
                | IntegratedMessage::StockSummary(_) => {
                    assert!(msg.source_time_secs().is_some(), "{msg:?}")
                }
                _ => assert!(
                    msg.source_time_secs().is_none(),
                    "{msg:?} should rely on Source Time Reference"
                ),
            }
        }
    }

    #[test]
    fn add_order_field_offsets_match_the_specification() {
        let m = AddOrder {
            source_time_nanos: 0x1122_3344,
            symbol_index: 0x5566_7788,
            symbol_seq_num: 0x99AA_BBCC,
            order_id: 0x0102_0304_0506_0708,
            price_raw: 0x0011_2233,
            volume: 0x4455_6677,
            side: Side::Sell,
            firm_id: FirmId5::new("XYZ"),
        };
        let b = m.encode();
        assert_eq!(u16::from_le_bytes([b[0], b[1]]), 39);
        assert_eq!(u16::from_le_bytes([b[2], b[3]]), 100);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 0x1122_3344);
        assert_eq!(
            u32::from_le_bytes(b[8..12].try_into().unwrap()),
            0x5566_7788
        );
        assert_eq!(
            u32::from_le_bytes(b[12..16].try_into().unwrap()),
            0x99AA_BBCC
        );
        assert_eq!(
            u64::from_le_bytes(b[16..24].try_into().unwrap()),
            0x0102_0304_0506_0708
        );
        assert_eq!(
            u32::from_le_bytes(b[24..28].try_into().unwrap()),
            0x0011_2233
        );
        assert_eq!(
            u32::from_le_bytes(b[28..32].try_into().unwrap()),
            0x4455_6677
        );
        assert_eq!(b[32], b'S');
        assert_eq!(&b[33..38], b"XYZ  ");
    }

    #[test]
    fn auction_executions_are_marked_not_printed_to_the_sip() {
        let m = OrderExecution {
            source_time_nanos: 1,
            symbol_index: 1,
            symbol_seq_num: 1,
            order_id: 1,
            trade_id: 1,
            price_raw: 1,
            volume: 100,
            printable_flag: 0,
            conditions: TradeConditions {
                settlement: '@',
                exemption: 'O',
                extended_hours: ' ',
                sro_detail: ' ',
            },
        };
        assert!(!m.printed_to_sip());
        assert!(m.conditions.is_auction());
    }

    #[test]
    fn odd_lot_prints_are_identifiable() {
        let c = TradeConditions {
            settlement: '@',
            exemption: ' ',
            extended_hours: ' ',
            sro_detail: 'I',
        };
        assert!(c.is_odd_lot());
        assert!(!c.is_auction());
    }

    #[test]
    fn a_price_change_on_modify_costs_book_position() {
        let m = ModifyOrder {
            source_time_nanos: 1,
            symbol_index: 1,
            symbol_seq_num: 1,
            order_id: 1,
            price_raw: 1,
            volume: 1,
            position_change: 1,
        };
        assert!(m.lost_position());
        let bytes = m.encode();
        assert_eq!(ModifyOrder::parse(&bytes).unwrap(), m);
    }

    #[test]
    fn imbalance_reports_whether_the_auction_will_run() {
        let mut m = Imbalance {
            source_time_secs: 1,
            source_time_nanos: 1,
            symbol_index: 1,
            symbol_seq_num: 1,
            reference_price_raw: 0,
            paired_qty: 0,
            total_imbalance_qty: 0,
            market_imbalance_qty: 0,
            auction_time: 930,
            auction_type: 'M',
            imbalance_side: ' ',
            continuous_book_clearing_price_raw: 0,
            auction_interest_clearing_price_raw: 0,
            ssr_filing_price_raw: 0,
            indicative_match_price_raw: 0,
            upper_collar_raw: 0,
            lower_collar_raw: 0,
            auction_status: 0,
            freeze_status: 0,
            num_extensions: 0,
            unpaired_qty: 0,
            unpaired_side: ' ',
            significant_imbalance: ' ',
        };
        assert!(m.auction_will_run());
        m.auction_status = 2;
        assert!(!m.auction_will_run());
    }

    #[test]
    fn negative_price_numerators_survive_the_round_trip() {
        // Pillar Equities does not publish negative prices today, but the field is signed
        // and a sign-losing decoder would be silently wrong if that ever changed.
        let m = AddOrder {
            source_time_nanos: 1,
            symbol_index: 1,
            symbol_seq_num: 1,
            order_id: 1,
            price_raw: -12_345,
            volume: 1,
            side: Side::Buy,
            firm_id: FirmId5::NUL,
        };
        assert_eq!(AddOrder::parse(&m.encode()).unwrap().price_raw, -12_345);
    }

    #[test]
    fn unknown_message_types_are_reported_as_not_ours() {
        assert!(decode(999, &[0u8; 40]).unwrap().is_none());
    }

    #[test]
    fn a_short_message_is_rejected_rather_than_read_past() {
        let bytes = TradeCancel {
            source_time_nanos: 1,
            symbol_index: 1,
            symbol_seq_num: 1,
            trade_id: 1,
        }
        .encode();
        assert!(TradeCancel::parse(&bytes[..bytes.len() - 1]).is_err());
    }
}
