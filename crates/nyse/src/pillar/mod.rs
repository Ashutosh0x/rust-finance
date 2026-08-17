//! NYSE Pillar order entry.
//!
//! Two protocols reach the same matching engines:
//!
//! * [`binary`] — the Pillar Binary Gateway. Fixed-layout little-endian messages over a
//!   stream protocol, with order attributes packed into a 64-bit bitfield. Lowest latency,
//!   most certification work.
//! * [`fix`] — the Pillar FIX Gateway, FIX 4.2 plus NYSE extension tags. Slower, but an
//!   existing FIX OMS can reach it with configuration rather than new code.
//!
//! Both are covered because the choice is a real trade-off, not a default: a firm running a
//! FIX OMS today can go live on the FIX gateway and migrate hot paths to binary later
//! without changing anything above [`crate`].
//!
//! One identifier ties order entry to market data on both: the exchange-assigned `OrderID`
//! on an acknowledgement is the same value the Integrated feed publishes for that order, so
//! a firm can locate its own resting order in the public book. On the binary gateway it is
//! a full 8-byte value; the Integrated feed's 4-byte `TradeID` needs the `System ID` and
//! `Market ID` from the Symbol Index Mapping message prepended to match the gateway's
//! 8-byte `DealID`.

pub mod binary;
pub mod fix;

/// Which order-entry protocol a session speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Binary,
    Fix,
}

impl Protocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Binary => "pillar-binary",
            Self::Fix => "pillar-fix",
        }
    }
}

/// NYSE Group equities markets reachable through Pillar, with the `Market ID` published in
/// the Symbol Index Mapping message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    Nyse,
    NyseArcaEquities,
    NyseArcaOptions,
    NyseBonds,
    NyseAmericanOptions,
    NyseAmericanEquities,
    NyseNationalEquities,
    /// Market ID 11, rebranded from NYSE Chicago.
    NyseTexas,
}

impl Market {
    pub const fn market_id(self) -> u16 {
        match self {
            Self::Nyse => 1,
            Self::NyseArcaEquities => 3,
            Self::NyseArcaOptions => 4,
            Self::NyseBonds => 5,
            Self::NyseAmericanOptions => 8,
            Self::NyseAmericanEquities => 9,
            Self::NyseNationalEquities => 10,
            Self::NyseTexas => 11,
        }
    }

    pub const fn from_market_id(id: u16) -> Option<Self> {
        Some(match id {
            1 => Self::Nyse,
            3 => Self::NyseArcaEquities,
            4 => Self::NyseArcaOptions,
            5 => Self::NyseBonds,
            8 => Self::NyseAmericanOptions,
            9 => Self::NyseAmericanEquities,
            10 => Self::NyseNationalEquities,
            11 => Self::NyseTexas,
            _ => return None,
        })
    }

    /// Maximum order quantity the matching engine accepts.
    ///
    /// NYSE allows 25,000,000 on auction orders (MOO, LOO, MOC, LOC, imbalance offset and
    /// D orders) and 5,000,000 otherwise; every other equities market is 5,000,000. NYSE
    /// Texas allows 25,000,000 on cross orders.
    pub const fn max_order_qty(self, auction_or_cross: bool) -> u32 {
        match (self, auction_or_cross) {
            (Self::Nyse, true) | (Self::NyseTexas, true) => 25_000_000,
            _ => 5_000_000,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nyse => "NYSE",
            Self::NyseArcaEquities => "NYSE Arca Equities",
            Self::NyseArcaOptions => "NYSE Arca Options",
            Self::NyseBonds => "NYSE Bonds",
            Self::NyseAmericanOptions => "NYSE American Options",
            Self::NyseAmericanEquities => "NYSE American Equities",
            Self::NyseNationalEquities => "NYSE National Equities",
            Self::NyseTexas => "NYSE Texas",
        }
    }
}

/// Rebuild the 8-byte gateway identifier from a 4-byte Integrated Feed `TradeID`.
///
/// Per the Pillar Common Client Specification's correlation rules, prepending a zero byte,
/// the `System ID` and the `Market ID` from the Symbol Index Mapping message to a 4-byte
/// feed id yields the value that appears as `DealID` on the gateway's Execution Report.
/// Without this the two sides look unrelated.
pub const fn correlate_id(system_id: u8, market_id: u16, feed_id: u32) -> u64 {
    // Layout (little-endian client byte ordering): [0][SystemID][MarketID:2][ID:4]
    (feed_id as u64) << 32 | (market_id as u64) << 16 | (system_id as u64) << 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_ids_round_trip() {
        for m in [
            Market::Nyse,
            Market::NyseArcaEquities,
            Market::NyseArcaOptions,
            Market::NyseBonds,
            Market::NyseAmericanOptions,
            Market::NyseAmericanEquities,
            Market::NyseNationalEquities,
            Market::NyseTexas,
        ] {
            assert_eq!(Market::from_market_id(m.market_id()), Some(m));
        }
        assert_eq!(Market::from_market_id(99), None);
    }

    #[test]
    fn auction_quantity_limits_differ_by_market() {
        assert_eq!(Market::Nyse.max_order_qty(true), 25_000_000);
        assert_eq!(Market::Nyse.max_order_qty(false), 5_000_000);
        assert_eq!(Market::NyseTexas.max_order_qty(true), 25_000_000);
        assert_eq!(Market::NyseArcaEquities.max_order_qty(true), 5_000_000);
        assert_eq!(Market::NyseNationalEquities.max_order_qty(false), 5_000_000);
    }

    #[test]
    fn correlating_a_feed_id_places_the_fields_at_the_documented_offsets() {
        let id = correlate_id(0x03, 0x0001, 0xAABB_CCDD);
        assert_eq!(id.to_le_bytes()[0], 0, "offset 0 is zero");
        assert_eq!(id.to_le_bytes()[1], 0x03, "SystemID at offset 1");
        assert_eq!(
            u16::from_le_bytes([id.to_le_bytes()[2], id.to_le_bytes()[3]]),
            1,
            "MarketID at offset 2"
        );
        assert_eq!(
            u32::from_le_bytes(id.to_le_bytes()[4..8].try_into().unwrap()),
            0xAABB_CCDD,
            "feed id at offset 4"
        );
    }

    #[test]
    fn two_markets_with_the_same_feed_id_correlate_differently() {
        let a = correlate_id(1, Market::Nyse.market_id(), 42);
        let b = correlate_id(1, Market::NyseArcaEquities.market_id(), 42);
        assert_ne!(
            a, b,
            "the market id is what disambiguates a shared trade id"
        );
    }
}
