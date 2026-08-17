//! Fixed-width ASCII fields, stored inline.
//!
//! Exchange protocols are full of short fixed-width text: an 8-byte stock symbol, a 4-byte
//! MPID, a 5-byte FirmID, a 4-byte liquidity indicator. Decoding those into `String` costs a
//! heap allocation *per message*, and on an order-by-order feed the highest-volume message
//! type is exactly the one that carries a symbol or a firm id.
//!
//! Benchmarking made the cost concrete: decoding an XDP Add Order into a struct with one
//! `String` field took 36 ns, while the sibling messages with no text field took 14 ns. The
//! allocator was two thirds of the decode.
//!
//! [`FixedStr`] stores the bytes inline, is `Copy`, and never allocates. It handles both
//! padding conventions, because the two venues disagree: Nasdaq pads alpha fields with
//! spaces, NYSE pads `zchar(n)` fields with NUL.

use std::fmt;

/// An `N`-byte ASCII field stored inline.
///
/// The stored form keeps whatever padding arrived on the wire; [`Self::as_str`] trims it. That
/// way a decoded value re-encodes to the same bytes it came from.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedStr<const N: usize> {
    bytes: [u8; N],
}

impl<const N: usize> FixedStr<N> {
    /// All spaces — the Nasdaq "field not present" form.
    pub const BLANK: Self = Self { bytes: [b' '; N] };
    /// All NULs — the NYSE `zchar` "field not present" form.
    pub const NUL: Self = Self { bytes: [0u8; N] };

    /// Copy from a wire field. Extra bytes are ignored; a short slice is space padded.
    #[inline]
    pub fn from_wire(raw: &[u8]) -> Self {
        let mut bytes = [b' '; N];
        let take = raw.len().min(N);
        bytes[..take].copy_from_slice(&raw[..take]);
        Self { bytes }
    }

    /// Build from a string, space padded on the right. Longer input is truncated.
    #[inline]
    pub fn new(s: &str) -> Self {
        Self::from_wire(s.as_bytes())
    }

    /// Build from a string, rejecting anything that would not survive the field intact.
    ///
    /// [`Self::new`] truncates, which is right for decoding (the wire field is `N` bytes by
    /// definition) and wrong for order entry, where a truncated symbol is a different
    /// instrument. Use this wherever a caller-supplied string reaches the wire.
    pub fn try_new(s: &str) -> Result<Self, String> {
        if s.len() > N {
            return Err(format!("{s:?} is {} bytes; the field is {N}", s.len()));
        }
        Ok(Self::from_wire(s.as_bytes()))
    }

    /// Build from a string, NUL padded on the right (NYSE `zchar(n)`).
    #[inline]
    pub fn new_nul_padded(s: &str) -> Self {
        let mut bytes = [0u8; N];
        let raw = s.as_bytes();
        let take = raw.len().min(N);
        bytes[..take].copy_from_slice(&raw[..take]);
        Self { bytes }
    }

    /// The raw field exactly as stored, for re-encoding.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; N] {
        &self.bytes
    }

    /// The value with padding removed.
    ///
    /// Stops at the first NUL and then trims trailing spaces, which covers both conventions
    /// without the caller needing to know which venue the field came from. Non-UTF-8 input
    /// yields an empty string rather than a panic — a corrupt field must not take down a feed
    /// handler.
    #[inline]
    pub fn as_str(&self) -> &str {
        let end = self.bytes.iter().position(|&b| b == 0).unwrap_or(N);
        let trimmed = &self.bytes[..end];
        let end = trimmed
            .iter()
            .rposition(|&b| b != b' ')
            .map_or(0, |idx| idx + 1);
        std::str::from_utf8(&trimmed[..end]).unwrap_or("")
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.as_str().len()
    }

    /// Field width in bytes.
    pub const fn width() -> usize {
        N
    }
}

impl<const N: usize> Default for FixedStr<N> {
    fn default() -> Self {
        Self::BLANK
    }
}

