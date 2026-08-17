//! ITCH 5.0 encoder.
//!
//! Used for two things, neither of which is producing market data:
//!
//! 1. **Round-trip tests.** Encoding a message from typed fields and decoding it back is
//!    the only way to assert that the byte offsets in [`super::messages`] match the
//!    specification tables without a live entitlement.
//! 2. **Capture tooling.** Rewriting a recorded session (for replay, or for trimming a
//!    capture down to a handful of instruments) needs to emit well-formed ITCH.
//!
//! It is deliberately *not* wired into any feed path. Nothing in this crate ever presents
//! encoder output as exchange data.

use exchange_core::wire::Writer;
use exchange_core::Price;

use super::messages::*;

/// Write the 10-byte prefix shared by every message: type, locate, tracking, timestamp.
fn header(w: &mut Writer, msg_type: u8, h: Header) {
    w.u8(msg_type)
        .be_u16(h.stock_locate)
        .be_u16(h.tracking_number);
    // 6-byte big-endian nanoseconds-since-midnight.
    let ts = h.timestamp.to_be_bytes();
    w.raw(&ts[2..]);
}

pub fn system_event(h: Header, code: char) -> Vec<u8> {
    let mut w = Writer::with_capacity(12);
    header(&mut w, b'S', h);
    w.ascii_char(code);
    w.into_vec()
}

#[allow(clippy::too_many_arguments)]
pub fn stock_directory(
    h: Header,
    stock: &str,
    market_category: char,
    financial_status: char,
    round_lot_size: u32,
    round_lots_only: bool,
    issue_classification: char,
    issue_subtype: &str,
    authenticity: char,
    short_sale_threshold: char,
    ipo_flag: char,
    luld_tier: char,
    etp_flag: char,
    etp_leverage_factor: u32,
    inverse_etp: bool,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(39);
    header(&mut w, b'R', h);
    w.space_padded(stock, 8)
        .ascii_char(market_category)
        .ascii_char(financial_status)
        .be_u32(round_lot_size)
        .ascii_char(if round_lots_only { 'Y' } else { 'N' })
        .ascii_char(issue_classification)
        .space_padded(issue_subtype, 2)
        .ascii_char(authenticity)
        .ascii_char(short_sale_threshold)
        .ascii_char(ipo_flag)
        .ascii_char(luld_tier)
        .ascii_char(etp_flag)
        .be_u32(etp_leverage_factor)
        .ascii_char(if inverse_etp { 'Y' } else { 'N' });
    w.into_vec()
}

pub fn stock_trading_action(h: Header, stock: &str, state: char, reason: &str) -> Vec<u8> {
    let mut w = Writer::with_capacity(25);
    header(&mut w, b'H', h);
    w.space_padded(stock, 8)
        .ascii_char(state)
        .ascii_char(' ') // Reserved
        .space_padded(reason, 4);
    w.into_vec()
}

pub fn reg_sho(h: Header, stock: &str, action: char) -> Vec<u8> {
    let mut w = Writer::with_capacity(20);
    header(&mut w, b'Y', h);
    w.space_padded(stock, 8).ascii_char(action);
    w.into_vec()
}

pub fn market_participant_position(
    h: Header,
    mpid: &str,
    stock: &str,
    primary: bool,
    mode: char,
    state: char,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(26);
    header(&mut w, b'L', h);
    w.space_padded(mpid, 4)
        .space_padded(stock, 8)
        .ascii_char(if primary { 'Y' } else { 'N' })
        .ascii_char(mode)
        .ascii_char(state);
    w.into_vec()
}

pub fn mwcb_decline_level(h: Header, l1: Price, l2: Price, l3: Price) -> Vec<u8> {
    let mut w = Writer::with_capacity(35);
    header(&mut w, b'V', h);
    // Price(8): canonical scale is 1e-9, the wire scale is 1e-8.
    w.be_u64((l1.raw() / 10) as u64)
        .be_u64((l2.raw() / 10) as u64)
        .be_u64((l3.raw() / 10) as u64);
    w.into_vec()
}

pub fn mwcb_status(h: Header, level: char) -> Vec<u8> {
    let mut w = Writer::with_capacity(12);
    header(&mut w, b'W', h);
    w.ascii_char(level);
    w.into_vec()
}

pub fn ipo_quoting_period(
    h: Header,
    stock: &str,
    release_secs: u32,
    qualifier: char,
    price: Price,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(28);
    header(&mut w, b'K', h);
    w.space_padded(stock, 8)
        .be_u32(release_secs)
        .ascii_char(qualifier)
        .be_u32(price.to_price4());
    w.into_vec()
}

pub fn luld_auction_collar(
    h: Header,
    stock: &str,
    reference: Price,
    upper: Price,
    lower: Price,
    extension: u32,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(35);
    header(&mut w, b'J', h);
    w.space_padded(stock, 8)
        .be_u32(reference.to_price4())
        .be_u32(upper.to_price4())
        .be_u32(lower.to_price4())
        .be_u32(extension);
    w.into_vec()
}

