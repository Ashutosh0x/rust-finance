//! Nasdaq TotalView-ITCH 5.0: decoding, symbol directory, and the mapping onto the
//! venue-neutral book event stream.

pub mod encode;
pub mod messages;

use std::collections::HashMap;

use exchange_core::book::BookSet;
use exchange_core::feed::{BookEvent, ImbalanceSide, Side, TradeCondition, TradingState};
use exchange_core::latency::{now_monotonic_ns, FeedLatency};
use exchange_core::{InstrumentKey, Nanos, WireResult};

pub use messages::{
    decode, message_length, peek_stock_locate, AddOrder, BrokenTrade, BuySell, CrossTrade,
    CrossType, DirectListingCapitalRaise, Header, ImbalanceDirection, IpoQuotingPeriodUpdate,
    ItchMessage, LuldAuctionCollar, MarketParticipantPosition, MwcbDeclineLevel, MwcbStatus,
    NetOrderImbalance, OperationalHalt, OrderCancel, OrderDelete, OrderExecuted,
    OrderExecutedWithPrice, OrderReplace, RegShoAction, RegShoRestriction, RetailPriceImprovement,
    StockDirectory, StockTradingAction, SystemEvent, SystemEventCode, TradeNonCross,
    TradingStateCode,
};

/// Referential data for one instrument, captured from the Stock Directory spin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentInfo {
    pub symbol: String,
    pub market_category: char,
    pub financial_status: char,
    pub round_lot_size: u32,
    pub round_lots_only: bool,
    pub etp: bool,
    pub etp_leverage_factor: u32,
    pub inverse_etp: bool,
    pub luld_tier: char,
    /// `P` live/production. Anything else must not be displayed as real market data.
    pub authenticity: char,
}

impl InstrumentInfo {
    /// True only for issues Nasdaq marks live/production. Test and demo symbols carry real
    /// looking prices and must never reach a strategy.
    pub fn is_production(&self) -> bool {
        self.authenticity == 'P'
    }
}

/// Locate-code → instrument map, rebuilt each session.
///
/// Locate codes "are dynamically assigned each day, starting with a value of 1", so this is
/// explicitly session scoped: carrying yesterday's map into today silently mislabels every
/// instrument.
#[derive(Debug, Default, Clone)]
pub struct SymbolDirectory {
    by_locate: HashMap<u16, InstrumentInfo>,
    by_symbol: HashMap<String, u16>,
}

impl SymbolDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.by_locate.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_locate.is_empty()
    }

    /// Absorb a Stock Directory message.
    pub fn observe(&mut self, msg: &StockDirectory<'_>) {
        let info = InstrumentInfo {
            symbol: msg.stock.to_string(),
            market_category: msg.market_category,
            financial_status: msg.financial_status,
            round_lot_size: msg.round_lot_size,
            round_lots_only: msg.round_lots_only,
            etp: msg.etp_flag == 'Y',
            etp_leverage_factor: msg.etp_leverage_factor,
            inverse_etp: msg.inverse_etp,
            luld_tier: msg.luld_reference_price_tier,
            authenticity: msg.authenticity,
        };
        self.by_symbol
            .insert(info.symbol.clone(), msg.header.stock_locate);
        self.by_locate.insert(msg.header.stock_locate, info);
    }

    pub fn get(&self, locate: u16) -> Option<&InstrumentInfo> {
        self.by_locate.get(&locate)
    }

    pub fn symbol(&self, locate: u16) -> Option<&str> {
        self.by_locate.get(&locate).map(|i| i.symbol.as_str())
    }

    pub fn locate(&self, symbol: &str) -> Option<u16> {
        self.by_symbol.get(symbol).copied()
    }

    pub fn clear(&mut self) {
        self.by_locate.clear();
        self.by_symbol.clear();
    }
}

/// Converts ITCH nanoseconds-since-midnight into nanoseconds since the UNIX epoch.
///
/// ITCH timestamps are relative to midnight in US/Eastern, which is a moving target across
/// DST boundaries. Rather than embed a timezone database in the feed handler, the caller
/// supplies the epoch of the session's local midnight once per day.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionClock {
    midnight_epoch_nanos: u64,
}

impl SessionClock {
    pub const fn new(midnight_epoch_nanos: u64) -> Self {
        Self {
            midnight_epoch_nanos,
        }
    }

