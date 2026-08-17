//! Level 3 (order-by-order) limit order book.
//!
//! Both direct feeds are order based, so the book is maintained by order id and the price
//! levels are a derived aggregate. That is what makes the venue-neutral event stream in
//! [`crate::feed`] work: an ITCH `Order Executed` and an XDP `Order Execution` mean exactly
//! the same thing to this structure.
//!
//! Consistency rules enforced here:
//!
//! * An event referencing an unknown order id is reported as [`BookError::UnknownOrder`]
//!   and **does not** mutate the book. On a live feed that means a dropped packet, and the
//!   only correct response is to recover state (retransmission or snapshot), never to
//!   guess.
//! * Level quantities are maintained incrementally but a level is deleted the moment its
//!   quantity reaches zero, so an empty level can never linger and quote a phantom price.

use std::collections::{BTreeMap, HashMap};

use crate::feed::{BookEvent, Side, TradingState};
use crate::price::Price;
use crate::{InstrumentKey, Nanos, OrderId};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BookError {
    #[error("event references unknown order {order_id} (book is out of sync — recover state)")]
    UnknownOrder { order_id: OrderId },
    #[error("order {order_id} has {have} shares, event removes {want}")]
    Oversubtract {
        order_id: OrderId,
        have: u64,
        want: u64,
    },
    #[error("order {order_id} already exists")]
    DuplicateOrder { order_id: OrderId },
    #[error("event for instrument {got} applied to book for {expected}")]
    WrongInstrument {
        expected: InstrumentKey,
        got: InstrumentKey,
    },
}

/// A resting displayed order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestingOrder {
    pub side: Side,
    pub price: Price,
    pub qty: u64,
}

/// Aggregated interest at one price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Level {
    pub qty: u64,
    pub orders: u32,
}

/// Top of book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bbo {
    pub bid: Option<(Price, u64)>,
    pub ask: Option<(Price, u64)>,
}

impl Bbo {
    /// Spread, or `None` if either side is empty.
    pub fn spread(&self) -> Option<Price> {
        match (self.bid, self.ask) {
            (Some((b, _)), Some((a, _))) => Some(a - b),
            _ => None,
        }
    }

    pub fn mid(&self) -> Option<Price> {
        match (self.bid, self.ask) {
            (Some((b, _)), Some((a, _))) => Some(Price::midpoint(b, a)),
            _ => None,
        }
    }

    /// True when the bid is at or above the ask — a locked or crossed book. On a healthy
    /// single-venue feed this is transient at best and usually means missed messages.
    pub fn is_crossed(&self) -> bool {
        matches!((self.bid, self.ask), (Some((b, _)), Some((a, _))) if b >= a)
    }
}

/// Running per-instrument statistics maintained alongside the book.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BookStats {
    /// Shares printed to the tape (non-printable executions excluded).
    pub printed_volume: u64,
    pub trade_count: u64,
    pub busted_trades: u64,
    pub last_trade_price: Option<Price>,
    pub last_trade_ts: Nanos,
    /// Events that could not be applied because state was missing.
    pub out_of_sync_events: u64,
}

/// One instrument's book.
#[derive(Debug, Clone)]
pub struct OrderBook {
    key: InstrumentKey,
    symbol: String,
    orders: HashMap<OrderId, RestingOrder>,
    bids: BTreeMap<i64, Level>,
    asks: BTreeMap<i64, Level>,
    state: TradingState,
    stats: BookStats,
    last_update_ts: Nanos,
}

