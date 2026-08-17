//! Venue-neutral order-book events.
//!
//! Both venues publish *order-based* (level 3) feeds, and although the wire formats differ
//! completely the semantics line up almost one for one:
//!
//! | Concept              | Nasdaq ITCH 5.0                       | NYSE Pillar Integrated |
//! |----------------------|---------------------------------------|------------------------|
//! | new displayed order  | `A` / `F` Add Order                   | `100` Add Order        |
//! | size/price change    | `X` Order Cancel (partial)            | `101` Modify Order     |
//! | cancel/replace       | `U` Order Replace                     | `104` Replace Order    |
//! | removal              | `D` Order Delete                      | `102` Delete Order     |
//! | execution of a book order | `E` / `C` Order Executed         | `103` Order Execution  |
//! | hidden-order print   | `P` Trade (non-cross)                 | `110` Non-Displayed Trade |
//! | auction print        | `Q` Cross Trade                       | `111` Cross Trade      |
//! | bust                 | `B` Broken Trade                      | `112` Trade Cancel     |
//! | auction imbalance    | `I` NOII                              | `105` Imbalance        |
//!
//! Mapping to this enum is what lets one [`crate::book::OrderBook`] implementation serve
//! both feeds.

use crate::price::Price;
use crate::{InstrumentKey, Nanos, OrderId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }

    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }
}

/// Auction imbalance direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImbalanceSide {
    Buy,
    Sell,
    None,
    /// ITCH `O`: insufficient orders to calculate.
    InsufficientOrders,
    /// ITCH `P`: the security is paused.
    Paused,
}

/// Whether a print counts toward consolidated volume.
///
/// ITCH marks non-printable executions so that cross volume is not double counted; the
/// Integrated feed carries the same idea in `PrintableFlag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeCondition {
    Printable,
    NonPrintable,
}

impl TradeCondition {
    #[inline]
    pub const fn counts_toward_volume(self) -> bool {
        matches!(self, TradeCondition::Printable)
    }
}

/// Why an instrument is not trading normally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingState {
    /// Trading normally on this venue.
    Trading,
    /// Halted across all US equity markets.
    Halted,
    /// LULD pause (Nasdaq-listed only on ITCH; `M` halt condition on Pillar).
    Paused,
    /// Quotation-only period during a cross-SRO halt or pause.
    QuotationOnly,
    /// Venue-specific operational halt: the instrument still trades elsewhere.
    OperationalHalt,
    /// Pre-open / closed / other session state.
    NotTrading,
}

/// The normalised event stream consumed by [`crate::book::OrderBook`].
///
/// Lifetimes: `symbol` borrows from the decode buffer where the wire format carries a
/// ticker inline (ITCH), and is empty where the wire format carries only a numeric key and
/// the symbol must be resolved from a directory (XDP).
#[derive(Debug, Clone, PartialEq)]
pub enum BookEvent<'a> {
    /// A new displayed order joined the book.
    Add {
        key: InstrumentKey,
        symbol: &'a str,
        ts: Nanos,
        order_id: OrderId,
        side: Side,
        price: Price,
        qty: u64,
        /// Nasdaq MPID / NYSE FirmID when the order is attributed.
        participant: Option<&'a str>,
    },
    /// Displayed size and/or price changed in place.
    Modify {
        key: InstrumentKey,
        ts: Nanos,
        order_id: OrderId,
        price: Option<Price>,
        qty: u64,
        /// True when the change costs the order its time priority.
        lost_priority: bool,
    },
    /// The order was removed and re-added under a new id.
    Replace {
        key: InstrumentKey,
        ts: Nanos,
        old_order_id: OrderId,
        new_order_id: OrderId,
        price: Price,
        qty: u64,
    },
    /// Displayed size reduced by `qty` (a partial cancel, priority retained).
    Reduce {
        key: InstrumentKey,
        ts: Nanos,
        order_id: OrderId,
        qty: u64,
    },
    /// The order left the book entirely.
    Delete {
        key: InstrumentKey,
        ts: Nanos,
        order_id: OrderId,
    },
    /// A resting order traded. `price` is `None` when the trade happened at the order's
    /// display price (ITCH `E`), `Some` when it printed away from it (ITCH `C`, XDP 103).
    Execute {
        key: InstrumentKey,
        ts: Nanos,
        order_id: OrderId,
        qty: u64,
        price: Option<Price>,
        trade_id: u64,
        condition: TradeCondition,
    },
    /// A print with no book impact: hidden liquidity, or a bulk auction print.
    Trade {
        key: InstrumentKey,
        symbol: &'a str,
        ts: Nanos,
        price: Price,
        qty: u64,
        trade_id: u64,
        /// `None` for auction prints and for ITCH trade messages after 2014-07-14, where
        /// the resting side is no longer disclosed.
        side: Option<Side>,
        condition: TradeCondition,
    },
    /// A previously reported trade was busted.
    Bust {
        key: InstrumentKey,
        ts: Nanos,
        trade_id: u64,
    },
    /// Trading status change.
    Status {
        key: InstrumentKey,
        symbol: &'a str,
        ts: Nanos,
        state: TradingState,
        reason: &'a str,
    },
    /// Auction imbalance (ITCH NOII / XDP Imbalance).
    Imbalance {
        key: InstrumentKey,
        ts: Nanos,
        paired_qty: u64,
        imbalance_qty: u64,
        side: ImbalanceSide,
        reference_price: Price,
        near_price: Price,
        far_price: Price,
    },
    /// Drop all state for this instrument and await a fresh snapshot.
    ///
    /// Emitted by XDP `Symbol Clear` (msg type 32) and synthesised on an unrecoverable
    /// sequence gap.
    Clear { key: InstrumentKey, ts: Nanos },
}

impl BookEvent<'_> {
    /// The instrument this event applies to.
    #[inline]
    pub const fn key(&self) -> InstrumentKey {
        match *self {
            BookEvent::Add { key, .. }
            | BookEvent::Modify { key, .. }
            | BookEvent::Replace { key, .. }
            | BookEvent::Reduce { key, .. }
            | BookEvent::Delete { key, .. }
            | BookEvent::Execute { key, .. }
            | BookEvent::Trade { key, .. }
            | BookEvent::Bust { key, .. }
            | BookEvent::Status { key, .. }
            | BookEvent::Imbalance { key, .. }
            | BookEvent::Clear { key, .. } => key,
        }
    }

    /// Exchange timestamp.
    #[inline]
    pub const fn ts(&self) -> Nanos {
        match *self {
            BookEvent::Add { ts, .. }
            | BookEvent::Modify { ts, .. }
            | BookEvent::Replace { ts, .. }
            | BookEvent::Reduce { ts, .. }
            | BookEvent::Delete { ts, .. }
            | BookEvent::Execute { ts, .. }
            | BookEvent::Trade { ts, .. }
            | BookEvent::Bust { ts, .. }
            | BookEvent::Status { ts, .. }
            | BookEvent::Imbalance { ts, .. }
            | BookEvent::Clear { ts, .. } => ts,
        }
    }

    /// True for events that mutate the resting book.
    #[inline]
    pub const fn mutates_book(&self) -> bool {
        matches!(
            self,
            BookEvent::Add { .. }
                | BookEvent::Modify { .. }
                | BookEvent::Replace { .. }
                | BookEvent::Reduce { .. }
                | BookEvent::Delete { .. }
                | BookEvent::Execute { .. }
                | BookEvent::Clear { .. }
        )
    }
}
