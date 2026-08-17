//! Translation from the venue-neutral book event stream into RustForge's `MarketEvent`.
//!
//! The direct feeds carry far more than the rest of the system consumes: order-by-order
//! depth, auction imbalances, halt reasons, retail interest indications. This module is the
//! narrowing point, and it is deliberately conservative about what it emits:
//!
//! * a **Trade** only for prints that count toward consolidated volume, so a non-printable
//!   auction execution never inflates the tape twice;
//! * a **Quote** only when the top of book actually changed, because emitting one per book
//!   message would multiply the downstream event rate by an order of magnitude for no
//!   information;
//! * a **BookUpdate** only when depth was subscribed.
//!
//! Nothing is invented. If a book has no bid, the quote carries no bid rather than a zero,
//! and a symbol that has not yet been resolved from a directory produces no event at all.

use common::events::{
    BookUpdateEvent, Envelope, MarketEvent, PriceLevel, QuoteEvent, TradeEvent, TradeSide,
};
use common::time::{SequenceId, UnixNanos};
use compact_str::CompactString;
use exchange_core::book::{Bbo, OrderBook};
use exchange_core::feed::BookEvent;
use exchange_core::Price;
use ingestion::source::DataType;

/// Decides which `MarketEvent`s a book event produces, given what was subscribed.
#[derive(Debug, Clone)]
pub struct Normalizer {
    want_trades: bool,
    want_quotes: bool,
    want_depth: bool,
    depth_levels: usize,
    /// Last emitted top of book per instrument, so an unchanged BBO is not republished.
    last_bbo: std::collections::HashMap<u32, Bbo>,
    sequence: u64,
}

impl Normalizer {
    pub fn new(data_types: &[DataType], depth_levels: usize) -> Self {
        Self {
            want_trades: data_types.contains(&DataType::Trades),
            want_quotes: data_types
                .iter()
                .any(|d| matches!(d, DataType::Quotes | DataType::OrderBookL1)),
            want_depth: data_types.contains(&DataType::OrderBookL2),
            depth_levels: depth_levels.max(1),
            last_bbo: std::collections::HashMap::new(),
            sequence: 0,
        }
    }

    fn next_sequence(&mut self) -> SequenceId {
        self.sequence += 1;
        SequenceId::new(self.sequence)
    }

    /// Produce the events for one applied book event.
    ///
    /// `book` must already reflect `event`; the quote and depth outputs are read from it.
    pub fn on_event(
        &mut self,
        event: &BookEvent<'_>,
        book: &OrderBook,
        symbol_override: Option<&str>,
    ) -> Vec<Envelope<MarketEvent>> {
        let symbol = symbol_override.unwrap_or_else(|| book.symbol());
        if symbol.is_empty() {
            // The instrument has not been resolved to a ticker yet. Emitting an event keyed
            // by a numeric index would be worse than emitting nothing.
            return Vec::new();
        }

        let ts_event = UnixNanos::new(event.ts());
        let ts_init = UnixNanos::now();
        let mut out = Vec::with_capacity(2);

        if self.want_trades {
            if let Some(trade) = trade_from(event, book) {
                let seq = self.next_sequence();
                out.push(Envelope::new(
                    ts_event,
                    ts_init,
                    seq,
                    MarketEvent::Trade(TradeEvent {
                        symbol: CompactString::new(symbol),
                        ..trade
                    }),
                ));
            }
        }

        if (self.want_quotes || self.want_depth) && event.mutates_book() {
            let bbo = book.bbo();
            let changed = self.last_bbo.get(&book.key()) != Some(&bbo);
            if changed {
                self.last_bbo.insert(book.key(), bbo);

                if self.want_quotes {
                    let seq = self.next_sequence();
                    out.push(Envelope::new(
                        ts_event,
                        ts_init,
                        seq,
                        MarketEvent::Quote(QuoteEvent {
                            symbol: CompactString::new(symbol),
                            bid: price_or_nan(bbo.bid.map(|(p, _)| p)),
                            bid_size: bbo.bid.map_or(0.0, |(_, q)| q as f64),
                            ask: price_or_nan(bbo.ask.map(|(p, _)| p)),
                            ask_size: bbo.ask.map_or(0.0, |(_, q)| q as f64),
                        }),
                    ));
                }

                if self.want_depth {
                    let seq = self.next_sequence();
                    out.push(Envelope::new(
                        ts_event,
                        ts_init,
                        seq,
                        MarketEvent::BookUpdate(BookUpdateEvent {
                            symbol: CompactString::new(symbol),
                            bids: levels(book, exchange_core::feed::Side::Buy, self.depth_levels),
                            asks: levels(book, exchange_core::feed::Side::Sell, self.depth_levels),
                        }),
                    ));
                }
            }
        }

        out
    }
}

