//! NYSE XDP / Pillar market data: framing, messages, recovery and book building.

pub mod common;
pub mod integrated;
pub mod packet;
pub mod receiver;
pub mod request_server;

use std::collections::HashMap;

use exchange_core::book::BookSet;
use exchange_core::feed::{BookEvent, ImbalanceSide, Side, TradeCondition, TradingState};
use exchange_core::latency::FeedLatency;
use exchange_core::{InstrumentKey, Nanos, Price, WireResult};

use common::{ControlMessage, SecurityStatusCode, SymbolIndexMapping};
use integrated::IntegratedMessage;
use packet::{Delivery, Packet};

/// The symbol index → referential data map.
///
/// Unlike Nasdaq locate codes, XDP symbol indexes are stable across days and shared across
/// all Pillar-powered NYSE equity markets, so this map can be seeded from the daily Symbol
/// Index Mapping file and only topped up from the feed. What it must never be is *absent*:
/// without the `PriceScaleCode` every price on the feed is an uninterpretable integer.
#[derive(Debug, Default, Clone)]
pub struct SymbolDirectory {
    by_index: HashMap<u32, SymbolIndexMapping>,
    by_symbol: HashMap<String, u32>,
}

impl SymbolDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.by_index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_index.is_empty()
    }

    /// Absorb a Symbol Index Mapping message.
    ///
    /// Repeats within a day are legal: "the correspondence between the Symbol and the
    /// Symbol Index will not change, but other field values might", and the latest values
    /// win without applying retroactively.
    pub fn observe(&mut self, m: SymbolIndexMapping) {
        self.by_symbol.insert(m.symbol.clone(), m.symbol_index);
        self.by_index.insert(m.symbol_index, m);
    }

    pub fn get(&self, index: u32) -> Option<&SymbolIndexMapping> {
        self.by_index.get(&index)
    }

    pub fn symbol(&self, index: u32) -> Option<&str> {
        self.by_index.get(&index).map(|m| m.symbol.as_str())
    }

    pub fn index(&self, symbol: &str) -> Option<u32> {
        self.by_symbol.get(symbol).copied()
    }

    /// Price scale for a symbol, or `None` when the mapping has not arrived yet.
    pub fn price_scale(&self, index: u32) -> Option<u8> {
        self.by_index.get(&index).map(|m| m.price_scale_code)
    }

    /// Decode a raw price numerator using the symbol's scale.
    ///
    /// Returns `None` rather than guessing a scale: a price decoded at the wrong scale is
    /// off by a factor of 100 or 1000, which is far worse than no price at all.
    pub fn price(&self, index: u32, raw: i32) -> Option<Price> {
        self.price_scale(index).map(|s| Price::from_xdp(raw, s))
    }

    pub fn clear(&mut self) {
        self.by_index.clear();
        self.by_symbol.clear();
    }
}

/// Reassembles full timestamps from the split representation the high-volume feeds use.
///
/// Data messages carry only `SourceTimeNS`. The seconds arrive once per second per
/// matching-engine partition in a Source Time Reference message, so the handler keeps the
/// most recent seconds value per partition and concatenates.
#[derive(Debug, Default, Clone)]
pub struct SourceTimeClock {
    /// Partition id → most recent whole seconds.
    per_partition: HashMap<u32, u32>,
    /// Fallback for feeds that publish a single partition, or before the first reference.
    latest: Option<u32>,
}

impl SourceTimeClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&mut self, partition: u32, secs: u32) {
        self.per_partition.insert(partition, secs);
        self.latest = Some(secs);
    }

    /// Combine a nanosecond offset with the partition's current second.
    ///
    /// Falls back to the most recently seen second from any partition, and finally to 0.
    /// A zero here means "no Source Time Reference has arrived yet", which downstream code
    /// can detect rather than silently trusting a 1970 timestamp.
    pub fn to_epoch_nanos(&self, partition: u32, nanos: u32) -> Nanos {
        let secs = self
            .per_partition
            .get(&partition)
            .copied()
            .or(self.latest)
            .unwrap_or(0);
        secs as u64 * 1_000_000_000 + nanos as u64
    }

    /// True once at least one Source Time Reference has been seen.
    pub fn is_primed(&self) -> bool {
        self.latest.is_some()
    }
}