    /// Leaves timestamps as raw nanoseconds-since-midnight. Only appropriate for capture
    /// analysis where every timestamp shares the same unstated session date.
    pub const fn raw() -> Self {
        Self {
            midnight_epoch_nanos: 0,
        }
    }

    #[inline]
    pub const fn to_epoch(self, since_midnight: u64) -> Nanos {
        self.midnight_epoch_nanos + since_midnight
    }
}

/// Map a decoded ITCH message onto the venue-neutral event stream.
///
/// Returns `None` for messages that carry no book or tape semantics (referential and
/// session-lifecycle messages); the caller handles those separately.
///
/// Every ITCH message carries the stock locate at a fixed offset, so no order-id → symbol
/// lookup table is needed here: the instrument key is always present on the wire.
pub fn to_book_event<'a>(msg: &ItchMessage<'a>, clock: SessionClock) -> Option<BookEvent<'a>> {
    let h = msg.header();
    let key = h.stock_locate as InstrumentKey;
    let ts = clock.to_epoch(h.timestamp);

    Some(match msg {
        ItchMessage::AddOrder(m) => BookEvent::Add {
            key,
            symbol: m.stock,
            ts,
            order_id: m.order_reference_number,
            side: match m.side {
                BuySell::Buy => Side::Buy,
                BuySell::Sell => Side::Sell,
            },
            price: m.price,
            qty: m.shares as u64,
            participant: m.attribution,
        },

        ItchMessage::OrderExecuted(m) => BookEvent::Execute {
            key,
            ts,
            order_id: m.order_reference_number,
            qty: m.executed_shares as u64,
            price: None,
            trade_id: m.match_number,
            condition: TradeCondition::Printable,
        },

        ItchMessage::OrderExecutedWithPrice(m) => BookEvent::Execute {
            key,
            ts,
            order_id: m.order_reference_number,
            qty: m.executed_shares as u64,
            price: Some(m.execution_price),
            trade_id: m.match_number,
            condition: if m.printable {
                TradeCondition::Printable
            } else {
                TradeCondition::NonPrintable
            },
        },

        ItchMessage::OrderCancel(m) => BookEvent::Reduce {
            key,
            ts,
            order_id: m.order_reference_number,
            qty: m.cancelled_shares as u64,
        },

        ItchMessage::OrderDelete(m) => BookEvent::Delete {
            key,
            ts,
            order_id: m.order_reference_number,
        },

        ItchMessage::OrderReplace(m) => BookEvent::Replace {
            key,
            ts,
            old_order_id: m.original_order_reference_number,
            new_order_id: m.new_order_reference_number,
            price: m.price,
            qty: m.shares as u64,
        },

        // Hidden-liquidity print. The side field has been pinned to 'B' since 2014-07-14
        // regardless of the resting side, so it is deliberately not propagated.
        ItchMessage::TradeNonCross(m) => BookEvent::Trade {
            key,
            symbol: m.stock,
            ts,
            price: m.price,
            qty: m.shares as u64,
            trade_id: m.match_number,
            side: None,
            condition: TradeCondition::Printable,
        },

        ItchMessage::CrossTrade(m) => BookEvent::Trade {
            key,
            symbol: m.stock,
            ts,
            price: m.cross_price,
            qty: m.shares,
            trade_id: m.match_number,
            side: None,
            condition: TradeCondition::Printable,
        },

        ItchMessage::BrokenTrade(m) => BookEvent::Bust {
            key,
            ts,
            trade_id: m.match_number,
        },

        ItchMessage::StockTradingAction(m) => BookEvent::Status {
            key,
            symbol: m.stock,
            ts,
            state: match m.trading_state {
                TradingStateCode::Trading => TradingState::Trading,
                TradingStateCode::Halted => TradingState::Halted,
                TradingStateCode::Paused => TradingState::Paused,
                TradingStateCode::QuotationOnly => TradingState::QuotationOnly,
            },
            reason: m.reason,
        },

        ItchMessage::OperationalHalt(m) => BookEvent::Status {
            key,
            symbol: m.stock,
            ts,
            state: if m.halted {
                TradingState::OperationalHalt
            } else {
                TradingState::Trading
            },
            reason: "operational",
        },

        ItchMessage::NetOrderImbalance(m) => BookEvent::Imbalance {
            key,
            ts,
            paired_qty: m.paired_shares,
            imbalance_qty: m.imbalance_shares,
            side: match m.imbalance_direction {
                ImbalanceDirection::Buy => ImbalanceSide::Buy,
                ImbalanceDirection::Sell => ImbalanceSide::Sell,
                ImbalanceDirection::None => ImbalanceSide::None,
                ImbalanceDirection::InsufficientOrders => ImbalanceSide::InsufficientOrders,
                ImbalanceDirection::Paused => ImbalanceSide::Paused,
            },
            reference_price: m.current_reference_price,
            near_price: m.near_price,
            far_price: m.far_price,
        },

        // Referential and session-lifecycle messages carry no book semantics.
        ItchMessage::SystemEvent(_)
        | ItchMessage::StockDirectory(_)
        | ItchMessage::RegShoRestriction(_)
        | ItchMessage::MarketParticipantPosition(_)
        | ItchMessage::MwcbDeclineLevel(_)
        | ItchMessage::MwcbStatus(_)
        | ItchMessage::IpoQuotingPeriodUpdate(_)
        | ItchMessage::LuldAuctionCollar(_)
        | ItchMessage::RetailPriceImprovement(_)
        | ItchMessage::DirectListingCapitalRaise(_) => return None,
    })
}