pub fn operational_halt(h: Header, stock: &str, market_code: char, halted: bool) -> Vec<u8> {
    let mut w = Writer::with_capacity(21);
    header(&mut w, b'h', h);
    w.space_padded(stock, 8)
        .ascii_char(market_code)
        .ascii_char(if halted { 'H' } else { 'T' });
    w.into_vec()
}

pub fn add_order(
    h: Header,
    order_ref: u64,
    side: char,
    shares: u32,
    stock: &str,
    price: Price,
    attribution: Option<&str>,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(40);
    header(&mut w, if attribution.is_some() { b'F' } else { b'A' }, h);
    w.be_u64(order_ref)
        .ascii_char(side)
        .be_u32(shares)
        .space_padded(stock, 8)
        .be_u32(price.to_price4());
    if let Some(mpid) = attribution {
        w.space_padded(mpid, 4);
    }
    w.into_vec()
}

pub fn order_executed(h: Header, order_ref: u64, shares: u32, match_number: u64) -> Vec<u8> {
    let mut w = Writer::with_capacity(31);
    header(&mut w, b'E', h);
    w.be_u64(order_ref).be_u32(shares).be_u64(match_number);
    w.into_vec()
}

pub fn order_executed_with_price(
    h: Header,
    order_ref: u64,
    shares: u32,
    match_number: u64,
    printable: bool,
    price: Price,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(36);
    header(&mut w, b'C', h);
    w.be_u64(order_ref)
        .be_u32(shares)
        .be_u64(match_number)
        .ascii_char(if printable { 'Y' } else { 'N' })
        .be_u32(price.to_price4());
    w.into_vec()
}

pub fn order_cancel(h: Header, order_ref: u64, cancelled: u32) -> Vec<u8> {
    let mut w = Writer::with_capacity(23);
    header(&mut w, b'X', h);
    w.be_u64(order_ref).be_u32(cancelled);
    w.into_vec()
}

pub fn order_delete(h: Header, order_ref: u64) -> Vec<u8> {
    let mut w = Writer::with_capacity(19);
    header(&mut w, b'D', h);
    w.be_u64(order_ref);
    w.into_vec()
}

pub fn order_replace(
    h: Header,
    original_ref: u64,
    new_ref: u64,
    shares: u32,
    price: Price,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(35);
    header(&mut w, b'U', h);
    w.be_u64(original_ref)
        .be_u64(new_ref)
        .be_u32(shares)
        .be_u32(price.to_price4());
    w.into_vec()
}

pub fn trade_non_cross(
    h: Header,
    order_ref: u64,
    side: char,
    shares: u32,
    stock: &str,
    price: Price,
    match_number: u64,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(44);
    header(&mut w, b'P', h);
    w.be_u64(order_ref)
        .ascii_char(side)
        .be_u32(shares)
        .space_padded(stock, 8)
        .be_u32(price.to_price4())
        .be_u64(match_number);
    w.into_vec()
}

pub fn cross_trade(
    h: Header,
    shares: u64,
    stock: &str,
    price: Price,
    match_number: u64,
    cross_type: char,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(40);
    header(&mut w, b'Q', h);
    w.be_u64(shares)
        .space_padded(stock, 8)
        .be_u32(price.to_price4())
        .be_u64(match_number)
        .ascii_char(cross_type);
    w.into_vec()
}

pub fn broken_trade(h: Header, match_number: u64) -> Vec<u8> {
    let mut w = Writer::with_capacity(19);
    header(&mut w, b'B', h);
    w.be_u64(match_number);
    w.into_vec()
}

#[allow(clippy::too_many_arguments)]
pub fn noii(
    h: Header,
    paired: u64,
    imbalance: u64,
    direction: char,
    stock: &str,
    far: Price,
    near: Price,
    reference: Price,
    cross_type: char,
    variation: char,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(50);
    header(&mut w, b'I', h);
    w.be_u64(paired)
        .be_u64(imbalance)
        .ascii_char(direction)
        .space_padded(stock, 8)
        .be_u32(far.to_price4())
        .be_u32(near.to_price4())
        .be_u32(reference.to_price4())
        .ascii_char(cross_type)
        .ascii_char(variation);
    w.into_vec()
}

pub fn retail_price_improvement(h: Header, stock: &str, flag: char) -> Vec<u8> {
    let mut w = Writer::with_capacity(20);
    header(&mut w, b'N', h);
    w.space_padded(stock, 8).ascii_char(flag);
    w.into_vec()
}

#[allow(clippy::too_many_arguments)]
pub fn direct_listing_capital_raise(
    h: Header,
    stock: &str,
    eligible: bool,
    min_price: Price,
    max_price: Price,
    near_price: Price,
    near_time: u64,
    lower_collar: Price,
    upper_collar: Price,
) -> Vec<u8> {
    let mut w = Writer::with_capacity(48);
    header(&mut w, b'O', h);
    w.space_padded(stock, 8)
        .ascii_char(if eligible { 'Y' } else { 'N' })
        .be_u32(min_price.to_price4())
        .be_u32(max_price.to_price4())
        .be_u32(near_price.to_price4())
        .be_u64(near_time)
        .be_u32(lower_collar.to_price4())
        .be_u32(upper_collar.to_price4());
    w.into_vec()
}
