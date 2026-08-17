//! Exact fixed-point prices.
//!
//! Every venue publishes prices as scaled integers with a different implied scale:
//!
//! | Wire format                          | Type  | Implied decimals |
//! |--------------------------------------|-------|------------------|
//! | ITCH 5.0 `Price(4)`                  | `u32` | 4                |
//! | ITCH 5.0 `Price(8)` (MWCB levels)    | `u64` | 8                |
//! | OUCH 4.2 `Price`                     | `u32` | 4                |
//! | NYSE XDP price field                 | `i32` | per-symbol `PriceScaleCode` (3, 4 or 6) |
//! | NYSE Pillar Binary Gateway `Price`   | `u64` | 8                |
//!
//! Rather than convert to `f64` at the decoder (which silently loses cents on large
//! notionals and makes tick arithmetic non-associative), everything is normalised to a
//! single signed integer scale of 1e-9 USD. That holds the widest venue maximum
//! ($999,999.999999 on Pillar) with more than three orders of magnitude of headroom in an
//! `i64`, and the conversion from every wire scale above is an exact integer multiply.

use std::fmt;

/// Number of decimal places in the canonical representation.
pub const PRICE_SCALE_DECIMALS: u32 = 9;
/// Canonical units per whole dollar.
pub const PRICE_ONE: i64 = 1_000_000_000;

/// A price in units of 1e-9 USD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Price(i64);

impl Price {
    pub const ZERO: Self = Self(0);

    /// The sentinel Nasdaq OUCH 4.2 uses for "market order in a cross": $214,748.3647,
    /// i.e. `0x7FFFFFFF` at 4 implied decimals.
    pub const OUCH_MARKET_SENTINEL_RAW: u32 = 0x7FFF_FFFF;
    /// ITCH `Price(4)` maximum: $200,000.0000 (`0x77359400`).
    pub const ITCH_PRICE4_MAX_RAW: u32 = 0x7735_9400;

    #[inline]
    pub const fn from_raw(units: i64) -> Self {
        Self(units)
    }

    #[inline]
    pub const fn raw(self) -> i64 {
        self.0
    }

    #[inline]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Build from an integer with `decimals` implied decimal places.
    ///
    /// Saturates rather than wrapping: a corrupt field must not silently alias onto a
    /// plausible price.
    #[inline]
    pub const fn from_scaled(value: i64, decimals: u32) -> Self {
        if decimals >= PRICE_SCALE_DECIMALS {
            let div = pow10(decimals - PRICE_SCALE_DECIMALS);
            Self(value / div)
        } else {
            let mul = pow10(PRICE_SCALE_DECIMALS - decimals);
            match value.checked_mul(mul) {
                Some(v) => Self(v),
                None if value < 0 => Self(i64::MIN),
                None => Self(i64::MAX),
            }
        }
    }

    /// ITCH 5.0 / OUCH 4.2 `Price(4)`.
    #[inline]
    pub const fn from_price4(raw: u32) -> Self {
        Self::from_scaled(raw as i64, 4)
    }

    /// ITCH 5.0 `Price(8)` (market-wide circuit-breaker decline levels).
    #[inline]
    pub const fn from_price8(raw: u64) -> Self {
        Self::from_scaled(raw as i64, 8)
    }

    /// NYSE Pillar Binary Gateway `Price` (unsigned little-endian, price scale 8).
    #[inline]
    pub const fn from_pillar(raw: u64) -> Self {
        Self::from_scaled(raw as i64, 8)
    }

    /// NYSE XDP price: numerator plus the symbol's `PriceScaleCode`.
    ///
    /// From the Pillar Common Client Specification: `price = numerator / 10^scale_code`.
    #[inline]
    pub const fn from_xdp(numerator: i32, scale_code: u8) -> Self {
        Self::from_scaled(numerator as i64, scale_code as u32)
    }

    /// Encode back to a `Price(4)` wire field (ITCH/OUCH). Truncates toward zero.
    #[inline]
    pub const fn to_price4(self) -> u32 {
        let v = self.0 / pow10(PRICE_SCALE_DECIMALS - 4);
        if v < 0 {
            0
        } else if v > u32::MAX as i64 {
            u32::MAX
        } else {
            v as u32
        }
    }

    /// Encode back to a Pillar Binary Gateway `Price` field (scale 8).
    #[inline]
    pub const fn to_pillar(self) -> u64 {
        let v = self.0 / 10;
        if v < 0 {
            0
        } else {
            v as u64
        }
    }

    /// Encode back to an XDP numerator for a symbol with the given scale code.
    #[inline]
    pub const fn to_xdp(self, scale_code: u8) -> i32 {
        let v = self.0 / pow10(PRICE_SCALE_DECIMALS - scale_code as u32);
        if v > i32::MAX as i64 {
            i32::MAX
        } else if v < i32::MIN as i64 {
            i32::MIN
        } else {
            v as i32
        }
    }

    /// Lossy conversion for the parts of the system that speak `f64`.
    #[inline]
    pub fn as_f64(self) -> f64 {
        self.0 as f64 / PRICE_ONE as f64
    }