/// Counters for what a session has processed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HandlerStats {
    pub messages_decoded: u64,
    pub decode_errors: u64,
    pub book_events: u64,
    pub book_errors: u64,
    pub filtered_out: u64,
}

/// Decodes ITCH, maintains the symbol directory and the books, and records latency.
///
/// Optionally filters by locate code, which is why [`peek_stock_locate`] exists: for a
/// twenty-symbol strategy on the full Nasdaq tape, over 99% of messages are discarded
/// before any field is parsed.
#[derive(Debug)]
pub struct ItchFeedHandler {
    directory: SymbolDirectory,
    books: BookSet,
    clock: SessionClock,
    /// `None` means "every instrument".
    watch: Option<std::collections::HashSet<u16>>,
    stats: HandlerStats,
    latency: FeedLatency,
    session_state: Option<SystemEventCode>,
}

impl ItchFeedHandler {
    pub fn new(clock: SessionClock) -> Self {
        Self {
            directory: SymbolDirectory::new(),
            books: BookSet::new(),
            clock,
            watch: None,
            stats: HandlerStats::default(),
            latency: FeedLatency::new(),
            session_state: None,
        }
    }

    /// Restrict processing to a set of locate codes.
    ///
    /// Locates are only known after the Stock Directory spin, so a symbol-based caller
    /// should feed the whole spin first and then call this with the resolved locates.
    pub fn watch_locates(&mut self, locates: impl IntoIterator<Item = u16>) {
        self.watch = Some(locates.into_iter().collect());
    }

    pub fn watch_all(&mut self) {
        self.watch = None;
    }

    pub fn directory(&self) -> &SymbolDirectory {
        &self.directory
    }

    pub fn books(&self) -> &BookSet {
        &self.books
    }

    pub fn stats(&self) -> HandlerStats {
        self.stats
    }

    pub fn latency(&self) -> &FeedLatency {
        &self.latency
    }

    /// The last System Event seen. `EndOfMessages` means the day's stream is complete.
    pub fn session_state(&self) -> Option<SystemEventCode> {
        self.session_state
    }

    /// Feed one delimited ITCH message.
    ///
    /// Returns the venue-neutral event that was applied to the books, so a caller that also
    /// needs to publish downstream does not have to decode the message a second time. The
    /// event borrows `raw`, which is what keeps that free.
    ///
    /// `recv_ns` is the local monotonic receive timestamp used for the decode-latency
    /// histogram; pass 0 when replaying a capture, where it is meaningless.
    pub fn on_message<'a>(
        &mut self,
        raw: &'a [u8],
        recv_ns: u64,
    ) -> WireResult<Option<BookEvent<'a>>> {
        if let (Some(watch), Some(locate)) = (self.watch.as_ref(), peek_stock_locate(raw)) {
            // Locate 0 marks messages that are not instrument specific (system events,
            // circuit breakers); those are always relevant.
            if locate != 0 && !watch.contains(&locate) {
                self.stats.filtered_out += 1;
                return Ok(None);
            }
        }

        let msg = match decode(raw) {
            Ok(m) => m,
            Err(e) => {
                self.stats.decode_errors += 1;
                return Err(e);
            }
        };
        self.stats.messages_decoded += 1;

        match &msg {
            ItchMessage::StockDirectory(sd) => self.directory.observe(sd),
            ItchMessage::SystemEvent(se) => {
                self.session_state = Some(se.event);
                if se.event == SystemEventCode::StartOfMessages {
                    // A new trading day reassigns every locate code.
                    self.directory.clear();
                    self.books = BookSet::new();
                }
            }
            _ => {}
        }

        // Split so the two stages are attributable separately: decoding is
        // this crate's parser, applying is the shared book. Reporting them as
        // one number hides which of the two regressed.
        let decoded_ns = if recv_ns != 0 { now_monotonic_ns() } else { 0 };

        let applied = to_book_event(&msg, self.clock);
        if let Some(event) = &applied {
            self.stats.book_events += 1;
            if self.books.apply(event).is_err() {
                self.stats.book_errors += 1;
            }
        }

        if recv_ns != 0 {
            self.latency.decode.record_span(recv_ns, decoded_ns);
            // Only when a book event was actually produced — charging the book
            // stage for a message that never touched it would dilute the very
            // tail the histogram exists to expose.
            if applied.is_some() {
                self.latency
                    .book
                    .record_span(decoded_ns, now_monotonic_ns());
            }
        }
        Ok(applied)
    }
}