impl OrderBook {
    pub fn new(key: InstrumentKey, symbol: impl Into<String>) -> Self {
        Self {
            key,
            symbol: symbol.into(),
            orders: HashMap::new(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            state: TradingState::NotTrading,
            stats: BookStats::default(),
            last_update_ts: 0,
        }
    }

    #[inline]
    pub fn key(&self) -> InstrumentKey {
        self.key
    }

    #[inline]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Set the ticker once it is resolved from a directory message (XDP publishes only a
    /// numeric symbol index on data messages).
    pub fn set_symbol(&mut self, symbol: impl Into<String>) {
        self.symbol = symbol.into();
    }

    #[inline]
    pub fn trading_state(&self) -> TradingState {
        self.state
    }

    #[inline]
    pub fn stats(&self) -> BookStats {
        self.stats
    }

    #[inline]
    pub fn last_update_ts(&self) -> Nanos {
        self.last_update_ts
    }

    #[inline]
    pub fn order_count(&self) -> usize {
        self.orders.len()
    }

    #[inline]
    pub fn order(&self, id: OrderId) -> Option<RestingOrder> {
        self.orders.get(&id).copied()
    }

    pub fn best_bid(&self) -> Option<(Price, u64)> {
        self.bids
            .iter()
            .next_back()
            .map(|(p, l)| (Price::from_raw(*p), l.qty))
    }

    pub fn best_ask(&self) -> Option<(Price, u64)> {
        self.asks
            .iter()
            .next()
            .map(|(p, l)| (Price::from_raw(*p), l.qty))
    }

    pub fn bbo(&self) -> Bbo {
        Bbo {
            bid: self.best_bid(),
            ask: self.best_ask(),
        }
    }

    /// Top `depth` levels of one side, best first.
    pub fn depth(&self, side: Side, depth: usize) -> Vec<(Price, Level)> {
        match side {
            Side::Buy => self
                .bids
                .iter()
                .rev()
                .take(depth)
                .map(|(p, l)| (Price::from_raw(*p), *l))
                .collect(),
            Side::Sell => self
                .asks
                .iter()
                .take(depth)
                .map(|(p, l)| (Price::from_raw(*p), *l))
                .collect(),
        }
    }

    /// Total displayed shares on one side.
    pub fn total_qty(&self, side: Side) -> u64 {
        let levels = match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        };
        levels.values().map(|l| l.qty).sum()
    }

    /// Drop every resting order. Used on `Clear` and on unrecoverable gaps.
    pub fn clear(&mut self) {
        self.orders.clear();
        self.bids.clear();
        self.asks.clear();
    }

    /// Apply one normalised feed event.
    ///
    /// Errors leave the book untouched so the caller can recover deterministically.
    pub fn apply(&mut self, ev: &BookEvent<'_>) -> Result<(), BookError> {
        if ev.key() != self.key {
            return Err(BookError::WrongInstrument {
                expected: self.key,
                got: ev.key(),
            });
        }

        let result = self.apply_inner(ev);
        if result.is_err() {
            self.stats.out_of_sync_events += 1;
        } else {
            self.last_update_ts = ev.ts();
        }
        result
    }

