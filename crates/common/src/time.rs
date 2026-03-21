//! Nanosecond-precision timestamps and swappable clock abstraction.
//! Inspired by NautilusTrader's deterministic time model.

use std::fmt;
use std::ops::{Add, Sub};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// ─── UnixNanos ───────────────────────────────────────────────────────────────

/// A nanosecond-precision UTC timestamp stored as u64.
/// Max representable: ~2554 AD. Zero = unset/sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct UnixNanos(u64);

impl UnixNanos {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub const fn new(ns: u64) -> Self {
        Self(ns)
    }

    /// Capture the current wall-clock time.
    #[inline]
    pub fn now() -> Self {
        let dur = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX epoch");
        Self(dur.as_nanos() as u64)
    }

    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000_000
    }

    #[inline]
    pub const fn as_micros(self) -> u64 {
        self.0 / 1_000
    }

    #[inline]
    pub const fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1_000_000_000.0
    }

    #[inline]
    pub const fn from_millis(ms: u64) -> Self {
        Self(ms * 1_000_000)
    }

    #[inline]
    pub const fn from_micros(us: u64) -> Self {
        Self(us * 1_000)
    }

    #[inline]
    pub const fn from_secs(s: u64) -> Self {
        Self(s * 1_000_000_000)
    }

    /// Duration between two timestamps in nanoseconds.
    #[inline]
    pub const fn delta(self, other: Self) -> i64 {
        self.0 as i64 - other.0 as i64
    }

    #[inline]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Saturating addition — won't panic on overflow.
    #[inline]
    pub const fn saturating_add(self, ns: u64) -> Self {
        Self(self.0.saturating_add(ns))
    }
}

impl Add<u64> for UnixNanos {
    type Output = Self;
    #[inline]
    fn add(self, rhs: u64) -> Self {
        Self(self.0 + rhs)
    }
}

impl Sub for UnixNanos {
    type Output = u64;
    #[inline]
    fn sub(self, rhs: Self) -> u64 {
        self.0.saturating_sub(rhs.0)
    }
}

impl fmt::Display for UnixNanos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.0 / 1_000_000_000;
        let nanos = self.0 % 1_000_000_000;
        write!(f, "{secs}.{nanos:09}")
    }
}

impl From<u64> for UnixNanos {
    #[inline]
    fn from(ns: u64) -> Self {
        Self(ns)
    }
}

impl From<UnixNanos> for u64 {
    #[inline]
    fn from(ts: UnixNanos) -> Self {
        ts.0
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for UnixNanos {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for UnixNanos {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u64::deserialize(deserializer).map(Self)
    }
}

// ─── SequenceId ──────────────────────────────────────────────────────────────

/// Monotonically increasing event sequence number.
/// Guarantees total ordering even when timestamps collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SequenceId(u64);

impl SequenceId {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SequenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "seq#{}", self.0)
    }
}

/// Thread-safe atomic sequence generator.
pub struct SequenceGenerator {
    next: AtomicU64,
}

impl SequenceGenerator {
    pub const fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    /// Generate the next unique sequence ID.
    #[inline]
    pub fn next_id(&self) -> SequenceId {
        SequenceId(self.next.fetch_add(1, Ordering::Relaxed))
    }

    /// Peek at the next ID without advancing.
    #[inline]
    pub fn peek(&self) -> SequenceId {
        SequenceId(self.next.load(Ordering::Relaxed))
    }

    /// Reset for deterministic backtesting.
    pub fn reset(&self) {
        self.next.store(1, Ordering::Release);
    }
}

impl Default for SequenceGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Clock Trait ─────────────────────────────────────────────────────────────

/// Swappable clock: real-time for live trading, manual for backtesting.
/// This is the key pattern from NautilusTrader that enables deterministic replay.
pub trait Clock: Send + Sync {
    /// Current timestamp.
    fn now(&self) -> UnixNanos;

    /// Human-readable name for logging.
    fn clock_type(&self) -> &'static str;
}

/// Wall-clock time for live trading. Wraps `SystemTime`.
#[derive(Debug, Clone, Copy)]
pub struct RealtimeClock;

impl Clock for RealtimeClock {
    #[inline]
    fn now(&self) -> UnixNanos {
        UnixNanos::now()
    }

    fn clock_type(&self) -> &'static str {
        "RealtimeClock"
    }
}

/// Manually-advanced clock for deterministic backtesting.
/// The backtest data iterator calls `set()` before processing each event.
#[derive(Debug)]
pub struct DeterministicClock {
    current: AtomicU64,
}

impl DeterministicClock {
    pub const fn new() -> Self {
        Self {
            current: AtomicU64::new(0),
        }
    }

    /// Advance the clock. Called by the backtest event iterator.
    /// Panics in debug if time moves backward (causality violation).
    pub fn set(&self, ts: UnixNanos) {
        #[cfg(debug_assertions)]
        {
            let prev = self.current.load(Ordering::Acquire);
            debug_assert!(
                ts.as_u64() >= prev,
                "DeterministicClock moved backward: {prev} -> {}",
                ts.as_u64()
            );
        }
        self.current.store(ts.as_u64(), Ordering::Release);
    }

    /// Reset for a new backtest run.
    pub fn reset(&self) {
        self.current.store(0, Ordering::Release);
    }
}

impl Clock for DeterministicClock {
    #[inline]
    fn now(&self) -> UnixNanos {
        UnixNanos::new(self.current.load(Ordering::Acquire))
    }

    fn clock_type(&self) -> &'static str {
        "DeterministicClock"
    }
}