/// Map an Integrated Feed message onto the venue-neutral event stream.
///
/// Returns `None` when the message has no book or tape semantics, or when the symbol's
/// price scale is not yet known — in the latter case the caller must recover the Symbol
/// Index Mapping before the message can be interpreted at all.
pub fn to_book_event(
    msg: &IntegratedMessage,
    directory: &SymbolDirectory,
    ts: Nanos,
) -> Option<BookEvent<'static>> {
    let index = msg.symbol_index();
    let key = index as InstrumentKey;
    let scale = directory.price_scale(index)?;
    let px = |raw: i32| Price::from_xdp(raw, scale);

    Some(match msg {
        IntegratedMessage::AddOrder(m) => BookEvent::Add {
            key,
            // XDP data messages carry no ticker; the book resolves it from the directory.
            symbol: "",
            ts,
            order_id: m.order_id,
            side: match m.side {
                integrated::Side::Buy => Side::Buy,
                integrated::Side::Sell => Side::Sell,
            },
            price: px(m.price_raw),
            qty: m.volume as u64,
            participant: None,
        },

        IntegratedMessage::AddOrderRefresh(m) => BookEvent::Add {
            key,
            symbol: "",
            ts,
            order_id: m.order_id,
            side: match m.side {
                integrated::Side::Buy => Side::Buy,
                integrated::Side::Sell => Side::Sell,
            },
            price: px(m.price_raw),
            qty: m.volume as u64,
            participant: None,
        },

        IntegratedMessage::ModifyOrder(m) => BookEvent::Modify {
            key,
            ts,
            order_id: m.order_id,
            price: Some(px(m.price_raw)),
            qty: m.volume as u64,
            lost_priority: m.lost_position(),
        },

        IntegratedMessage::ReplaceOrder(m) => BookEvent::Replace {
            key,
            ts,
            old_order_id: m.order_id,
            new_order_id: m.new_order_id,
            price: px(m.price_raw),
            qty: m.volume as u64,
        },

        IntegratedMessage::DeleteOrder(m) => BookEvent::Delete {
            key,
            ts,
            order_id: m.order_id,
        },

        IntegratedMessage::OrderExecution(m) => BookEvent::Execute {
            key,
            ts,
            order_id: m.order_id,
            qty: m.volume as u64,
            price: Some(px(m.price_raw)),
            trade_id: m.trade_id as u64,
            condition: if m.printed_to_sip() {
                TradeCondition::Printable
            } else {
                TradeCondition::NonPrintable
            },
        },

        IntegratedMessage::NonDisplayedTrade(m) => BookEvent::Trade {
            key,
            symbol: "",
            ts,
            price: px(m.price_raw),
            qty: m.volume as u64,
            trade_id: m.trade_id as u64,
            side: None,
            condition: if m.printed_to_sip() {
                TradeCondition::Printable
            } else {
                TradeCondition::NonPrintable
            },
        },

        IntegratedMessage::CrossTrade(m) => BookEvent::Trade {
            key,
            symbol: "",
            ts,
            price: px(m.price_raw),
            qty: m.volume as u64,
            trade_id: m.cross_id as u64,
            side: None,
            condition: TradeCondition::Printable,
        },

        IntegratedMessage::TradeCancel(m) => BookEvent::Bust {
            key,
            ts,
            trade_id: m.trade_id as u64,
        },

        IntegratedMessage::Imbalance(m) => BookEvent::Imbalance {
            key,
            ts,
            paired_qty: m.paired_qty as u64,
            imbalance_qty: m.total_imbalance_qty as u64,
            side: match m.imbalance_side {
                'B' => ImbalanceSide::Buy,
                'S' => ImbalanceSide::Sell,
                _ => ImbalanceSide::None,
            },
            reference_price: px(m.reference_price_raw),
            near_price: px(m.indicative_match_price_raw),
            far_price: px(m.auction_interest_clearing_price_raw),
        },

        // No book or tape impact: a cross correction adjusts historical volume, RPI is an
        // indication of hidden interest, and the stock summary is a periodic digest.
        IntegratedMessage::CrossCorrection(_)
        | IntegratedMessage::RetailPriceImprovement(_)
        | IntegratedMessage::StockSummary(_) => return None,
    })
}

