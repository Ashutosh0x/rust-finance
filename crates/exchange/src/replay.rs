//! Capture replay: drive the real feed handlers from recorded bytes.
//!
//! Direct feeds are entitled, so most work on a feed handler happens without one. Replay
//! closes that gap honestly: it takes bytes that came off an exchange (or that a test wrote
//! deliberately) and pushes them through exactly the same decoders, book builders and
//! normalisers the live path uses. No parallel implementation, no simulated exchange.
//!
//! Two capture formats, matching how the two venues are actually recorded:
//!
//! * [`ReplayFormat::MoldUdp64`] — length-prefixed MoldUDP64 datagrams, as a UDP capture is
//!   normally flattened for storage.
//! * [`ReplayFormat::Xdp`] — length-prefixed XDP packets, likewise.
//!
//! The framing is a 4-byte big-endian length before each datagram, which is the minimum
//! needed to recover datagram boundaries from a byte stream. It is a *storage* format, not a
//! wire format, and it is stated here so nobody mistakes it for one.
//!
//! What replay is for: validating a book builder against a known-good session, reproducing a
//! production incident, and regression-testing decode changes. What it is not for: producing
//! market data. A replayed book is labelled as replayed by the caller, and nothing here ever
//! presents it as live.

use std::io::Read;
use std::path::Path;

use common::events::{Envelope, MarketEvent};
use ingestion::source::DataType;
use nasdaq::itch::{ItchFeedHandler, SessionClock};
use nasdaq::moldudp64::MoldPacket;
use nyse::xdp::XdpFeedHandler;

use crate::normalize::Normalizer;

/// Which venue's capture is being replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayFormat {
    /// MoldUDP64 datagrams carrying ITCH 5.0.
    MoldUdp64,
    /// XDP packets carrying the NYSE Integrated Feed.
    Xdp,
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("I/O error reading the capture: {0}")]
    Io(#[from] std::io::Error),
    #[error("capture is truncated at offset {offset}: need {need} bytes, {have} remain")]
    Truncated {
        offset: usize,
        need: usize,
        have: usize,
    },
    #[error("datagram at offset {offset} declares {len} bytes, which exceeds the 64 KiB maximum")]
    OversizedDatagram { offset: usize, len: usize },
}

/// Split a length-prefixed capture into datagrams.
pub struct CaptureReader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> CaptureReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    /// Next datagram, or `None` at a clean end of file.
    pub fn next_datagram(&mut self) -> Result<Option<&'a [u8]>, ReplayError> {
        if self.offset >= self.data.len() {
            return Ok(None);
        }
        if self.offset + 4 > self.data.len() {
            return Err(ReplayError::Truncated {
                offset: self.offset,
                need: 4,
                have: self.data.len() - self.offset,
            });
        }
        let len = u32::from_be_bytes([
            self.data[self.offset],
            self.data[self.offset + 1],
            self.data[self.offset + 2],
            self.data[self.offset + 3],
        ]) as usize;
        if len > 65_536 {
            return Err(ReplayError::OversizedDatagram {
                offset: self.offset,
                len,
            });
        }
        let start = self.offset + 4;
        if start + len > self.data.len() {
            return Err(ReplayError::Truncated {
                offset: self.offset,
                need: len,
                have: self.data.len() - start,
            });
        }
        self.offset = start + len;
        Ok(Some(&self.data[start..start + len]))
    }
}

/// Frame a datagram for storage in a capture file.
pub fn frame_datagram(datagram: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(datagram.len() as u32).to_be_bytes());
    out.extend_from_slice(datagram);
}

/// What a replay produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayStats {
    pub datagrams: u64,
    pub messages: u64,
    pub events_emitted: u64,
    pub decode_errors: u64,
    pub book_errors: u64,
    /// Instruments whose price scale or ticker never arrived, so their messages could not
    /// be interpreted. A non-zero value here means the capture is missing its directory
    /// spin, not that the decoder is broken.
    pub unresolved_instruments: u64,
}

/// Replays a capture through the live decoding path.
pub struct Replayer {
    format: ReplayFormat,
    normalizer: Normalizer,
    itch: Option<ItchFeedHandler>,
    xdp: Option<XdpFeedHandler>,
    stats: ReplayStats,
}