// `now_monotonic_ns` moved to exchange_core::latency so every crate in a
// tick-to-trade span reads from one origin.

#[cfg(test)]
mod tests {
    use super::*;
    use exchange_core::Price;

    fn h(locate: u16, ts: u64) -> Header {
        Header {
            stock_locate: locate,
            tracking_number: 7,
            timestamp: ts,
        }
    }

    /// 09:30:00 ET expressed the way ITCH does: nanoseconds since local midnight.
    const OPEN_NS: u64 = 34_200_000_000_000;

    #[test]
    fn every_message_type_round_trips_at_the_documented_length() {
        let cases: Vec<(u8, Vec<u8>)> = vec![
            (b'S', encode::system_event(h(0, OPEN_NS), 'Q')),
            (
                b'R',
                encode::stock_directory(
                    h(1, OPEN_NS),
                    "AAPL",
                    'Q',
                    'N',
                    100,
                    false,
                    'C',
                    "",
                    'P',
                    'N',
                    'N',
                    '1',
                    'N',
                    0,
                    false,
                ),
            ),
            (
                b'H',
                encode::stock_trading_action(h(1, OPEN_NS), "AAPL", 'T', ""),
            ),
            (b'Y', encode::reg_sho(h(1, OPEN_NS), "AAPL", '0')),
            (
                b'L',
                encode::market_participant_position(h(1, OPEN_NS), "NSDQ", "AAPL", true, 'N', 'A'),
            ),
            (
                b'V',
                encode::mwcb_decline_level(
                    h(0, OPEN_NS),
                    Price::from_price8(100),
                    Price::from_price8(200),
                    Price::from_price8(300),
                ),
            ),
            (b'W', encode::mwcb_status(h(0, OPEN_NS), '1')),
            (
                b'K',
                encode::ipo_quoting_period(
                    h(2, OPEN_NS),
                    "NEWCO",
                    34_200,
                    'A',
                    Price::from_price4(170_000),
                ),
            ),
            (
                b'J',
                encode::luld_auction_collar(
                    h(1, OPEN_NS),
                    "AAPL",
                    Price::from_price4(1_000_000),
                    Price::from_price4(1_050_000),
                    Price::from_price4(950_000),
                    1,
                ),
            ),
            (
                b'h',
                encode::operational_halt(h(1, OPEN_NS), "AAPL", 'Q', true),
            ),
            (
                b'A',
                encode::add_order(
                    h(1, OPEN_NS),
                    42,
                    'B',
                    500,
                    "AAPL",
                    Price::from_price4(1_000_000),
                    None,
                ),
            ),
            (
                b'F',
                encode::add_order(
                    h(1, OPEN_NS),
                    43,
                    'S',
                    300,
                    "AAPL",
                    Price::from_price4(1_000_100),
                    Some("NSDQ"),
                ),
            ),
            (b'E', encode::order_executed(h(1, OPEN_NS), 42, 100, 900)),
            (
                b'C',
                encode::order_executed_with_price(
                    h(1, OPEN_NS),
                    42,
                    100,
                    901,
                    false,
                    Price::from_price4(999_900),
                ),
            ),
            (b'X', encode::order_cancel(h(1, OPEN_NS), 42, 50)),
            (b'D', encode::order_delete(h(1, OPEN_NS), 42)),
            (
                b'U',
                encode::order_replace(h(1, OPEN_NS), 42, 99, 400, Price::from_price4(1_000_200)),
            ),
            (
                b'P',
                encode::trade_non_cross(
                    h(1, OPEN_NS),
                    0,
                    'B',
                    250,
                    "AAPL",
                    Price::from_price4(1_000_050),
                    902,
                ),
            ),
            (
                b'Q',
                encode::cross_trade(
                    h(1, OPEN_NS),
                    1_000_000,
                    "AAPL",
                    Price::from_price4(1_000_000),
                    903,
                    'O',
                ),
            ),
            (b'B', encode::broken_trade(h(1, OPEN_NS), 903)),
            (
                b'I',
                encode::noii(
                    h(1, OPEN_NS),
                    5_000,
                    1_200,
                    'B',
                    "AAPL",
                    Price::from_price4(1_000_000),
                    Price::from_price4(1_000_100),
                    Price::from_price4(1_000_050),
                    'O',
                    'L',
                ),
            ),
            (
                b'N',
                encode::retail_price_improvement(h(1, OPEN_NS), "AAPL", 'A'),
            ),
            (
                b'O',
                encode::direct_listing_capital_raise(
                    h(3, OPEN_NS),
                    "DLCR",
                    true,
                    Price::from_price4(80_000),
                    Price::from_price4(180_000),
                    Price::from_price4(100_000),
                    OPEN_NS,
                    Price::from_price4(90_000),
                    Price::from_price4(110_000),
                ),
            ),
        ];

        for (ty, bytes) in cases {
            assert_eq!(
                bytes.len(),
                message_length(ty).unwrap(),
                "encoded length for type {}",
                ty as char
            );
            let msg = decode(&bytes).unwrap_or_else(|e| panic!("type {}: {e}", ty as char));
            assert_eq!(msg.message_type(), ty, "type byte for {}", ty as char);
            assert_eq!(msg.header().timestamp, OPEN_NS);
            assert_eq!(msg.header().tracking_number, 7);
        }
    }