/// Map a control message onto the venue-neutral event stream.
pub fn control_to_book_event(msg: &ControlMessage) -> Option<BookEvent<'static>> {
    match msg {
        ControlMessage::SymbolClear(m) => Some(BookEvent::Clear {
            key: m.symbol_index as InstrumentKey,
            ts: m.source_time_secs as u64 * 1_000_000_000 + m.source_time_nanos as u64,
        }),

        ControlMessage::SecurityStatus(m) => {
            let state = match m.status {
                SecurityStatusCode::TradingHalt => {
                    // An LULD pause is reported as a halt with a specific condition byte.
                    if matches!(m.halt_condition, common::HaltCondition::LuldPause) {
                        TradingState::Paused
                    } else {
                        TradingState::Halted
                    }
                }
                SecurityStatusCode::Resume | SecurityStatusCode::CoreSession => {
                    TradingState::Trading
                }
                SecurityStatusCode::EarlySession | SecurityStatusCode::LateSession => {
                    TradingState::Trading
                }
                SecurityStatusCode::PreOpening
                | SecurityStatusCode::BeginAcceptingOrders
                | SecurityStatusCode::Closed => TradingState::NotTrading,
                // Short-sale-restriction and price-indication updates say nothing about
                // whether the symbol is trading, so they must not overwrite that state.
                _ => return None,
            };
            Some(BookEvent::Status {
                key: m.symbol_index as InstrumentKey,
                symbol: "",
                ts: m.source_time_secs as u64 * 1_000_000_000 + m.source_time_nanos as u64,
                state,
                reason: "",
            })
        }

        _ => None,
    }
}

/// Counters for a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HandlerStats {
    pub packets: u64,
    pub messages_decoded: u64,
    pub decode_errors: u64,
    pub book_events: u64,
    pub book_errors: u64,
    /// Messages dropped because the symbol's price scale was not yet known.
    pub awaiting_symbol_mapping: u64,
    pub unknown_message_types: u64,
    pub filtered_out: u64,
}

/// Decodes XDP packets, maintains the symbol directory, the source-time clock and the books.
#[derive(Debug)]
pub struct XdpFeedHandler {
    directory: SymbolDirectory,
    clock: SourceTimeClock,
    books: BookSet,
    watch: Option<std::collections::HashSet<u32>>,
    stats: HandlerStats,
    latency: FeedLatency,
    /// True while a refresh (snapshot) sequence is being applied.
    in_refresh: bool,
    /// Sequence number to resume the live channel from once the refresh completes.
    refresh_resume_seq: Option<u32>,
    /// Events applied since the caller last drained them.
    ///
    /// XDP data messages carry no inline text, so the mapped events own their data and can
    /// be buffered — which lets one packet's worth of work be published in one go instead
    /// of decoding every message twice.
    applied: Vec<BookEvent<'static>>,
}

impl Default for XdpFeedHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl XdpFeedHandler {
    pub fn new() -> Self {
        Self {
            directory: SymbolDirectory::new(),
            clock: SourceTimeClock::new(),
            books: BookSet::new(),
            watch: None,
            stats: HandlerStats::default(),
            latency: FeedLatency::new(),
            in_refresh: false,
            refresh_resume_seq: None,
            applied: Vec::with_capacity(64),
        }
    }

    /// Seed the directory from the daily Symbol Index Mapping file, so prices are
    /// interpretable from the very first data message rather than after the startup spin.
    pub fn seed_directory(&mut self, mappings: impl IntoIterator<Item = SymbolIndexMapping>) {
        for m in mappings {
            self.directory.observe(m);
        }
    }