impl Replayer {
    /// `session_midnight_epoch_nanos` matters only for ITCH, whose timestamps are relative
    /// to local midnight; pass 0 to keep raw nanoseconds-since-midnight.
    pub fn new(
        format: ReplayFormat,
        data_types: &[DataType],
        depth_levels: usize,
        session_midnight_epoch_nanos: u64,
    ) -> Self {
        Self {
            format,
            normalizer: Normalizer::new(data_types, depth_levels),
            itch: matches!(format, ReplayFormat::MoldUdp64)
                .then(|| ItchFeedHandler::new(SessionClock::new(session_midnight_epoch_nanos))),
            xdp: matches!(format, ReplayFormat::Xdp).then(XdpFeedHandler::new),
            stats: ReplayStats::default(),
        }
    }

    pub fn stats(&self) -> &ReplayStats {
        &self.stats
    }

    /// The ITCH handler, for inspecting book state after a replay.
    pub fn itch(&self) -> Option<&ItchFeedHandler> {
        self.itch.as_ref()
    }

    /// The XDP handler, for inspecting book state after a replay.
    pub fn xdp(&self) -> Option<&XdpFeedHandler> {
        self.xdp.as_ref()
    }

    /// Replay a whole capture, collecting the events it produces.
    pub fn replay(&mut self, capture: &[u8]) -> Result<Vec<Envelope<MarketEvent>>, ReplayError> {
        let mut out = Vec::new();
        let mut reader = CaptureReader::new(capture);
        while let Some(datagram) = reader.next_datagram()? {
            self.stats.datagrams += 1;
            out.extend(self.on_datagram(datagram));
        }
        self.stats.events_emitted = out.len() as u64;
        Ok(out)
    }