    fn apply_inner(&mut self, ev: &BookEvent<'_>) -> Result<(), BookError> {
        match *ev {
            BookEvent::Add {
                order_id,
                side,
                price,
                qty,
                symbol,
                ..
            } => {
                if self.orders.contains_key(&order_id) {
                    return Err(BookError::DuplicateOrder { order_id });
                }
                if self.symbol.is_empty() && !symbol.is_empty() {
                    self.symbol = symbol.to_string();
                }
                if qty > 0 {
                    self.insert_order(order_id, RestingOrder { side, price, qty });
                }
                Ok(())
            }

            BookEvent::Modify {
                order_id,
                price,
                qty,
                ..
            } => {
                let existing = *self
                    .orders
                    .get(&order_id)
                    .ok_or(BookError::UnknownOrder { order_id })?;
                self.remove_from_level(existing);
                if qty == 0 {
                    self.orders.remove(&order_id);
                } else {
                    let updated = RestingOrder {
                        side: existing.side,
                        price: price.unwrap_or(existing.price),
                        qty,
                    };
                    self.orders.insert(order_id, updated);
                    self.add_to_level(updated);
                }
                Ok(())
            }

            BookEvent::Replace {
                old_order_id,
                new_order_id,
                price,
                qty,
                ..
            } => {
                let existing = *self
                    .orders
                    .get(&old_order_id)
                    .ok_or(BookError::UnknownOrder {
                        order_id: old_order_id,
                    })?;
                self.remove_from_level(existing);
                self.orders.remove(&old_order_id);
                if qty > 0 {
                    // Side is not restated on a replace; it cannot change, so carry it over.
                    self.insert_order(
                        new_order_id,
                        RestingOrder {
                            side: existing.side,
                            price,
                            qty,
                        },
                    );
                }
                Ok(())
            }

            BookEvent::Reduce { order_id, qty, .. } => self.decrement(order_id, qty),

            BookEvent::Delete { order_id, .. } => {
                let existing = *self
                    .orders
                    .get(&order_id)
                    .ok_or(BookError::UnknownOrder { order_id })?;
                self.remove_from_level(existing);
                self.orders.remove(&order_id);
                Ok(())
            }

            BookEvent::Execute {
                order_id,
                qty,
                price,
                condition,
                ts,
                ..
            } => {
                let existing = *self
                    .orders
                    .get(&order_id)
                    .ok_or(BookError::UnknownOrder { order_id })?;
                self.decrement(order_id, qty)?;
                let print_price = price.unwrap_or(existing.price);
                self.record_print(print_price, qty, condition, ts);
                Ok(())
            }

            BookEvent::Trade {
                price,
                qty,
                condition,
                ts,
                ..
            } => {
                self.record_print(price, qty, condition, ts);
                Ok(())
            }

            BookEvent::Bust { .. } => {
                self.stats.busted_trades += 1;
                Ok(())
            }

            BookEvent::Status { state, symbol, .. } => {
                self.state = state;
                if self.symbol.is_empty() && !symbol.is_empty() {
                    self.symbol = symbol.to_string();
                }
                Ok(())
            }

            BookEvent::Imbalance { .. } => Ok(()),

            BookEvent::Clear { .. } => {
                self.clear();
                Ok(())
            }
        }
    }

    fn record_print(
        &mut self,
        price: Price,
        qty: u64,
        cond: crate::feed::TradeCondition,
        ts: Nanos,
    ) {
        self.stats.trade_count += 1;
        self.stats.last_trade_price = Some(price);
        self.stats.last_trade_ts = ts;
        if cond.counts_toward_volume() {
            self.stats.printed_volume = self.stats.printed_volume.saturating_add(qty);
        }
    }

    fn decrement(&mut self, order_id: OrderId, qty: u64) -> Result<(), BookError> {
        let existing = *self
            .orders
            .get(&order_id)
            .ok_or(BookError::UnknownOrder { order_id })?;
        if qty > existing.qty {
            return Err(BookError::Oversubtract {
                order_id,
                have: existing.qty,
                want: qty,
            });
        }
        self.remove_from_level(existing);
        let remaining = existing.qty - qty;
        if remaining == 0 {
            self.orders.remove(&order_id);
        } else {
            let updated = RestingOrder {
                qty: remaining,
                ..existing
            };
            self.orders.insert(order_id, updated);
            self.add_to_level(updated);
        }
        Ok(())
    }

    fn insert_order(&mut self, id: OrderId, order: RestingOrder) {
        self.orders.insert(id, order);
        self.add_to_level(order);
    }

    fn add_to_level(&mut self, order: RestingOrder) {
        let levels = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = levels.entry(order.price.raw()).or_default();
        level.qty += order.qty;
        level.orders += 1;
    }

    fn remove_from_level(&mut self, order: RestingOrder) {
        let levels = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        if let std::collections::btree_map::Entry::Occupied(mut e) = levels.entry(order.price.raw())
        {
            let level = e.get_mut();
            level.qty = level.qty.saturating_sub(order.qty);
            level.orders = level.orders.saturating_sub(1);
            if level.qty == 0 || level.orders == 0 {
                e.remove();
            }
        }
    }
}

