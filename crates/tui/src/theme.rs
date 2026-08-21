//! Central colour palette for the terminal.
//!
//! A Bloomberg/Rust trading-terminal aesthetic: a near-black charcoal base,
//! warm off-white typography, neon mint-green positive indicators, muted
//! crimson negatives, and restrained blue, violet, cyan, amber and yellow
//! accents. Saturated and slightly desaturated — deliberately not pastel.
//!
//! # Why every colour lives here
//!
//! Widgets must never construct a `Color` inline. Two rules follow from that:
//!
//! 1. **No named ANSI colours** (`Color::Yellow`, `Color::DarkGray`, …). Those
//!    resolve against the *user's* terminal palette, so the same build looks
//!    different in Windows Terminal, iTerm2 and a bare console — and "muted
//!    gold" becomes whatever the user's theme calls yellow. Every colour below
//!    is a true-colour RGB triple so it renders identically everywhere.
//! 2. **One definition per concept.** Changing the bull colour should be a
//!    one-line edit, not a search across six files.

use ratatui::style::Color;

// ── Base surfaces ───────────────────────────────────────────────────────────

/// Terminal background — near-black charcoal. `#0B0F10`
pub const BG: Color = Color::Rgb(0x0B, 0x0F, 0x10);
/// Panel fill — dark graphite. `#11171A`
pub const PANEL: Color = Color::Rgb(0x11, 0x17, 0x1A);
/// Alternate panel fill, for banded rows and nested blocks. `#151B1E`
pub const PANEL_ALT: Color = Color::Rgb(0x15, 0x1B, 0x1E);
/// Panel borders and rules — muted dark gray. `#394247`
pub const BORDER: Color = Color::Rgb(0x39, 0x42, 0x47);

// ── Typography ──────────────────────────────────────────────────────────────

/// Primary text — warm off-white. `#D7D9D6`
pub const TEXT: Color = Color::Rgb(0xD7, 0xD9, 0xD6);
/// Secondary text: labels, column headers, units. `#8B9294`
pub const TEXT_DIM: Color = Color::Rgb(0x8B, 0x92, 0x94);
/// Tertiary text: disabled entries, watermarks, axis minor ticks.
/// Derived by interpolating `TEXT_DIM` toward `BORDER`. `#626A6D`
pub const TEXT_FAINT: Color = Color::Rgb(0x62, 0x6A, 0x6D);

// ── Directional (P&L, candles, quotes) ──────────────────────────────────────

/// Positive / bid / bullish — neon mint. `#58E58A`
pub const POSITIVE: Color = Color::Rgb(0x58, 0xE5, 0x8A);
/// Emphasis for a live uptick or a flashing gain. `#63F08F`
pub const POSITIVE_BRIGHT: Color = Color::Rgb(0x63, 0xF0, 0x8F);
/// Filled bull volume bars and depth shading — derived dark mint. `#2E6B45`
pub const POSITIVE_DEEP: Color = Color::Rgb(0x2E, 0x6B, 0x45);

/// Negative / ask / bearish — muted coral-red. `#D85C5C`
pub const NEGATIVE: Color = Color::Rgb(0xD8, 0x5C, 0x5C);
/// Filled bear volume bars and depth shading — dark brick. `#7E3D40`
pub const NEGATIVE_DEEP: Color = Color::Rgb(0x7E, 0x3D, 0x40);

// ── Accents ─────────────────────────────────────────────────────────────────

/// Electric blue — selection, focus, informational highlights. `#5CA9E6`
pub const BLUE: Color = Color::Rgb(0x5C, 0xA9, 0xE6);
/// Soft violet — AI/analytics surfaces. `#A77BEF`
pub const PURPLE: Color = Color::Rgb(0xA7, 0x7B, 0xEF);
/// Amber — warnings, degraded state, pending fills. `#E59A55`
pub const ORANGE: Color = Color::Rgb(0xE5, 0x9A, 0x55);
/// Teal — live/streaming indicators, latency, clocks. `#54D7D0`
pub const CYAN: Color = Color::Rgb(0x54, 0xD7, 0xD0);
/// Muted gold — figures needing attention without alarm. `#D6C45A`
pub const YELLOW: Color = Color::Rgb(0xD6, 0xC4, 0x5A);

// ── Semantic aliases ────────────────────────────────────────────────────────
// Widgets should prefer these: they say what the colour *means*, so the
// intent survives a future palette change.

/// Bullish candle body and wick.
pub const BULL: Color = POSITIVE;
/// Bearish candle body and wick.
pub const BEAR: Color = NEGATIVE;
/// Bull volume histogram.
pub const BULL_DIM: Color = POSITIVE_DEEP;
/// Bear volume histogram.
pub const BEAR_DIM: Color = NEGATIVE_DEEP;
/// Crosshair and guide lines.
pub const CROSSHAIR: Color = TEXT_FAINT;
/// Background of the last-price tag pinned into the price axis.
pub const PRICE_TAG_BG: Color = BLUE;
/// Text drawn on top of a filled accent chip.
pub const ON_ACCENT: Color = BG;
/// Brand mark in the header.
pub const BRAND: Color = ORANGE;
/// Healthy connection / live feed.
pub const OK: Color = POSITIVE;
/// Recoverable problem — reconnecting, stale data.
pub const WARN: Color = ORANGE;
/// Hard failure — rejected order, auth failure.
pub const ERR: Color = NEGATIVE;

/// Directional colour for a signed change.
///
/// Returns [`TEXT_DIM`] for an exactly flat value, so a row of genuinely
/// unchanged quotes does not read as a wall of green.
#[inline]
#[must_use]
pub fn delta(change: f64) -> Color {
    if change > 0.0 {
        POSITIVE
    } else if change < 0.0 {
        NEGATIVE
    } else {
        TEXT_DIM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_is_flat_for_zero() {
        assert_eq!(delta(0.0), TEXT_DIM);
        assert_eq!(delta(0.01), POSITIVE);
        assert_eq!(delta(-0.01), NEGATIVE);
    }

    #[test]
    fn palette_is_true_colour_only() {
        // Guards the invariant in the module docs: a named ANSI colour here
        // would silently pick up the user's terminal theme.
        for c in [
            BG,
            PANEL,
            PANEL_ALT,
            BORDER,
            TEXT,
            TEXT_DIM,
            TEXT_FAINT,
            POSITIVE,
            POSITIVE_BRIGHT,
            POSITIVE_DEEP,
            NEGATIVE,
            NEGATIVE_DEEP,
            BLUE,
            PURPLE,
            ORANGE,
            CYAN,
            YELLOW,
        ] {
            assert!(matches!(c, Color::Rgb(_, _, _)), "{c:?} is not true-colour");
        }
    }
}