    /// Restrict processing to a set of symbol indexes.
    pub fn watch_indexes(&mut self, indexes: impl IntoIterator<Item = u32>) {
        self.watch = Some(indexes.into_iter().collect());
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

    pub fn clock(&self) -> &SourceTimeClock {
        &self.clock
    }

    /// Sequence number the live channel should resume from after the current refresh.
    pub fn refresh_resume_sequence(&self) -> Option<u32> {
        self.refresh_resume_seq
    }

    /// Process one datagram.
    ///
    /// `skip` discards messages at the front of the packet that were already applied — the
    /// partial-overlap case a redundant A/B feed pair produces constantly.
    pub fn on_packet(&mut self, datagram: &[u8], skip: u32) -> WireResult<()> {
        let packet = Packet::parse(datagram)?;
        self.stats.packets += 1;

        match packet.header.delivery {
            Delivery::Refresh {
                first_in_sequence,
                last_in_sequence,
            } => {
                if first_in_sequence {
                    self.in_refresh = true;
                }
                if last_in_sequence {
                    self.in_refresh = false;
                }
            }
            Delivery::MessageUnavailable => {
                tracing::warn!(
                    target: "nyse::xdp",
                    sequence = packet.header.sequence,
                    "requested range is unavailable; a full refresh is the only recovery"
                );
            }
            _ => {}
        }

        for raw in packet.messages().skip(skip as usize) {
            self.on_message(raw.msg_type, raw.bytes)?;
        }
        Ok(())
    }

    fn on_message(&mut self, msg_type: u16, bytes: &[u8]) -> WireResult<()> {
        // Control messages first: they establish the state the data messages need.
        match common::decode_control(msg_type, bytes) {
            Ok(Some(control)) => {
                self.stats.messages_decoded += 1;
                self.apply_control(control);
                return Ok(());
            }
            Ok(None) => {}
            Err(e) => {
                self.stats.decode_errors += 1;
                return Err(e);
            }
        }

        let msg = match integrated::decode(msg_type, bytes) {
            Ok(Some(m)) => m,
            Ok(None) => {
                self.stats.unknown_message_types += 1;
                return Ok(());
            }
            Err(e) => {
                self.stats.decode_errors += 1;
                return Err(e);
            }
        };
        self.stats.messages_decoded += 1;

        if let Some(watch) = self.watch.as_ref() {
            if !watch.contains(&msg.symbol_index()) {
                self.stats.filtered_out += 1;
                return Ok(());
            }
        }

        if self.directory.price_scale(msg.symbol_index()).is_none() {
            // Without the symbol's price scale, decoding the price would be a guess.
            self.stats.awaiting_symbol_mapping += 1;
            return Ok(());
        }

        let ts = match msg.source_time_secs() {
            Some(secs) => secs as u64 * 1_000_000_000 + msg.source_time_nanos() as u64,
            // Partition id is not on the data message; the clock falls back to the most
            // recent reference, which is correct for a single-partition channel.
            None => self.clock.to_epoch_nanos(0, msg.source_time_nanos()),
        };

        if let Some(event) = to_book_event(&msg, &self.directory, ts) {
            self.apply_event(&event, msg.symbol_index());
        }
        Ok(())
    }

    fn apply_control(&mut self, control: ControlMessage) {
        match &control {
            ControlMessage::SymbolIndexMapping(m) => {
                self.directory.observe((**m).clone());
                // Give the book its ticker now that the mapping is known.
                if let Some(book) = self.books.get_mut(m.symbol_index as InstrumentKey) {
                    book.set_symbol(m.symbol.clone());
                }
            }
            ControlMessage::SourceTimeReference(m) => {
                self.clock.observe(m.id, m.source_time_secs);
                // Data messages carry no partition id, so also prime the fallback slot.
                self.clock.observe(0, m.source_time_secs);
            }
            ControlMessage::RefreshHeader(m) => {
                self.refresh_resume_seq = Some(m.last_seq_num);
            }
            _ => {}
        }

        if let Some(event) = control_to_book_event(&control) {
            let index = event.key();
            self.apply_event(&event, index);
        }
    }

    fn apply_event(&mut self, event: &BookEvent<'static>, symbol_index: u32) {
        self.stats.book_events += 1;
        self.applied.push(event.clone());
        match self.books.apply(event) {
            Ok(_) => {
                if let Some(symbol) = self.directory.symbol(symbol_index) {
                    let symbol = symbol.to_string();
                    if let Some(book) = self.books.get_mut(symbol_index as InstrumentKey) {
                        if book.symbol().is_empty() {
                            book.set_symbol(symbol);
                        }
                    }
                }
            }
            Err(e) => {
                self.stats.book_errors += 1;
                tracing::debug!(target: "nyse::xdp", error = %e, "book event rejected");
            }
        }
    }

    /// True while a refresh sequence is in progress.
    pub fn in_refresh(&self) -> bool {
        self.in_refresh
    }

    /// Take the events applied since the last drain.
    ///
    /// Callers that publish downstream drain after each packet; callers that only want the
    /// book state can ignore this, but should drain periodically so the buffer does not
    /// grow for the life of the session.
    pub fn drain_events(&mut self) -> Vec<BookEvent<'static>> {
        std::mem::take(&mut self.applied)
    }