    #[test]
    fn add_order_fields_decode_exactly() {
        let bytes = encode::add_order(
            h(11, OPEN_NS),
            0xDEAD_BEEF,
            'S',
            1_234,
            "MSFT",
            Price::from_price4(4_205_000),
            Some("MSCO"),
        );
        let ItchMessage::AddOrder(m) = decode(&bytes).unwrap() else {
            panic!("expected AddOrder");
        };
        assert_eq!(m.header.stock_locate, 11);
        assert_eq!(m.order_reference_number, 0xDEAD_BEEF);
        assert_eq!(m.side, BuySell::Sell);
        assert_eq!(m.shares, 1_234);
        assert_eq!(m.stock, "MSFT");
        assert_eq!(m.price.to_string(), "420.50");
        assert_eq!(m.attribution, Some("MSCO"));
    }

    #[test]
    fn unattributed_add_order_is_four_bytes_shorter() {
        let with = encode::add_order(h(1, 1), 1, 'B', 1, "A", Price::ZERO, Some("NSDQ"));
        let without = encode::add_order(h(1, 1), 1, 'B', 1, "A", Price::ZERO, None);
        assert_eq!(with.len() - without.len(), 4);
        assert_eq!(with[0], b'F');
        assert_eq!(without[0], b'A');
    }

    #[test]
    fn stock_locate_is_readable_without_decoding() {
        let bytes = encode::add_order(h(9_999, OPEN_NS), 1, 'B', 1, "AAPL", Price::ZERO, None);
        assert_eq!(peek_stock_locate(&bytes), Some(9_999));
    }