/// An absent side is `NaN`, not zero: a zero bid is a price, and downstream arithmetic on it
/// silently produces nonsense, whereas `NaN` propagates visibly.
fn price_or_nan(p: Option<Price>) -> f64 {
    p.map_or(f64::NAN, |p| p.as_f64())
}

fn levels(book: &OrderBook, side: exchange_core::feed::Side, n: usize) -> Vec<PriceLevel> {
    book.depth(side, n)
        .into_iter()
        .map(|(price, level)| PriceLevel {
            price: price.as_f64(),
            quantity: level.qty as f64,
        })
        .collect()
}

/// Extract a tape print from a book event, if it is one that should print.
fn trade_from(event: &BookEvent<'_>, book: &OrderBook) -> Option<TradeEvent> {
    match *event {
        BookEvent::Execute {
            qty,
            price,
            condition,
            order_id,
            ..
        } => {
            if !condition.counts_toward_volume() {
                return None;
            }
            // An execution at the display price does not restate it, so recover the price
            // from the resting order. After a full fill the order is gone, and the book's
            // last print — which this same event just set — is the correct fallback.
            let px = price
                .or_else(|| book.order(order_id).map(|o| o.price))
                .or_else(|| book.stats().last_trade_price)?;
            // The aggressor took the resting side's liquidity, so the resting side's
            // opposite is the trade's initiator.
            let side = book
                .order(order_id)
                .map(|o| match o.side {
                    exchange_core::feed::Side::Buy => TradeSide::Sell,
                    exchange_core::feed::Side::Sell => TradeSide::Buy,
                })
                .unwrap_or(TradeSide::Unknown);
            Some(TradeEvent {
                symbol: CompactString::default(),
                price: px.as_f64(),
                quantity: qty as f64,
                side,
            })
        }

        BookEvent::Trade {
            price,
            qty,
            condition,
            side,
            ..
        } => {
            if !condition.counts_toward_volume() {
                return None;
            }
            Some(TradeEvent {
                symbol: CompactString::default(),
                price: price.as_f64(),
                quantity: qty as f64,
                side: match side {
                    Some(exchange_core::feed::Side::Buy) => TradeSide::Buy,
                    Some(exchange_core::feed::Side::Sell) => TradeSide::Sell,
                    None => TradeSide::Unknown,
                },
            })
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange_core::feed::{Side, TradeCondition};

    fn book_with(bid: Option<(i64, u64)>, ask: Option<(i64, u64)>) -> OrderBook {
        let mut b = OrderBook::new(1, "AAPL");
        let mut id = 1;
        for (side, level) in [(Side::Buy, bid), (Side::Sell, ask)] {
            if let Some((px, qty)) = level {
                b.apply(&BookEvent::Add {
                    key: 1,
                    symbol: "AAPL",
                    ts: 1,
                    order_id: id,
                    side,
                    price: Price::from_price4(px as u32),
                    qty,
                    participant: None,
                })
                .unwrap();
                id += 1;
            }
        }
        b
    }

    fn add_event(id: u64, side: Side, px: u32, qty: u64) -> BookEvent<'static> {
        BookEvent::Add {
            key: 1,
            symbol: "AAPL",
            ts: 100,
            order_id: id,
            side,
            price: Price::from_price4(px),
            qty,
            participant: None,
        }
    }

    #[test]
    fn a_quote_is_emitted_only_when_the_top_of_book_changes() {
        let mut n = Normalizer::new(&[DataType::Quotes], 5);
        let mut book = OrderBook::new(1, "AAPL");

        let e = add_event(1, Side::Buy, 1_000_000, 100);
        book.apply(&e).unwrap();
        assert_eq!(n.on_event(&e, &book, None).len(), 1, "first quote");

        // A second order behind the touch does not change the BBO.
        let e = add_event(2, Side::Buy, 999_900, 100);
        book.apply(&e).unwrap();
        assert!(n.on_event(&e, &book, None).is_empty(), "no BBO change");

        // Joining the touch changes the displayed size, which is a BBO change.
        let e = add_event(3, Side::Buy, 1_000_000, 200);
        book.apply(&e).unwrap();
        assert_eq!(n.on_event(&e, &book, None).len(), 1, "size changed");
    }

    #[test]
    fn an_absent_side_is_nan_rather_than_zero() {
        let mut n = Normalizer::new(&[DataType::Quotes], 5);
        let book = book_with(Some((1_000_000, 100)), None);
        let e = add_event(1, Side::Buy, 1_000_000, 100);
        let out = n.on_event(&e, &book, None);
        let MarketEvent::Quote(q) = &out[0].payload else {
            panic!("expected a quote")
        };
        assert_eq!(q.bid, 100.0);
        assert!(q.ask.is_nan(), "an empty ask must not read as a price of 0");
        assert_eq!(q.ask_size, 0.0);
    }

    #[test]
    fn non_printable_executions_produce_no_trade() {
        let mut n = Normalizer::new(&[DataType::Trades], 5);
        let mut book = book_with(None, Some((1_000_000, 500)));
        let e = BookEvent::Execute {
            key: 1,
            ts: 100,
            order_id: 1,
            qty: 500,
            price: Some(Price::from_price4(1_000_000)),
            trade_id: 1,
            condition: TradeCondition::NonPrintable,
        };
        book.apply(&e).unwrap();
        assert!(n.on_event(&e, &book, None).is_empty());
    }

    #[test]
    fn an_execution_at_the_display_price_recovers_it_from_the_resting_order() {
        let mut n = Normalizer::new(&[DataType::Trades], 5);
        let mut book = book_with(None, Some((1_000_000, 500)));
        // Partial fill: the order survives, so its price is available.
        let e = BookEvent::Execute {
            key: 1,
            ts: 100,
            order_id: 1,
            qty: 200,
            price: None,
            trade_id: 1,
            condition: TradeCondition::Printable,
        };
        book.apply(&e).unwrap();
        let out = n.on_event(&e, &book, None);
        let MarketEvent::Trade(t) = &out[0].payload else {
            panic!("expected a trade")
        };
        assert_eq!(t.price, 100.0);
        assert_eq!(t.quantity, 200.0);
        assert_eq!(t.side, TradeSide::Buy, "the aggressor lifted an offer");
    }

    #[test]
    fn a_full_fill_still_prints_after_the_order_leaves_the_book() {
        let mut n = Normalizer::new(&[DataType::Trades], 5);
        let mut book = book_with(None, Some((1_000_000, 500)));
        let e = BookEvent::Execute {
            key: 1,
            ts: 100,
            order_id: 1,
            qty: 500,
            price: None,
            trade_id: 1,
            condition: TradeCondition::Printable,
        };
        book.apply(&e).unwrap();
        assert_eq!(book.order_count(), 0);
        let out = n.on_event(&e, &book, None);
        let MarketEvent::Trade(t) = &out[0].payload else {
            panic!("expected a trade")
        };
        assert_eq!(t.price, 100.0, "recovered from the book's last print");
        assert_eq!(t.side, TradeSide::Unknown, "the resting side is gone");
    }

    #[test]
    fn hidden_liquidity_prints_carry_no_side() {
        let mut n = Normalizer::new(&[DataType::Trades], 5);
        let book = book_with(None, None);
        let e = BookEvent::Trade {
            key: 1,
            symbol: "AAPL",
            ts: 100,
            price: Price::from_price4(1_000_050),
            qty: 250,
            trade_id: 7,
            side: None,
            condition: TradeCondition::Printable,
        };
        let out = n.on_event(&e, &book, None);
        let MarketEvent::Trade(t) = &out[0].payload else {
            panic!("expected a trade")
        };
        assert_eq!(t.side, TradeSide::Unknown);
        assert_eq!(t.price, 100.005);
    }

    #[test]
    fn depth_is_emitted_only_when_subscribed() {
        let mut book = OrderBook::new(1, "AAPL");
        let e = add_event(1, Side::Buy, 1_000_000, 100);
        book.apply(&e).unwrap();

        let mut quotes_only = Normalizer::new(&[DataType::Quotes], 5);
        assert_eq!(quotes_only.on_event(&e, &book, None).len(), 1);

        let mut with_depth = Normalizer::new(&[DataType::Quotes, DataType::OrderBookL2], 5);
        let out = with_depth.on_event(&e, &book, None);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[1].payload, MarketEvent::BookUpdate(_)));
    }

    #[test]
    fn depth_is_capped_at_the_requested_number_of_levels() {
        let mut n = Normalizer::new(&[DataType::OrderBookL2], 2);
        let mut book = OrderBook::new(1, "AAPL");
        let mut last = add_event(1, Side::Buy, 1_000_000, 100);
        for (i, px) in [1_000_000u32, 999_900, 999_800, 999_700].iter().enumerate() {
            last = add_event(i as u64 + 1, Side::Buy, *px, 100);
            book.apply(&last).unwrap();
        }
        let out = n.on_event(&last, &book, None);
        let MarketEvent::BookUpdate(b) = &out[0].payload else {
            panic!("expected depth")
        };
        assert_eq!(b.bids.len(), 2);
        assert_eq!(b.bids[0].price, 100.0, "best bid first");
        assert_eq!(b.bids[1].price, 99.99);
    }

    #[test]
    fn an_unresolved_symbol_produces_no_events() {
        let mut n = Normalizer::new(&[DataType::Quotes, DataType::Trades], 5);
        let mut book = OrderBook::new(1, "");
        let e = BookEvent::Add {
            key: 1,
            symbol: "",
            ts: 100,
            order_id: 1,
            side: Side::Buy,
            price: Price::from_price4(1_000_000),
            qty: 100,
            participant: None,
        };
        book.apply(&e).unwrap();
        assert!(n.on_event(&e, &book, None).is_empty());
        // Once a symbol is known, the same event does produce output.
        assert_eq!(n.on_event(&e, &book, Some("AAPL")).len(), 1);
    }

    #[test]
    fn sequence_ids_increase_monotonically_across_events() {
        let mut n = Normalizer::new(&[DataType::Quotes], 5);
        let mut book = OrderBook::new(1, "AAPL");
        let mut seqs = Vec::new();
        for (i, px) in [1_000_000u32, 1_000_100, 1_000_200].iter().enumerate() {
            let e = add_event(i as u64 + 1, Side::Buy, *px, 100);
            book.apply(&e).unwrap();
            for env in n.on_event(&e, &book, None) {
                seqs.push(env.sequence_id);
            }
        }
        assert_eq!(seqs.len(), 3);
        assert!(seqs.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn events_carry_the_exchange_timestamp_not_the_local_one() {
        let mut n = Normalizer::new(&[DataType::Quotes], 5);
        let mut book = OrderBook::new(1, "AAPL");
        let e = add_event(1, Side::Buy, 1_000_000, 100);
        book.apply(&e).unwrap();
        let out = n.on_event(&e, &book, None);
        assert_eq!(out[0].ts_event, UnixNanos::new(100));
        assert!(
            out[0].ts_init > out[0].ts_event,
            "ts_init is local wall clock"
        );
    }
}
