#![forbid(unsafe_code)]
//! Shared primitives for direct exchange connectivity.
//!
//! Everything in this crate is protocol-agnostic: the Nasdaq (ITCH/OUCH/SoupBinTCP/
//! MoldUDP64) and NYSE (XDP/Pillar) crates both decode their own wire formats into the
//! types defined here, so a single order-book engine, gap tracker and latency recorder
//! serve every venue.
//!
//! Design constraints, in priority order:
//!   1. No `unsafe`. Every wire read is bounds-checked by [`wire::Cursor`].
//!   2. No allocation on the decode hot path. Decoders borrow from the receive buffer.
//!   3. Exact arithmetic. Prices are fixed-point integers ([`price::Price`]); floats are
//!      produced only at the boundary where the rest of the system needs `f64`.

pub mod book;
pub mod error;
pub mod feed;
pub mod fixed_str;
pub mod gap;
pub mod latency;
pub mod price;
pub mod wire;

pub use error::{WireError, WireResult};
pub use feed::{BookEvent, ImbalanceSide, Side, TradeCondition};
pub use fixed_str::{FirmId5, FixedStr, Mpid4, Symbol8, UserData8};
pub use price::Price;
pub use wire::Cursor;

/// Nanoseconds since the UNIX epoch.
pub type Nanos = u64;

/// Venue-assigned identifier for a resting order.
///
/// Nasdaq ITCH order reference numbers are `u64`; NYSE Pillar order IDs are `u64` on the
/// Integrated feed. A single width covers both.
pub type OrderId = u64;

/// Compact per-session instrument handle.
///
/// ITCH calls this the "stock locate code" (`u16`, reassigned daily); XDP calls it the
/// "symbol index" (`u32`, stable across days). Widened to `u32` so both fit.
pub type InstrumentKey = u32;
