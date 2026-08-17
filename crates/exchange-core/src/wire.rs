//! Bounds-checked, allocation-free binary reader/writer.
//!
//! Both endiannesses are supported on one cursor because the two venues disagree:
//!
//! * Nasdaq ITCH 5.0 / OUCH 4.2 / SoupBinTCP / MoldUDP64 — "All integer fields are big
//!   endian (network byte order) binary encoded numbers", alpha fields left-justified and
//!   **space** padded.
//! * NYSE Pillar XDP and the Pillar Binary Gateway — "Binary fields are published in
//!   Little-Endian ordering", ASCII strings left aligned and **null** padded.

use crate::error::{WireError, WireResult};

/// A read cursor over a borrowed byte slice.
///
/// Every accessor is bounds-checked and returns [`WireError::Truncated`] rather than
/// panicking, because the bytes come off a network socket and a malformed packet must not
/// be able to abort a feed handler.
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    #[inline]
    pub const fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Start reading at `offset` bytes into `buf`.
    #[inline]
    pub fn at(buf: &'a [u8], offset: usize) -> Self {
        Self { buf, pos: offset }
    }

    #[inline]
    pub const fn position(&self) -> usize {
        self.pos
    }

    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Move the cursor to an absolute offset. Used by decoders that address fields by the
    /// offsets printed in the specification tables rather than reading sequentially.
    #[inline]
    pub fn seek(&mut self, offset: usize) {
        self.pos = offset;
    }

    #[inline]
    pub fn skip(&mut self, n: usize) -> WireResult<()> {
        self.take(n).map(|_| ())
    }

    /// Borrow the next `n` bytes and advance.
    #[inline]
    pub fn take(&mut self, n: usize) -> WireResult<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(WireError::Truncated {
            at: self.pos,
            need: n,
            have: self.remaining(),
        })?;
        if end > self.buf.len() {
            return Err(WireError::Truncated {
                at: self.pos,
                need: n,
                have: self.remaining(),
            });
        }
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// Borrow the remainder without advancing.
    #[inline]
    pub fn peek_rest(&self) -> &'a [u8] {
        &self.buf[self.pos.min(self.buf.len())..]
    }

    // ── Fixed-width integers ────────────────────────────────────────────────

    #[inline]
    pub fn u8(&mut self) -> WireResult<u8> {
        Ok(self.take(1)?[0])
    }

    #[inline]
    pub fn i8(&mut self) -> WireResult<i8> {
        Ok(self.take(1)?[0] as i8)
    }

    #[inline]
    pub fn be_u16(&mut self) -> WireResult<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    #[inline]
    pub fn be_u32(&mut self) -> WireResult<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    #[inline]
    pub fn be_u64(&mut self) -> WireResult<u64> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// ITCH timestamps are 6-byte big-endian nanoseconds-since-midnight.
    #[inline]
    pub fn be_u48(&mut self) -> WireResult<u64> {
        let b = self.take(6)?;
        Ok(u64::from_be_bytes([
            0, 0, b[0], b[1], b[2], b[3], b[4], b[5],
        ]))
    }

    #[inline]
    pub fn le_u16(&mut self) -> WireResult<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    #[inline]
    pub fn le_u32(&mut self) -> WireResult<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    #[inline]
    pub fn le_i32(&mut self) -> WireResult<i32> {
        Ok(self.le_u32()? as i32)
    }

    #[inline]
    pub fn le_u64(&mut self) -> WireResult<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    // ── Text ────────────────────────────────────────────────────────────────

    /// A single ASCII character field.
    #[inline]
    pub fn ascii_char(&mut self) -> WireResult<char> {
        Ok(self.u8()? as char)
    }

    /// Fixed-width field, right-padded with spaces (Nasdaq convention).
    #[inline]
    pub fn space_padded(&mut self, n: usize, field: &'static str) -> WireResult<&'a str> {
        let raw = self.take(n)?;
        let end = raw
            .iter()
            .rposition(|&b| b != b' ')
            .map_or(0, |idx| idx + 1);
        std::str::from_utf8(&raw[..end]).map_err(|_| WireError::NotAscii { field })
    }

    /// Fixed-width field, right-padded with NUL (NYSE convention).
    #[inline]
    pub fn nul_padded(&mut self, n: usize, field: &'static str) -> WireResult<&'a str> {
        let raw = self.take(n)?;
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        std::str::from_utf8(&raw[..end])
            .map(str::trim_end)
            .map_err(|_| WireError::NotAscii { field })
    }
}

/// A growable write buffer for building outbound order-entry messages.
///
/// Order entry is the only place this crate serialises: OUCH 4.2 (big-endian) and the
/// Pillar Binary Gateway (little-endian) both need exact byte layouts.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    #[inline]
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    #[inline]
    pub fn with_capacity(n: usize) -> Self {
        Self {
            buf: Vec::with_capacity(n),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    #[inline]
    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    #[inline]
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    #[inline]
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    #[inline]
    pub fn ascii_char(&mut self, v: char) -> &mut Self {
        self.buf.push(v as u8);
        self
    }

    #[inline]
    pub fn be_u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    #[inline]
    pub fn be_u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    #[inline]
    pub fn be_u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    #[inline]
    pub fn le_u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    #[inline]
    pub fn le_u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    #[inline]
    pub fn le_u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    #[inline]
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(bytes);
        self
    }

    /// Write `s` into an `n`-byte field, right-padded with `pad`, truncating if longer.
    pub fn padded(&mut self, s: &str, n: usize, pad: u8) -> &mut Self {
        let bytes = s.as_bytes();
        let take = bytes.len().min(n);
        self.buf.extend_from_slice(&bytes[..take]);
        self.buf.extend(std::iter::repeat_n(pad, n - take));
        self
    }

    /// Nasdaq alpha field: right-padded with spaces.
    #[inline]
    pub fn space_padded(&mut self, s: &str, n: usize) -> &mut Self {
        self.padded(s, n, b' ')
    }

    /// NYSE `zchar(n)` field: right-padded with NUL.
    #[inline]
    pub fn nul_padded(&mut self, s: &str, n: usize) -> &mut Self {
        self.padded(s, n, 0)
    }

    /// SoupBinTCP login packets carry sequence numbers as *right-aligned ASCII digits in a
    /// fixed-width field, left-padded with spaces*.
    pub fn ascii_numeric_right(&mut self, v: u64, n: usize) -> &mut Self {
        let s = v.to_string();
        let bytes = s.as_bytes();
        if bytes.len() >= n {
            self.buf.extend_from_slice(&bytes[bytes.len() - n..]);
        } else {
            self.buf.extend(std::iter::repeat_n(b' ', n - bytes.len()));
            self.buf.extend_from_slice(bytes);
        }
        self
    }

    /// Patch a previously written little-endian `u16` (used for `MsgHeader.length`).
    pub fn patch_le_u16(&mut self, offset: usize, v: u16) {
        let b = v.to_le_bytes();
        if offset + 2 <= self.buf.len() {
            self.buf[offset] = b[0];
            self.buf[offset + 1] = b[1];
        }
    }
}

/// Parse a fixed-width ASCII numeric field (spaces allowed as padding on either side).
pub fn parse_ascii_u64(raw: &[u8], field: &'static str) -> WireResult<u64> {
    let s = std::str::from_utf8(raw).map_err(|_| WireError::NotAscii { field })?;
    let t = s.trim();
    if t.is_empty() {
        return Ok(0);
    }
    t.parse::<u64>().map_err(|_| WireError::NotAscii { field })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_is_an_error_not_a_panic() {
        let mut c = Cursor::new(&[0x01, 0x02]);
        assert!(c.be_u32().is_err());
        assert_eq!(c.position(), 0, "failed read must not advance the cursor");
    }

    #[test]
    fn be_u48_matches_itch_timestamp_layout() {
        // 34,200,000,000,000 ns = 09:30:00.000 ET open, as it appears on the wire.
        let ns: u64 = 34_200_000_000_000;
        let full = ns.to_be_bytes();
        let mut c = Cursor::new(&full[2..]);
        assert_eq!(c.be_u48().unwrap(), ns);
    }

    #[test]
    fn space_padded_trims_only_trailing_spaces() {
        let mut c = Cursor::new(b"AAPL    ");
        assert_eq!(c.space_padded(8, "stock").unwrap(), "AAPL");
    }

    #[test]
    fn nul_padded_stops_at_first_nul() {
        let mut c = Cursor::new(b"MSFT\0\0\0\0\0\0\0");
        assert_eq!(c.nul_padded(11, "symbol").unwrap(), "MSFT");
    }

    #[test]
    fn ascii_numeric_right_pads_left_with_spaces() {
        let mut w = Writer::new();
        w.ascii_numeric_right(1, 20);
        assert_eq!(w.as_slice(), b"                   1");
    }

    #[test]
    fn ascii_numeric_right_truncates_from_the_left_when_too_wide() {
        let mut w = Writer::new();
        w.ascii_numeric_right(123456, 4);
        assert_eq!(w.as_slice(), b"3456");
    }

    #[test]
    fn writer_round_trips_through_cursor() {
        let mut w = Writer::new();
        w.be_u64(42).le_u32(7).space_padded("IBM", 8);
        let bytes = w.into_vec();
        let mut c = Cursor::new(&bytes);
        assert_eq!(c.be_u64().unwrap(), 42);
        assert_eq!(c.le_u32().unwrap(), 7);
        assert_eq!(c.space_padded(8, "s").unwrap(), "IBM");
    }
}