    #[test]
    fn a_wrong_length_is_rejected_rather_than_misread() {
        let mut bytes = encode::order_delete(h(1, OPEN_NS), 42);
        bytes.push(0);
        assert!(matches!(
            decode(&bytes),
            Err(exchange_core::WireError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn an_unknown_type_byte_is_reported_with_its_ascii() {
        let err = decode(&[b'z'; 19]).unwrap_err();
        assert!(matches!(
            err,
            exchange_core::WireError::UnknownMessageType { got_ascii: 'z', .. }
        ));
    }

    #[test]
    fn an_invalid_enum_value_is_rejected() {
        let mut bytes = encode::add_order(h(1, 1), 1, 'B', 1, "AAPL", Price::ZERO, None);
        bytes[19] = b'X'; // Buy/Sell Indicator
        assert!(matches!(
            decode(&bytes),
            Err(exchange_core::WireError::InvalidEnum {
                field: "Buy/Sell Indicator",
                ..
            })
        ));
    }

    #[test]
    fn mwcb_levels_use_eight_implied_decimals() {
        // 8 implied decimals: 2_650_12345678 is $26,501.2345678.
        let raw: u64 = 265_012_345_678;
        let mut w = exchange_core::wire::Writer::new();
        w.u8(b'V').be_u16(0).be_u16(0);
        w.raw(&0u64.to_be_bytes()[2..]);
        w.be_u64(raw).be_u64(raw).be_u64(raw);
        let bytes = w.into_vec();
        let ItchMessage::MwcbDeclineLevel(m) = decode(&bytes).unwrap() else {
            panic!("expected MwcbDeclineLevel");
        };
        assert_eq!(m.level1, Price::from_price8(raw));
        assert_eq!(m.level1.to_string(), "2650.12345678");
    }

    #[test]
    fn directory_maps_locate_to_symbol_and_back() {
        let bytes = encode::stock_directory(
            h(77, OPEN_NS),
            "TSLA",
            'Q',
            'N',
            100,
            false,
            'C',
            "",
            'P',
            'N',
            'N',
            '1',
            'N',
            0,
            false,
        );
        let ItchMessage::StockDirectory(sd) = decode(&bytes).unwrap() else {
            panic!("expected StockDirectory");
        };
        let mut dir = SymbolDirectory::new();
        dir.observe(&sd);
        assert_eq!(dir.symbol(77), Some("TSLA"));
        assert_eq!(dir.locate("TSLA"), Some(77));
        assert!(dir.get(77).unwrap().is_production());
    }

    #[test]
    fn test_symbols_are_flagged_as_non_production() {
        let bytes = encode::stock_directory(
            h(1, 1),
            "ZVZZT",
            'Q',
            'N',
            100,
            false,
            'C',
            "",
            'T', // authenticity = Test
            'N',
            'N',
            '1',
            'N',
            0,
            false,
        );
        let ItchMessage::StockDirectory(sd) = decode(&bytes).unwrap() else {
            panic!()
        };
        let mut dir = SymbolDirectory::new();
        dir.observe(&sd);
        assert!(!dir.get(1).unwrap().is_production());
    }

    #[test]
    fn session_clock_rebases_onto_the_unix_epoch() {
        // 2024-01-02 00:00:00 America/New_York = 1704171600 epoch seconds.
        let clock = SessionClock::new(1_704_171_600_000_000_000);
        assert_eq!(clock.to_epoch(OPEN_NS), 1_704_205_800_000_000_000);
    }

    #[test]
    fn handler_builds_a_book_from_a_message_sequence() {
        let mut hd = ItchFeedHandler::new(SessionClock::raw());
        let msgs = vec![
            encode::system_event(h(0, 1), 'O'),
            encode::stock_directory(
                h(1, 2),
                "AAPL",
                'Q',
                'N',
                100,
                false,
                'C',
                "",
                'P',
                'N',
                'N',
                '1',
                'N',
                0,
                false,
            ),
            encode::add_order(
                h(1, 3),
                100,
                'B',
                500,
                "AAPL",
                Price::from_price4(1_000_000),
                None,
            ),
            encode::add_order(
                h(1, 4),
                101,
                'S',
                300,
                "AAPL",
                Price::from_price4(1_000_200),
                None,
            ),
            encode::order_executed(h(1, 5), 101, 100, 5000),
            encode::order_cancel(h(1, 6), 100, 200),
        ];
        for m in &msgs {
            hd.on_message(m, 0).unwrap();
        }

        let book = hd.books().get(1).expect("book for locate 1");
        assert_eq!(book.symbol(), "AAPL");
        assert_eq!(book.best_bid(), Some((Price::from_price4(1_000_000), 300)));
        assert_eq!(book.best_ask(), Some((Price::from_price4(1_000_200), 200)));
        assert_eq!(book.stats().printed_volume, 100);
        assert_eq!(hd.stats().book_errors, 0);
        assert_eq!(hd.directory().symbol(1), Some("AAPL"));
    }

    #[test]
    fn start_of_day_clears_stale_locate_assignments() {
        let mut hd = ItchFeedHandler::new(SessionClock::raw());
        hd.on_message(
            &encode::stock_directory(
                h(1, 1),
                "OLDCO",
                'Q',
                'N',
                100,
                false,
                'C',
                "",
                'P',
                'N',
                'N',
                '1',
                'N',
                0,
                false,
            ),
            0,
        )
        .unwrap();
        assert_eq!(hd.directory().symbol(1), Some("OLDCO"));

        hd.on_message(&encode::system_event(h(0, 2), 'O'), 0)
            .unwrap();
        assert!(hd.directory().is_empty(), "locate codes are day scoped");
    }

    #[test]
    fn watch_list_discards_other_instruments_before_decoding() {
        let mut hd = ItchFeedHandler::new(SessionClock::raw());
        hd.watch_locates([1u16]);
        hd.on_message(
            &encode::add_order(
                h(1, 1),
                1,
                'B',
                100,
                "AAPL",
                Price::from_price4(1_000_000),
                None,
            ),
            0,
        )
        .unwrap();
        hd.on_message(
            &encode::add_order(
                h(2, 2),
                2,
                'B',
                100,
                "MSFT",
                Price::from_price4(4_000_000),
                None,
            ),
            0,
        )
        .unwrap();
        assert_eq!(hd.stats().messages_decoded, 1);
        assert_eq!(hd.stats().filtered_out, 1);
        assert!(hd.books().get(2).is_none());
    }

    #[test]
    fn system_events_reach_a_filtered_handler_because_locate_is_zero() {
        let mut hd = ItchFeedHandler::new(SessionClock::raw());
        hd.watch_locates([42u16]);
        hd.on_message(&encode::system_event(h(0, 1), 'Q'), 0)
            .unwrap();
        assert_eq!(
            hd.session_state(),
            Some(SystemEventCode::StartOfMarketHours)
        );
    }

    #[test]
    fn non_printable_executions_do_not_double_count_cross_volume() {
        let mut hd = ItchFeedHandler::new(SessionClock::raw());
        hd.on_message(
            &encode::add_order(
                h(1, 1),
                1,
                'S',
                1_000,
                "AAPL",
                Price::from_price4(1_000_000),
                None,
            ),
            0,
        )
        .unwrap();
        hd.on_message(
            &encode::order_executed_with_price(
                h(1, 2),
                1,
                1_000,
                5,
                false,
                Price::from_price4(1_000_000),
            ),
            0,
        )
        .unwrap();
        hd.on_message(
            &encode::cross_trade(
                h(1, 3),
                1_000,
                "AAPL",
                Price::from_price4(1_000_000),
                6,
                'O',
            ),
            0,
        )
        .unwrap();
        // The individual non-printable execution is excluded; only the bulk print counts.
        assert_eq!(hd.books().get(1).unwrap().stats().printed_volume, 1_000);
    }

    #[test]
    fn replace_preserves_the_side_of_the_original_order() {
        let mut hd = ItchFeedHandler::new(SessionClock::raw());
        hd.on_message(
            &encode::add_order(
                h(1, 1),
                1,
                'S',
                100,
                "AAPL",
                Price::from_price4(1_000_000),
                None,
            ),
            0,
        )
        .unwrap();
        hd.on_message(
            &encode::order_replace(h(1, 2), 1, 2, 200, Price::from_price4(1_000_500)),
            0,
        )
        .unwrap();
        let book = hd.books().get(1).unwrap();
        assert_eq!(book.best_ask(), Some((Price::from_price4(1_000_500), 200)));
        assert_eq!(book.best_bid(), None);
    }

    #[test]
    fn a_halt_message_updates_trading_state() {
        let mut hd = ItchFeedHandler::new(SessionClock::raw());
        hd.on_message(
            &encode::add_order(
                h(1, 1),
                1,
                'B',
                100,
                "AAPL",
                Price::from_price4(1_000_000),
                None,
            ),
            0,
        )
        .unwrap();
        hd.on_message(&encode::stock_trading_action(h(1, 2), "AAPL", 'H', "T1"), 0)
            .unwrap();
        assert_eq!(
            hd.books().get(1).unwrap().trading_state(),
            TradingState::Halted
        );
    }

    #[test]
    fn referential_messages_produce_no_book_event() {
        let bytes = encode::reg_sho(h(1, 1), "AAPL", '1');
        let msg = decode(&bytes).unwrap();
        assert!(to_book_event(&msg, SessionClock::raw()).is_none());
    }
}