impl<const N: usize> fmt::Display for FixedStr<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const N: usize> fmt::Debug for FixedStr<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl<const N: usize> From<&str> for FixedStr<N> {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl<const N: usize> PartialEq<str> for FixedStr<N> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl<const N: usize> PartialEq<&str> for FixedStr<N> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// An 8-byte Nasdaq stock symbol.
pub type Symbol8 = FixedStr<8>;
/// A 4-byte market participant identifier (Nasdaq MPID, NYSE MPID, Nasdaq Firm).
pub type Mpid4 = FixedStr<4>;
/// A 5-byte NYSE Integrated Feed FirmID.
pub type FirmId5 = FixedStr<5>;
/// An 8-byte user-data / reference field.
pub type UserData8 = FixedStr<8>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_padded_fields_trim_on_read_and_survive_re_encoding() {
        let s = Symbol8::new("AAPL");
        assert_eq!(s.as_str(), "AAPL");
        assert_eq!(s.as_bytes(), b"AAPL    ");
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn nul_padded_fields_stop_at_the_terminator() {
        let f = FirmId5::new_nul_padded("XYZ");
        assert_eq!(f.as_str(), "XYZ");
        assert_eq!(f.as_bytes(), b"XYZ\0\0");
    }

    #[test]
    fn a_wire_field_of_either_convention_reads_the_same() {
        assert_eq!(FirmId5::from_wire(b"ABCD ").as_str(), "ABCD");
        assert_eq!(FirmId5::from_wire(b"ABCD\0").as_str(), "ABCD");
        assert_eq!(FirmId5::from_wire(b"AB\0\0\0").as_str(), "AB");
        assert_eq!(FirmId5::from_wire(b"     ").as_str(), "");
        assert_eq!(FirmId5::from_wire(b"\0\0\0\0\0").as_str(), "");
    }

    #[test]
    fn a_full_width_value_keeps_every_byte() {
        let s = Symbol8::new("ABCDEFGH");
        assert_eq!(s.as_str(), "ABCDEFGH");
        assert_eq!(s.len(), 8);
    }

    #[test]
    fn try_new_rejects_what_new_would_silently_truncate() {
        assert!(Symbol8::try_new("TOOLONGSYMBOL").is_err());
        assert!(Symbol8::try_new("ABCDEFGH").is_ok());
        assert_eq!(Symbol8::try_new("AAPL").unwrap().as_str(), "AAPL");
        // The lenient constructor is what makes the checked one necessary.
        assert_eq!(Symbol8::new("TOOLONGSYMBOL").as_str(), "TOOLONGS");
    }

    #[test]
    fn overlong_input_is_truncated_to_the_field_width() {
        assert_eq!(Symbol8::new("TOOLONGSYMBOL").as_str(), "TOOLONGS");
        assert_eq!(FirmId5::from_wire(b"ABCDEFGH").as_str(), "ABCDE");
    }

    #[test]
    fn short_input_is_padded_not_left_uninitialised() {
        assert_eq!(Symbol8::from_wire(b"AB").as_bytes(), b"AB      ");
    }

    #[test]
    fn blank_and_nul_forms_both_read_as_empty() {
        assert!(Symbol8::BLANK.is_empty());
        assert!(Symbol8::NUL.is_empty());
        assert!(Symbol8::default().is_empty());
    }

    #[test]
    fn a_non_utf8_field_yields_an_empty_string_rather_than_panicking() {
        let bad = Symbol8::from_wire(&[0xFF, 0xFE, b'A', b' ', b' ', b' ', b' ', b' ']);
        assert_eq!(
            bad.as_str(),
            "",
            "corrupt text must not take down a feed handler"
        );
    }

    #[test]
    fn comparison_against_str_works_without_allocating() {
        let s = Symbol8::new("IBM");
        assert_eq!(s, "IBM");
        assert_ne!(s, "IBMX");
        assert_eq!(s.to_string(), "IBM");
        assert_eq!(format!("{s:?}"), "\"IBM\"");
    }

    #[test]
    fn the_type_is_copy_and_inline() {
        // The whole point: no pointer, no allocation, and cheap to move.
        assert_eq!(std::mem::size_of::<Symbol8>(), 8);
        assert_eq!(std::mem::size_of::<Mpid4>(), 4);
        assert_eq!(std::mem::size_of::<FirmId5>(), 5);
        assert!(std::mem::size_of::<Symbol8>() < std::mem::size_of::<String>());

        let a = Symbol8::new("AAPL");
        let b = a; // Copy, not move
        assert_eq!(a, b);
    }

    #[test]
    fn width_is_reported_by_the_type() {
        assert_eq!(Symbol8::width(), 8);
        assert_eq!(Mpid4::width(), 4);
        assert_eq!(FirmId5::width(), 5);
    }
}