    /// Build from `f64` (order entry from strategy code). Rounds half away from zero.
    #[inline]
    pub fn from_f64(v: f64) -> Self {
        if !v.is_finite() {
            return Self::ZERO;
        }
        let scaled = v * PRICE_ONE as f64;
        Self(scaled.round() as i64)
    }

    /// Whole number of ticks of size `tick`, truncating toward zero. Returns `None` for a
    /// non-positive tick so callers cannot divide by zero.
    #[inline]
    pub fn ticks(self, tick: Price) -> Option<i64> {
        if tick.0 <= 0 {
            None
        } else {
            Some(self.0 / tick.0)
        }
    }

    /// Midpoint, rounded toward negative infinity so it is deterministic across calls.
    #[inline]
    pub fn midpoint(bid: Price, ask: Price) -> Price {
        Price(bid.0.saturating_add(ask.0).div_euclid(2))
    }
}

/// `10^n` as `i64`, saturating at the `i64` range. `const fn` so scale conversions fold at
/// compile time when the decimals are literals.
const fn pow10(n: u32) -> i64 {
    let mut out: i64 = 1;
    let mut i = 0;
    while i < n && out <= i64::MAX / 10 {
        out *= 10;
        i += 1;
    }
    out
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let neg = self.0 < 0;
        let abs = self.0.unsigned_abs();
        let whole = abs / PRICE_ONE as u64;
        let frac = abs % PRICE_ONE as u64;
        // Trim trailing zeros but always show at least two decimals (US equity convention).
        let mut frac_str = format!("{:09}", frac);
        while frac_str.len() > 2 && frac_str.ends_with('0') {
            frac_str.pop();
        }
        if neg {
            f.write_str("-")?;
        }
        write!(f, "{}.{}", whole, frac_str)
    }
}

impl std::ops::Add for Price {
    type Output = Price;
    #[inline]
    fn add(self, rhs: Price) -> Price {
        Price(self.0.saturating_add(rhs.0))
    }
}

impl std::ops::Sub for Price {
    type Output = Price;
    #[inline]
    fn sub(self, rhs: Price) -> Price {
        Price(self.0.saturating_sub(rhs.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itch_price4_decodes_exactly() {
        // $123.4500 on the wire is 1_234_500 at 4 implied decimals.
        let p = Price::from_price4(1_234_500);
        assert_eq!(p.raw(), 123_450_000_000);
        assert_eq!(p.to_string(), "123.45");
        assert_eq!(p.to_price4(), 1_234_500);
    }

    #[test]
    fn itch_price4_max_is_two_hundred_thousand() {
        let p = Price::from_price4(Price::ITCH_PRICE4_MAX_RAW);
        assert_eq!(p.as_f64(), 200_000.0);
    }

    #[test]
    fn ouch_market_sentinel_is_the_documented_dollar_value() {
        let p = Price::from_price4(Price::OUCH_MARKET_SENTINEL_RAW);
        assert_eq!(p.to_string(), "214748.3647");
    }

    #[test]
    fn xdp_scale_codes_all_round_trip() {
        // Pillar Common Client Spec §4.5: price = numerator / 10^PriceScaleCode.
        for (num, code, expect) in [
            (12_345_600_i32, 6, "12.3456"),
            (1_234_500_i32, 4, "123.45"),
            (123_456_i32, 3, "123.456"),
        ] {
            let p = Price::from_xdp(num, code);
            assert_eq!(p.to_string(), expect, "scale code {code}");
            assert_eq!(p.to_xdp(code), num, "scale code {code} round trip");
        }
    }

    #[test]
    fn pillar_price_scale_eight_matches_spec_example() {
        // Pillar Binary Gateway data types: "123000000 = $1.23".
        assert_eq!(Price::from_pillar(123_000_000).to_string(), "1.23");
        assert_eq!(Price::from_pillar(123_000_000).to_pillar(), 123_000_000);
    }

    #[test]
    fn pillar_max_price_fits_without_saturating() {
        // 999,999.999999 is the documented Pillar maximum.
        let p = Price::from_pillar(99_999_999_999_900);
        assert_eq!(p.to_string(), "999999.999999");
        assert!(p.raw() < i64::MAX / 1000);
    }

    #[test]
    fn display_keeps_two_decimals_minimum() {
        assert_eq!(Price::from_price4(1_000_000).to_string(), "100.00");
    }

    #[test]
    fn midpoint_is_deterministic_on_odd_spreads() {
        let bid = Price::from_price4(1_000_100); // 100.01
        let ask = Price::from_price4(1_000_200); // 100.02
        assert_eq!(Price::midpoint(bid, ask).to_string(), "100.015");
    }

    #[test]
    fn ticks_rejects_a_zero_tick_instead_of_dividing() {
        assert_eq!(Price::from_price4(1_000_000).ticks(Price::ZERO), None);
        let penny = Price::from_scaled(1, 2);
        assert_eq!(Price::from_price4(1_000_000).ticks(penny), Some(10_000));
    }
}