/// A set of books keyed by the venue's instrument handle.
#[derive(Debug, Default, Clone)]
pub struct BookSet {
    books: HashMap<InstrumentKey, OrderBook>,
}

impl BookSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.books.len()
    }

    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
    }

    pub fn get(&self, key: InstrumentKey) -> Option<&OrderBook> {
        self.books.get(&key)
    }

    pub fn get_mut(&mut self, key: InstrumentKey) -> Option<&mut OrderBook> {
        self.books.get_mut(&key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&InstrumentKey, &OrderBook)> {
        self.books.iter()
    }

    /// Apply an event, creating the book on first sight of the instrument.
    pub fn apply(&mut self, ev: &BookEvent<'_>) -> Result<&OrderBook, BookError> {
        let key = ev.key();
        let symbol = match ev {
            BookEvent::Add { symbol, .. }
            | BookEvent::Trade { symbol, .. }
            | BookEvent::Status { symbol, .. } => *symbol,
            _ => "",
        };
        let book = self
            .books
            .entry(key)
            .or_insert_with(|| OrderBook::new(key, symbol));
        book.apply(ev)?;
        Ok(book)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::TradeCondition;

    fn add(key: u32, id: u64, side: Side, px: i64, qty: u64) -> BookEvent<'static> {
        BookEvent::Add {
            key,
            symbol: "AAPL",
            ts: 1,
            order_id: id,
            side,
            price: Price::from_price4(px as u32),
            qty,
            participant: None,
        }
    }

    #[test]
    fn add_and_delete_maintain_top_of_book() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 10, Side::Buy, 1_000_000, 100)).unwrap();
        b.apply(&add(1, 11, Side::Buy, 1_000_100, 200)).unwrap();
        b.apply(&add(1, 20, Side::Sell, 1_000_300, 300)).unwrap();

        assert_eq!(b.best_bid(), Some((Price::from_price4(1_000_100), 200)));
        assert_eq!(b.best_ask(), Some((Price::from_price4(1_000_300), 300)));

        b.apply(&BookEvent::Delete {
            key: 1,
            ts: 2,
            order_id: 11,
        })
        .unwrap();
        assert_eq!(b.best_bid(), Some((Price::from_price4(1_000_000), 100)));
    }

    #[test]
    fn levels_aggregate_multiple_orders_and_vanish_when_empty() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 1, Side::Buy, 1_000_000, 100)).unwrap();
        b.apply(&add(1, 2, Side::Buy, 1_000_000, 150)).unwrap();
        assert_eq!(
            b.depth(Side::Buy, 5)[0].1,
            Level {
                qty: 250,
                orders: 2
            }
        );

        b.apply(&BookEvent::Delete {
            key: 1,
            ts: 2,
            order_id: 1,
        })
        .unwrap();
        b.apply(&BookEvent::Delete {
            key: 1,
            ts: 3,
            order_id: 2,
        })
        .unwrap();
        assert!(b.depth(Side::Buy, 5).is_empty());
        assert_eq!(b.best_bid(), None);
    }

    #[test]
    fn partial_cancel_keeps_the_order_but_shrinks_the_level() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 1, Side::Buy, 1_000_000, 500)).unwrap();
        b.apply(&BookEvent::Reduce {
            key: 1,
            ts: 2,
            order_id: 1,
            qty: 200,
        })
        .unwrap();
        assert_eq!(b.order(1).unwrap().qty, 300);
        assert_eq!(b.best_bid().unwrap().1, 300);
    }

    #[test]
    fn execution_removes_shares_and_counts_printed_volume() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 1, Side::Sell, 1_000_000, 500)).unwrap();
        b.apply(&BookEvent::Execute {
            key: 1,
            ts: 5,
            order_id: 1,
            qty: 500,
            price: None,
            trade_id: 99,
            condition: TradeCondition::Printable,
        })
        .unwrap();
        assert_eq!(b.order_count(), 0);
        assert_eq!(b.best_ask(), None);
        assert_eq!(b.stats().printed_volume, 500);
        assert_eq!(
            b.stats().last_trade_price,
            Some(Price::from_price4(1_000_000))
        );
    }

    #[test]
    fn non_printable_execution_does_not_inflate_volume() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 1, Side::Sell, 1_000_000, 100)).unwrap();
        b.apply(&BookEvent::Execute {
            key: 1,
            ts: 5,
            order_id: 1,
            qty: 100,
            price: Some(Price::from_price4(999_900)),
            trade_id: 1,
            condition: TradeCondition::NonPrintable,
        })
        .unwrap();
        assert_eq!(b.stats().printed_volume, 0);
        assert_eq!(b.stats().trade_count, 1);
    }

    #[test]
    fn replace_carries_the_side_forward() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 1, Side::Sell, 1_000_000, 100)).unwrap();
        b.apply(&BookEvent::Replace {
            key: 1,
            ts: 3,
            old_order_id: 1,
            new_order_id: 2,
            price: Price::from_price4(1_000_500),
            qty: 400,
        })
        .unwrap();
        assert!(b.order(1).is_none());
        let new = b.order(2).unwrap();
        assert_eq!(new.side, Side::Sell);
        assert_eq!(new.qty, 400);
        assert_eq!(b.best_ask(), Some((Price::from_price4(1_000_500), 400)));
    }

    #[test]
    fn unknown_order_is_reported_and_leaves_the_book_untouched() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 1, Side::Buy, 1_000_000, 100)).unwrap();
        let before = b.bbo();
        let err = b
            .apply(&BookEvent::Delete {
                key: 1,
                ts: 2,
                order_id: 4242,
            })
            .unwrap_err();
        assert_eq!(err, BookError::UnknownOrder { order_id: 4242 });
        assert_eq!(b.bbo(), before);
        assert_eq!(b.stats().out_of_sync_events, 1);
    }

    #[test]
    fn oversubtract_is_rejected_rather_than_wrapping() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 1, Side::Buy, 1_000_000, 100)).unwrap();
        assert!(matches!(
            b.apply(&BookEvent::Reduce {
                key: 1,
                ts: 2,
                order_id: 1,
                qty: 500,
            }),
            Err(BookError::Oversubtract { .. })
        ));
        assert_eq!(b.order(1).unwrap().qty, 100);
    }

    #[test]
    fn clear_wipes_resting_state_but_keeps_statistics() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 1, Side::Buy, 1_000_000, 100)).unwrap();
        b.apply(&BookEvent::Trade {
            key: 1,
            symbol: "AAPL",
            ts: 4,
            price: Price::from_price4(1_000_000),
            qty: 50,
            trade_id: 7,
            side: None,
            condition: TradeCondition::Printable,
        })
        .unwrap();
        b.apply(&BookEvent::Clear { key: 1, ts: 5 }).unwrap();
        assert_eq!(b.order_count(), 0);
        assert_eq!(b.stats().printed_volume, 50);
    }

    #[test]
    fn crossed_book_is_detectable() {
        let mut b = OrderBook::new(1, "AAPL");
        b.apply(&add(1, 1, Side::Buy, 1_000_500, 100)).unwrap();
        b.apply(&add(1, 2, Side::Sell, 1_000_000, 100)).unwrap();
        assert!(b.bbo().is_crossed());
    }

    #[test]
    fn book_set_creates_books_on_demand() {
        let mut set = BookSet::new();
        set.apply(&add(7, 1, Side::Buy, 1_000_000, 100)).unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set.get(7).unwrap().symbol(), "AAPL");
    }
}