    /// Replay from a file.
    pub fn replay_file(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<Envelope<MarketEvent>>, ReplayError> {
        let mut buf = Vec::new();
        std::fs::File::open(path)?.read_to_end(&mut buf)?;
        self.replay(&buf)
    }

    /// Feed one datagram.
    pub fn on_datagram(&mut self, datagram: &[u8]) -> Vec<Envelope<MarketEvent>> {
        match self.format {
            ReplayFormat::MoldUdp64 => self.on_mold(datagram),
            ReplayFormat::Xdp => self.on_xdp(datagram),
        }
    }

    fn on_mold(&mut self, datagram: &[u8]) -> Vec<Envelope<MarketEvent>> {
        let Some(handler) = self.itch.as_mut() else {
            return Vec::new();
        };
        let Ok(packet) = MoldPacket::parse(datagram) else {
            self.stats.decode_errors += 1;
            return Vec::new();
        };
        // Strict validation, so a malformed capture is reported rather than half-applied.
        let Ok(messages) = packet.validate() else {
            self.stats.decode_errors += 1;
            return Vec::new();
        };

        let mut out = Vec::new();
        for raw in messages {
            self.stats.messages += 1;
            let applied = match handler.on_message(raw, 0) {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(_) => {
                    self.stats.decode_errors += 1;
                    continue;
                }
            };
            let key = applied.key();
            let Some(book) = handler.books().get(key) else {
                continue;
            };
            let symbol = handler.directory().symbol(key as u16);
            if symbol.is_none() && book.symbol().is_empty() {
                self.stats.unresolved_instruments += 1;
            }
            out.extend(self.normalizer.on_event(&applied, book, symbol));
        }
        self.stats.book_errors = handler.stats().book_errors;
        out
    }

    fn on_xdp(&mut self, datagram: &[u8]) -> Vec<Envelope<MarketEvent>> {
        let Some(handler) = self.xdp.as_mut() else {
            return Vec::new();
        };
        if handler.on_packet(datagram, 0).is_err() {
            self.stats.decode_errors += 1;
            return Vec::new();
        }

        let mut out = Vec::new();
        for event in handler.drain_events() {
            self.stats.messages += 1;
            let key = event.key();
            let Some(book) = handler.books().get(key) else {
                continue;
            };
            let symbol = handler.directory().symbol(key);
            if symbol.is_none() {
                self.stats.unresolved_instruments += 1;
            }
            out.extend(self.normalizer.on_event(&event, book, symbol));
        }
        self.stats.book_errors = handler.stats().book_errors;
        self.stats.unresolved_instruments += handler.stats().awaiting_symbol_mapping;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange_core::Price;

    fn itch_capture() -> Vec<u8> {
        use nasdaq::itch::{encode, Header};
        use nasdaq::moldudp64::{encode_packet, SessionId};

        let h = |ts: u64| Header {
            stock_locate: 1,
            tracking_number: 0,
            timestamp: ts,
        };
        let directory = encode::stock_directory(
            h(1),
            "AAPL",
            'Q',
            'N',
            100,
            false,
            'C',
            "",
            'P',
            'N',
            'N',
            '1',
            'N',
            0,
            false,
        );
        let bid = encode::add_order(
            h(2),
            1,
            'B',
            500,
            "AAPL",
            Price::from_price4(1_000_000),
            None,
        );
        let ask = encode::add_order(
            h(3),
            2,
            'S',
            300,
            "AAPL",
            Price::from_price4(1_000_200),
            None,
        );
        let fill = encode::order_executed(h(4), 2, 100, 900);

        let session = SessionId::from_str_padded("20260818AA");
        let p1 = encode_packet(session, 1, &[&directory, &bid]);
        let p2 = encode_packet(session, 3, &[&ask, &fill]);

        let mut out = Vec::new();
        frame_datagram(&p1, &mut out);
        frame_datagram(&p2, &mut out);
        out
    }

    fn xdp_capture() -> Vec<u8> {
        use nyse::xdp::common::{encode_symbol_index_mapping, SymbolIndexMapping};
        use nyse::xdp::{integrated, packet};

        let mapping = encode_symbol_index_mapping(&SymbolIndexMapping {
            symbol_index: 4242,
            symbol: "IBM".into(),
            market_id: 1,
            system_id: 1,
            exchange_code: 'N',
            price_scale_code: 4,
            security_type: 'C',
            lot_size: 100,
            prev_close_price: Price::ZERO,
            prev_close_volume: 0,
            price_resolution: 0,
            round_lot: 'Y',
            mpv: 1,
            unit_of_trade: 100,
        });
        let time_ref = nyse::xdp::common::encode_source_time_reference(0, 0, 1_700_000_000);
        let add = integrated::AddOrder {
            source_time_nanos: 1,
            symbol_index: 4242,
            symbol_seq_num: 1,
            order_id: 1,
            price_raw: 1_805_000,
            volume: 500,
            side: integrated::Side::Buy,
            firm_id: exchange_core::FirmId5::NUL,
        }
        .encode();

        let pkt = packet::encode_packet(
            packet::delivery_flag::ORIGINAL,
            1,
            1_700_000_000,
            0,
            &[&mapping, &time_ref, &add],
        );
        let mut out = Vec::new();
        frame_datagram(&pkt, &mut out);
        out
    }

    #[test]
    fn framing_round_trips_through_the_reader() {
        let mut capture = Vec::new();
        for payload in [&b"aaa"[..], &b""[..], &b"bbbbbb"[..]] {
            frame_datagram(payload, &mut capture);
        }
        let mut reader = CaptureReader::new(&capture);
        assert_eq!(reader.next_datagram().unwrap(), Some(&b"aaa"[..]));
        assert_eq!(reader.next_datagram().unwrap(), Some(&b""[..]));
        assert_eq!(reader.next_datagram().unwrap(), Some(&b"bbbbbb"[..]));
        assert_eq!(reader.next_datagram().unwrap(), None);
    }

    #[test]
    fn a_truncated_capture_is_reported_rather_than_silently_short() {
        let mut capture = Vec::new();
        frame_datagram(b"0123456789", &mut capture);
        capture.truncate(capture.len() - 3);
        let mut reader = CaptureReader::new(&capture);
        assert!(matches!(
            reader.next_datagram(),
            Err(ReplayError::Truncated { .. })
        ));
    }

    #[test]
    fn an_absurd_length_prefix_is_rejected() {
        let capture = vec![0xFF, 0xFF, 0xFF, 0xFF];
        let mut reader = CaptureReader::new(&capture);
        assert!(matches!(
            reader.next_datagram(),
            Err(ReplayError::OversizedDatagram { .. })
        ));
    }

    #[test]
    fn replaying_an_itch_capture_rebuilds_the_book() {
        let mut r = Replayer::new(
            ReplayFormat::MoldUdp64,
            &[DataType::Trades, DataType::Quotes],
            5,
            0,
        );
        let events = r.replay(&itch_capture()).unwrap();

        let book = r.itch().unwrap().books().get(1).expect("book for AAPL");
        assert_eq!(book.symbol(), "AAPL");
        assert_eq!(book.best_bid(), Some((Price::from_price4(1_000_000), 500)));
        assert_eq!(book.best_ask(), Some((Price::from_price4(1_000_200), 200)));
        assert_eq!(book.stats().printed_volume, 100);

        assert_eq!(r.stats().datagrams, 2);
        assert_eq!(r.stats().messages, 4);
        assert_eq!(r.stats().decode_errors, 0);
        assert_eq!(r.stats().book_errors, 0);

        let trades = events
            .iter()
            .filter(|e| matches!(e.payload, MarketEvent::Trade(_)))
            .count();
        assert_eq!(trades, 1);
        assert!(events
            .iter()
            .any(|e| matches!(e.payload, MarketEvent::Quote(_))));
    }

    #[test]
    fn replaying_an_xdp_capture_rebuilds_the_book() {
        let mut r = Replayer::new(ReplayFormat::Xdp, &[DataType::Quotes], 5, 0);
        let events = r.replay(&xdp_capture()).unwrap();

        let book = r.xdp().unwrap().books().get(4242).expect("book for IBM");
        assert_eq!(book.symbol(), "IBM");
        assert_eq!(book.best_bid().unwrap().0.to_string(), "180.50");
        assert_eq!(r.stats().decode_errors, 0);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn replay_is_deterministic() {
        let capture = itch_capture();
        let run = |c: &[u8]| {
            let mut r = Replayer::new(ReplayFormat::MoldUdp64, &[DataType::Quotes], 5, 0);
            let events = r.replay(c).unwrap();
            let bbo = r.itch().unwrap().books().get(1).unwrap().bbo();
            (events.len(), bbo)
        };
        assert_eq!(run(&capture), run(&capture));
    }

    #[test]
    fn the_session_clock_shifts_replayed_timestamps_onto_the_epoch() {
        let capture = itch_capture();
        let mut raw = Replayer::new(ReplayFormat::MoldUdp64, &[DataType::Quotes], 5, 0);
        let raw_events = raw.replay(&capture).unwrap();

        let midnight = 1_755_489_600_000_000_000u64;
        let mut based = Replayer::new(ReplayFormat::MoldUdp64, &[DataType::Quotes], 5, midnight);
        let based_events = based.replay(&capture).unwrap();

        assert_eq!(raw_events.len(), based_events.len());
        assert_eq!(
            based_events[0].ts_event.as_u64() - raw_events[0].ts_event.as_u64(),
            midnight
        );
    }

    #[test]
    fn a_capture_without_its_directory_spin_is_reported_as_unresolved() {
        use nyse::xdp::{integrated, packet};
        let add = integrated::AddOrder {
            source_time_nanos: 1,
            symbol_index: 4242,
            symbol_seq_num: 1,
            order_id: 1,
            price_raw: 1_805_000,
            volume: 500,
            side: integrated::Side::Buy,
            firm_id: exchange_core::FirmId5::NUL,
        }
        .encode();
        let pkt = packet::encode_packet(packet::delivery_flag::ORIGINAL, 1, 0, 0, &[&add]);
        let mut capture = Vec::new();
        frame_datagram(&pkt, &mut capture);

        let mut r = Replayer::new(ReplayFormat::Xdp, &[DataType::Quotes], 5, 0);
        let events = r.replay(&capture).unwrap();
        assert!(events.is_empty(), "no price scale, so nothing is published");
        assert_eq!(
            r.stats().unresolved_instruments,
            1,
            "and the reason is reported rather than hidden"
        );
    }

    #[test]
    fn a_malformed_datagram_is_counted_and_skipped() {
        let mut capture = Vec::new();
        frame_datagram(&[0u8; 8], &mut capture); // too short to be a Mold packet
        let mut r = Replayer::new(ReplayFormat::MoldUdp64, &[DataType::Quotes], 5, 0);
        let events = r.replay(&capture).unwrap();
        assert!(events.is_empty());
        assert_eq!(r.stats().decode_errors, 1);
    }

    #[test]
    fn an_empty_capture_replays_to_nothing() {
        let mut r = Replayer::new(ReplayFormat::MoldUdp64, &[DataType::Quotes], 5, 0);
        assert!(r.replay(&[]).unwrap().is_empty());
        assert_eq!(r.stats().datagrams, 0);
    }
}