    /// Discard buffered events without returning them.
    pub fn clear_events(&mut self) {
        self.applied.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange_core::feed::Side as CoreSide;

    fn mapping(index: u32, symbol: &str, scale: u8) -> SymbolIndexMapping {
        SymbolIndexMapping {
            symbol_index: index,
            symbol: symbol.into(),
            market_id: 1,
            system_id: 1,
            exchange_code: 'N',
            price_scale_code: scale,
            security_type: 'C',
            lot_size: 100,
            prev_close_price: Price::ZERO,
            prev_close_volume: 0,
            price_resolution: 0,
            round_lot: 'Y',
            mpv: 1,
            unit_of_trade: 100,
        }
    }

    fn add(index: u32, order_id: u64, side: integrated::Side, price_raw: i32, vol: u32) -> Vec<u8> {
        integrated::AddOrder {
            source_time_nanos: 1,
            symbol_index: index,
            symbol_seq_num: 1,
            order_id,
            price_raw,
            volume: vol,
            side,
            firm_id: exchange_core::FirmId5::NUL,
        }
        .encode()
    }

    #[test]
    fn directory_maps_index_to_symbol_and_price_scale() {
        let mut d = SymbolDirectory::new();
        d.observe(mapping(4242, "IBM", 4));
        assert_eq!(d.symbol(4242), Some("IBM"));
        assert_eq!(d.index("IBM"), Some(4242));
        assert_eq!(d.price_scale(4242), Some(4));
        assert_eq!(d.price(4242, 1_805_000).unwrap().to_string(), "180.50");
    }

    #[test]
    fn an_unknown_symbol_yields_no_price_rather_than_a_wrong_one() {
        let d = SymbolDirectory::new();
        assert!(d.price(9999, 1_805_000).is_none());
    }

    #[test]
    fn source_time_clock_concatenates_seconds_and_nanoseconds() {
        let mut c = SourceTimeClock::new();
        assert!(!c.is_primed());
        c.observe(3, 1_700_000_000);
        assert!(c.is_primed());
        assert_eq!(c.to_epoch_nanos(3, 123_456_789), 1_700_000_000_123_456_789);
    }

    #[test]
    fn each_partition_keeps_its_own_second() {
        let mut c = SourceTimeClock::new();
        c.observe(1, 1_700_000_000);
        c.observe(2, 1_700_000_001);
        assert_eq!(c.to_epoch_nanos(1, 5), 1_700_000_000_000_000_005);
        assert_eq!(c.to_epoch_nanos(2, 5), 1_700_000_001_000_000_005);
        // An unseen partition falls back to the most recent second seen anywhere.
        assert_eq!(c.to_epoch_nanos(9, 5), 1_700_000_001_000_000_005);
    }

    #[test]
    fn handler_builds_a_book_from_a_packet_stream() {
        let mut h = XdpFeedHandler::new();
        let msgs = [
            common::encode_symbol_index_mapping(&mapping(4242, "IBM", 4)),
            common::encode_source_time_reference(0, 0, 1_700_000_000),
            add(4242, 1, integrated::Side::Buy, 1_805_000, 500),
            add(4242, 2, integrated::Side::Sell, 1_805_200, 300),
        ];
        let refs: Vec<&[u8]> = msgs.iter().map(|m| m.as_slice()).collect();
        let pkt = packet::encode_packet(packet::delivery_flag::ORIGINAL, 1, 0, 0, &refs);
        h.on_packet(&pkt, 0).unwrap();

        let book = h.books().get(4242).unwrap();
        assert_eq!(book.symbol(), "IBM");
        assert_eq!(book.best_bid().unwrap().0.to_string(), "180.50");
        assert_eq!(book.best_ask().unwrap().0.to_string(), "180.52");
        assert_eq!(h.stats().book_errors, 0);
        assert_eq!(book.last_update_ts(), 1_700_000_000_000_000_001);
    }

    #[test]
    fn messages_arriving_before_their_symbol_mapping_are_counted_not_guessed() {
        let mut h = XdpFeedHandler::new();
        let pkt = packet::encode_packet(
            packet::delivery_flag::ORIGINAL,
            1,
            0,
            0,
            &[&add(4242, 1, integrated::Side::Buy, 1_805_000, 500)],
        );
        h.on_packet(&pkt, 0).unwrap();
        assert_eq!(h.stats().awaiting_symbol_mapping, 1);
        assert_eq!(h.stats().book_events, 0);
        assert!(h.books().get(4242).is_none());
    }

    #[test]
    fn skip_discards_already_applied_messages_from_an_overlapping_packet() {
        let mut h = XdpFeedHandler::new();
        h.seed_directory([mapping(4242, "IBM", 4)]);
        let a = add(4242, 1, integrated::Side::Buy, 1_805_000, 100);
        let b = add(4242, 2, integrated::Side::Buy, 1_805_100, 200);
        let pkt = packet::encode_packet(packet::delivery_flag::ORIGINAL, 1, 0, 0, &[&a, &b]);
        h.on_packet(&pkt, 1).unwrap();
        assert!(h.books().get(4242).unwrap().order(1).is_none());
        assert!(h.books().get(4242).unwrap().order(2).is_some());
    }

    #[test]
    fn symbol_clear_wipes_the_book_before_a_refresh() {
        let mut h = XdpFeedHandler::new();
        h.seed_directory([mapping(4242, "IBM", 4)]);
        let pkt = packet::encode_packet(
            packet::delivery_flag::ORIGINAL,
            1,
            0,
            0,
            &[&add(4242, 1, integrated::Side::Buy, 1_805_000, 500)],
        );
        h.on_packet(&pkt, 0).unwrap();
        assert_eq!(h.books().get(4242).unwrap().order_count(), 1);

        let clear = common::encode_symbol_clear(&common::SymbolClear {
            source_time_secs: 1_700_000_000,
            source_time_nanos: 0,
            symbol_index: 4242,
            next_source_seq_num: 1,
        });
        let pkt = packet::encode_packet(packet::delivery_flag::ORIGINAL, 2, 0, 0, &[&clear]);
        h.on_packet(&pkt, 0).unwrap();
        assert_eq!(h.books().get(4242).unwrap().order_count(), 0);
    }

    #[test]
    fn a_refresh_sequence_is_tracked_from_start_to_end() {
        let mut h = XdpFeedHandler::new();
        h.seed_directory([mapping(4242, "IBM", 4)]);

        let header = common::encode_refresh_header(&common::RefreshHeader {
            source_time_secs: 1_700_000_000,
            source_time_nanos: 0,
            last_seq_num: 500_000,
            symbol_index: 4242,
        });
        let start =
            packet::encode_packet(packet::delivery_flag::REFRESH_START, 1, 0, 0, &[&header]);
        h.on_packet(&start, 0).unwrap();
        assert!(h.in_refresh());
        assert_eq!(h.refresh_resume_sequence(), Some(500_000));

        let refresh_order = integrated::AddOrderRefresh {
            source_time_secs: 1_700_000_000,
            source_time_nanos: 0,
            symbol_index: 4242,
            symbol_seq_num: 1,
            order_id: 77,
            price_raw: 1_805_000,
            volume: 900,
            side: integrated::Side::Buy,
            firm_id: exchange_core::FirmId5::NUL,
        }
        .encode();
        let end = packet::encode_packet(
            packet::delivery_flag::REFRESH_END,
            2,
            0,
            0,
            &[&refresh_order],
        );
        h.on_packet(&end, 0).unwrap();
        assert!(!h.in_refresh());
        assert_eq!(h.books().get(4242).unwrap().best_bid().unwrap().1, 900);
    }

    #[test]
    fn a_halt_updates_trading_state_but_an_ssr_change_does_not() {
        let mut h = XdpFeedHandler::new();
        h.seed_directory([mapping(4242, "IBM", 4)]);
        h.on_packet(
            &packet::encode_packet(
                packet::delivery_flag::ORIGINAL,
                1,
                0,
                0,
                &[&add(4242, 1, integrated::Side::Buy, 1_805_000, 100)],
            ),
            0,
        )
        .unwrap();

        let status = common::SecurityStatus {
            source_time_secs: 1_700_000_000,
            source_time_nanos: 0,
            symbol_index: 4242,
            symbol_seq_num: 1,
            status: SecurityStatusCode::TradingHalt,
            halt_condition: common::HaltCondition::LuldPause,
            price1_raw: 0,
            price2_raw: 0,
            ssr_triggering_exchange_id: ' ',
            ssr_triggering_volume: 0,
            time: 0,
            ssr_state: '~',
            market_state: 'O',
        };
        let bytes = common::encode_security_status(&status, '4', 'M');
        h.on_packet(
            &packet::encode_packet(packet::delivery_flag::ORIGINAL, 2, 0, 0, &[&bytes]),
            0,
        )
        .unwrap();
        assert_eq!(
            h.books().get(4242).unwrap().trading_state(),
            TradingState::Paused
        );

        // An SSR activation must not silently un-pause the symbol.
        let ssr = common::encode_security_status(&status, 'A', '~');
        h.on_packet(
            &packet::encode_packet(packet::delivery_flag::ORIGINAL, 3, 0, 0, &[&ssr]),
            0,
        )
        .unwrap();
        assert_eq!(
            h.books().get(4242).unwrap().trading_state(),
            TradingState::Paused
        );
    }

    #[test]
    fn execution_and_replace_maintain_the_book() {
        let mut h = XdpFeedHandler::new();
        h.seed_directory([mapping(4242, "IBM", 4)]);
        let msgs = [
            add(4242, 1, integrated::Side::Sell, 1_805_000, 500),
            integrated::OrderExecution {
                source_time_nanos: 2,
                symbol_index: 4242,
                symbol_seq_num: 2,
                order_id: 1,
                trade_id: 9,
                price_raw: 1_805_000,
                volume: 200,
                printable_flag: 1,
                conditions: Default::default(),
            }
            .encode(),
            integrated::ReplaceOrder {
                source_time_nanos: 3,
                symbol_index: 4242,
                symbol_seq_num: 3,
                order_id: 1,
                new_order_id: 2,
                price_raw: 1_805_500,
                volume: 400,
            }
            .encode(),
        ];
        let refs: Vec<&[u8]> = msgs.iter().map(|m| m.as_slice()).collect();
        h.on_packet(
            &packet::encode_packet(packet::delivery_flag::ORIGINAL, 1, 0, 0, &refs),
            0,
        )
        .unwrap();

        let book = h.books().get(4242).unwrap();
        assert!(book.order(1).is_none());
        let replaced = book.order(2).unwrap();
        assert_eq!(
            replaced.side,
            CoreSide::Sell,
            "replace carries the side over"
        );
        assert_eq!(replaced.qty, 400);
        assert_eq!(book.stats().printed_volume, 200);
    }

    #[test]
    fn a_watch_list_filters_by_symbol_index() {
        let mut h = XdpFeedHandler::new();
        h.seed_directory([mapping(1, "AAA", 4), mapping(2, "BBB", 4)]);
        h.watch_indexes([1u32]);
        let a = add(1, 1, integrated::Side::Buy, 100, 10);
        let b = add(2, 2, integrated::Side::Buy, 100, 10);
        h.on_packet(
            &packet::encode_packet(packet::delivery_flag::ORIGINAL, 1, 0, 0, &[&a, &b]),
            0,
        )
        .unwrap();
        assert_eq!(h.stats().filtered_out, 1);
        assert!(h.books().get(1).is_some());
        assert!(h.books().get(2).is_none());
    }

    #[test]
    fn non_printable_executions_do_not_double_count_auction_volume() {
        let mut h = XdpFeedHandler::new();
        h.seed_directory([mapping(4242, "IBM", 4)]);
        let msgs = [
            add(4242, 1, integrated::Side::Sell, 1_805_000, 1_000),
            integrated::OrderExecution {
                source_time_nanos: 2,
                symbol_index: 4242,
                symbol_seq_num: 2,
                order_id: 1,
                trade_id: 9,
                price_raw: 1_805_000,
                volume: 1_000,
                printable_flag: 0, // auction execution
                conditions: integrated::TradeConditions {
                    settlement: '@',
                    exemption: 'O',
                    extended_hours: ' ',
                    sro_detail: ' ',
                },
            }
            .encode(),
            integrated::CrossTrade {
                source_time_nanos: 3,
                symbol_index: 4242,
                symbol_seq_num: 3,
                cross_id: 1,
                price_raw: 1_805_000,
                volume: 1_000,
                cross_type: 'O',
            }
            .encode(),
        ];
        let refs: Vec<&[u8]> = msgs.iter().map(|m| m.as_slice()).collect();
        h.on_packet(
            &packet::encode_packet(packet::delivery_flag::ORIGINAL, 1, 0, 0, &refs),
            0,
        )
        .unwrap();
        assert_eq!(h.books().get(4242).unwrap().stats().printed_volume, 1_000);
    }
}
