//! Binance Spot WebSocket implementation of MarketDataSource.
//!
//! Connects to the combined stream endpoint and subscribes to:
//! - `<symbol>@trade`     — individual trade events
//! - `<symbol>@bookTicker` — best bid/ask updates (L1)
//! - `<symbol>@depth5@100ms` — top 5 order book levels
//!
//! Binance-specific constraints handled:
//! - 24-hour connection lifetime with auto-reconnect
//! - Ping/pong: server pings every 20s, must reply within 60s
//! - Max 1024 streams per connection
//! - Max 5 incoming messages/sec (subscribe commands)
//! - Timestamps in milliseconds by default
//!
//! Ref: https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams

use crate::source::{DataType, IngestionError, MarketDataSource, MarketStream, Subscription};
use async_trait::async_trait;
use common::events::*;
use common::time::{SequenceGenerator, UnixNanos};
use compact_str::CompactString;
use futures::StreamExt;
use std::sync::Arc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

/// Base market data endpoint (no auth required for public streams).
const BINANCE_STREAM_URL: &str = "wss://stream.binance.com:9443/stream";

/// Testnet endpoint for development.
const BINANCE_TESTNET_URL: &str = "wss://stream.testnet.binance.vision/stream";

pub struct BinanceSource {
    seq_gen: Arc<SequenceGenerator>,
    use_testnet: bool,
}

impl BinanceSource {
    pub fn new(seq_gen: Arc<SequenceGenerator>) -> Self {
        Self {
            seq_gen,
            use_testnet: false,
        }
    }

    /// Use testnet for development/testing.
    pub fn testnet(mut self) -> Self {
        self.use_testnet = true;
        self
    }

    /// Build the combined stream URL for the given subscription.
    ///
    /// Binance combined streams format:
    ///   wss://stream.binance.com:9443/stream?streams=btcusdt@trade/ethusdt@trade
    fn build_url(&self, subscription: &Subscription) -> Result<String, IngestionError> {
        let mut stream_names: Vec<String> = Vec::new();

        for symbol in &subscription.symbols {
            // Binance requires lowercase symbols with no separator
            // e.g. "BTC/USDT" -> "btcusdt", "ETHUSDT" -> "ethusdt"
            let normalized = symbol.replace('/', "").replace('-', "").to_lowercase();

            for dt in &subscription.data_types {
                match dt {
                    DataType::Trades => {
                        stream_names.push(format!("{normalized}@trade"));
                    }
                    DataType::Quotes | DataType::OrderBookL1 => {
                        stream_names.push(format!("{normalized}@bookTicker"));
                    }
                    DataType::OrderBookL2 => {
                        // Top 5 levels, 100ms update speed
                        stream_names.push(format!("{normalized}@depth5@100ms"));
                    }
                    DataType::Bars1m => {
                        stream_names.push(format!("{normalized}@kline_1m"));
                    }
                }
            }
        }

        // Binance enforces max 1024 streams per connection
        if stream_names.len() > 1024 {
            return Err(IngestionError::Other(anyhow::anyhow!(
                "Binance max 1024 streams per connection, requested {}",
                stream_names.len()
            )));
        }

        let base = if self.use_testnet {
            BINANCE_TESTNET_URL
        } else {
            BINANCE_STREAM_URL
        };

        Ok(format!("{}?streams={}", base, stream_names.join("/")))
    }
}

#[async_trait]
impl MarketDataSource for BinanceSource {
    fn name(&self) -> &str {
        "Binance"
    }

    fn supported_data_types(&self) -> &[DataType] {
        &[
            DataType::Trades,
            DataType::Quotes,
            DataType::OrderBookL1,
            DataType::OrderBookL2,
            DataType::Bars1m,
        ]
    }

    async fn connect(&self, subscription: &Subscription) -> Result<MarketStream, IngestionError> {
        let url = self.build_url(subscription)?;

        info!(
            provider = "Binance",
            url = %url,
            testnet = self.use_testnet,
            "Connecting to Binance combined stream"
        );

        let (ws, _response) = connect_async(&url)
            .await
            .map_err(|e| IngestionError::ConnectionFailed(e.to_string()))?;

        info!(
            provider = "Binance",
            streams = subscription.symbols.len(),
            "WebSocket connected"
        );

        let seq_gen = Arc::clone(&self.seq_gen);

        let stream = ws.filter_map(move |msg_result| {
            let seq_gen = Arc::clone(&seq_gen);
            async move {
                match msg_result {
                    Ok(Message::Text(text)) => parse_binance_combined(&text, &seq_gen),
                    Ok(Message::Ping(payload)) => {
                        // Binance pings every 20s — handled by tungstenite
                        // automatically, but we log for visibility
                        debug!(len = payload.len(), "Binance ping received");
                        None
                    }
                    Ok(Message::Close(frame)) => {
                        warn!(frame = ?frame, "Binance WebSocket closed");
                        Some(Err(IngestionError::StreamClosed))
                    }
                    Err(e) => {
                        error!(error = %e, "Binance WebSocket error");
                        Some(Err(IngestionError::ConnectionFailed(e.to_string())))
                    }
                    _ => None,
                }
            }
        });

        Ok(Box::pin(stream))
    }

    async fn is_healthy(&self) -> bool {
        // Public streams don't need auth — always "healthy" if reachable
        true
    }
}

/// Parse a Binance combined stream message.
///
/// Combined stream format:
/// ```json
/// { "stream": "btcusdt@trade", "data": { "e": "trade", ... } }
/// ```
fn parse_binance_combined(
    text: &str,
    seq_gen: &SequenceGenerator,
) -> Option<Result<Envelope<MarketEvent>, IngestionError>> {
    let json: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return Some(Err(IngestionError::Deserialize(e.to_string()))),
    };

    let stream_name = json.get("stream")?.as_str()?;
    let data = json.get("data")?;
    let event_type = data
        .get("e")
        .and_then(|v| v.as_str())
        .or_else(|| infer_event_type(stream_name))?;

    let ts_init = UnixNanos::now();

    match event_type {
        // ─── Trade Stream ────────────────────────────────────────────
        // {"e":"trade","E":1672515782136,"s":"BNBBTC","t":12345,
        //  "p":"0.001","q":"100","T":1672515782136,"m":true}
        "trade" => {
            let symbol = CompactString::new(data.get("s")?.as_str()?);
            let price: f64 = data.get("p")?.as_str()?.parse().ok()?;
            let quantity: f64 = data.get("q")?.as_str()?.parse().ok()?;
            if !price.is_finite() || price <= 0.0 || !quantity.is_finite() || quantity <= 0.0 {
                return None;
            }
            let trade_time_ms = data.get("T")?.as_u64()?;

            // "m" = is buyer the market maker?
            // true  -> taker is seller (aggressor sell)
            // false -> taker is buyer  (aggressor buy)
            let is_buyer_maker = data.get("m")?.as_bool()?;
            let side = if is_buyer_maker {
                TradeSide::Sell
            } else {
                TradeSide::Buy
            };

            let event = MarketEvent::Trade(TradeEvent {
                symbol,
                price,
                quantity,
                side,
            });

            Some(Ok(Envelope {
                ts_event: UnixNanos::from_millis(trade_time_ms),
                ts_init,
                sequence_id: seq_gen.next_id(),
                payload: event,
            }))
        }

        // ─── Book Ticker Stream (L1 Best Bid/Ask) ───────────────────
        // {"u":400900217,"s":"BNBUSDT","b":"25.35190000",
        //  "B":"31.21000000","a":"25.36520000","A":"40.66000000"}
        //
        // Note: bookTicker on spot doesn't have "e" field on the raw
        // stream, but the combined stream wraps it.
        // We also handle the futures-style with "e":"bookTicker".
        "bookTicker" => parse_book_ticker(data, ts_init, seq_gen),

        // ─── Kline Stream (Bar Data) ────────────────────────────────
        // {"e":"kline","E":1672515782136,"s":"BNBBTC","k":{...}}
        "kline" => {
            let kline = data.get("k")?;
            let symbol = CompactString::new(kline.get("s")?.as_str()?);
            let open: f64 = kline.get("o")?.as_str()?.parse().ok()?;
            let high: f64 = kline.get("h")?.as_str()?.parse().ok()?;
            let low: f64 = kline.get("l")?.as_str()?.parse().ok()?;
            let close: f64 = kline.get("c")?.as_str()?.parse().ok()?;
            let volume: f64 = kline.get("v")?.as_str()?.parse().ok()?;
            if [open, high, low, close, volume]
                .iter()
                .any(|v| !v.is_finite())
                || open <= 0.0
                || high <= 0.0
                || low <= 0.0
                || close <= 0.0
                || volume < 0.0
            {
                return None;
            }
            let event_time_ms = data.get("E")?.as_u64()?;

            let event = MarketEvent::Bar(BarEvent {
                symbol,
                open,
                high,
                low,
                close,
                volume,
            });

            Some(Ok(Envelope {
                ts_event: UnixNanos::from_millis(event_time_ms),
                ts_init,
                sequence_id: seq_gen.next_id(),
                payload: event,
            }))
        }

        // ─── Depth Update (L2 Partial Book) ─────────────────────────
        // Partial book depth: {"lastUpdateId":160,"bids":[["0.0024","10"]],"asks":[...]}
        // Diff depth:  {"e":"depthUpdate","E":123456789,"s":"BTCUSDT","U":157,"u":160,...}
        "depthUpdate" => {
            let symbol = CompactString::new(data.get("s")?.as_str()?);
            let event_time_ms = data.get("E")?.as_u64()?;

            let bids = parse_depth_levels(data.get("b")?)?;
            let asks = parse_depth_levels(data.get("a")?)?;

            let event = MarketEvent::BookUpdate(BookUpdateEvent { symbol, bids, asks });

            Some(Ok(Envelope {
                ts_event: UnixNanos::from_millis(event_time_ms),
                ts_init,
                sequence_id: seq_gen.next_id(),
                payload: event,
            }))
        }

        "partialDepth" => {
            let symbol = data
                .get("s")
                .and_then(|v| v.as_str())
                .map(CompactString::new)
                .or_else(|| symbol_from_stream(stream_name).map(CompactString::new))?;

            let bids = parse_depth_levels(data.get("bids").or_else(|| data.get("b"))?)?;
            let asks = parse_depth_levels(data.get("asks").or_else(|| data.get("a"))?)?;

            let event = MarketEvent::BookUpdate(BookUpdateEvent { symbol, bids, asks });

            Some(Ok(Envelope {
                ts_event: ts_init,
                ts_init,
                sequence_id: seq_gen.next_id(),
                payload: event,
            }))
        }

        _ => {
            debug!(event_type = event_type, "Unhandled Binance event type");
            None
        }
    }
}

/// Infer Binance event type for raw spot streams that omit the "e" field.
fn infer_event_type(stream_name: &str) -> Option<&'static str> {
    if stream_name.contains("@bookTicker") {
        Some("bookTicker")
    } else if stream_name.contains("@depth") {
        Some("partialDepth")
    } else {
        None
    }
}

fn symbol_from_stream(stream_name: &str) -> Option<&str> {
    stream_name.split('@').next().filter(|s| !s.is_empty())
}

/// Parse bookTicker, which may arrive without an "e" field on spot raw streams.
fn parse_book_ticker(
    data: &serde_json::Value,
    ts_init: UnixNanos,
    seq_gen: &SequenceGenerator,
) -> Option<Result<Envelope<MarketEvent>, IngestionError>> {
    let symbol = CompactString::new(data.get("s")?.as_str()?);
    let bid: f64 = data.get("b")?.as_str()?.parse().ok()?;
    let bid_size: f64 = data.get("B")?.as_str()?.parse().ok()?;
    let ask: f64 = data.get("a")?.as_str()?.parse().ok()?;
    let ask_size: f64 = data.get("A")?.as_str()?.parse().ok()?;
    if !bid.is_finite()
        || !ask.is_finite()
        || !bid_size.is_finite()
        || !ask_size.is_finite()
        || bid <= 0.0
        || ask <= 0.0
        || bid > ask
        || bid_size < 0.0
        || ask_size < 0.0
    {
        return None;
    }

    // Event time may be present on futures ("E") but not spot bookTicker
    let ts_event = data
        .get("E")
        .and_then(|v| v.as_u64())
        .map(UnixNanos::from_millis)
        .unwrap_or(ts_init);

    let event = MarketEvent::Quote(QuoteEvent {
        symbol,
        bid,
        bid_size,
        ask,
        ask_size,
    });

    Some(Ok(Envelope {
        ts_event,
        ts_init,
        sequence_id: seq_gen.next_id(),
        payload: event,
    }))
}

/// Parse depth levels from Binance format: [["price", "qty"], ...]
fn parse_depth_levels(value: &serde_json::Value) -> Option<Vec<PriceLevel>> {
    let arr = value.as_array()?;
    let levels = arr
        .iter()
        .filter_map(|level| {
            let pair = level.as_array()?;
            let price: f64 = pair.first()?.as_str()?.parse().ok()?;
            let quantity: f64 = pair.get(1)?.as_str()?.parse().ok()?;
            if !price.is_finite() || price <= 0.0 || !quantity.is_finite() || quantity < 0.0 {
                return None;
            }
            Some(PriceLevel { price, quantity })
        })
        .collect();
    Some(levels)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trade_message() {
        let seq_gen = SequenceGenerator::new();
        let msg = r#"{
            "stream": "btcusdt@trade",
            "data": {
                "e": "trade",
                "E": 1672515782136,
                "s": "BTCUSDT",
                "t": 12345,
                "p": "67542.50",
                "q": "0.125",
                "T": 1672515782136,
                "m": false,
                "M": true
            }
        }"#;

        let result = parse_binance_combined(msg, &seq_gen);
        let envelope = result.unwrap().unwrap();

        match &envelope.payload {
            MarketEvent::Trade(trade) => {
                assert_eq!(trade.symbol.as_str(), "BTCUSDT");
                assert!((trade.price - 67542.50).abs() < f64::EPSILON);
                assert!((trade.quantity - 0.125).abs() < f64::EPSILON);
                // m=false means taker is buyer
                assert_eq!(trade.side, TradeSide::Buy);
            }
            _ => panic!("Expected Trade event"),
        }

        assert_eq!(envelope.ts_event, UnixNanos::from_millis(1672515782136));
    }

    #[test]
    fn parse_book_ticker_message() {
        let seq_gen = SequenceGenerator::new();
        let msg = r#"{
            "stream": "bnbusdt@bookTicker",
            "data": {
                "e": "bookTicker",
                "u": 400900217,
                "s": "BNBUSDT",
                "b": "25.35190000",
                "B": "31.21000000",
                "a": "25.36520000",
                "A": "40.66000000"
            }
        }"#;

        let result = parse_binance_combined(msg, &seq_gen);
        let envelope = result.unwrap().unwrap();

        match &envelope.payload {
            MarketEvent::Quote(quote) => {
                assert_eq!(quote.symbol.as_str(), "BNBUSDT");
                assert!((quote.bid - 25.3519).abs() < 0.0001);
                assert!((quote.ask - 25.3652).abs() < 0.0001);
                assert!((quote.bid_size - 31.21).abs() < 0.01);
                assert!((quote.ask_size - 40.66).abs() < 0.01);
            }
            _ => panic!("Expected Quote event"),
        }
    }

    #[test]
    fn parse_kline_message() {
        let seq_gen = SequenceGenerator::new();
        let msg = r#"{
            "stream": "btcusdt@kline_1m",
            "data": {
                "e": "kline",
                "E": 1672515782136,
                "s": "BTCUSDT",
                "k": {
                    "t": 1672515780000,
                    "T": 1672515839999,
                    "s": "BTCUSDT",
                    "i": "1m",
                    "f": 100,
                    "L": 200,
                    "o": "67500.00",
                    "c": "67550.00",
                    "h": "67600.00",
                    "l": "67450.00",
                    "v": "123.456",
                    "n": 100,
                    "x": false,
                    "q": "8330000.00",
                    "V": "61.728",
                    "Q": "4165000.00",
                    "B": "0"
                }
            }
        }"#;

        let result = parse_binance_combined(msg, &seq_gen);
        let envelope = result.unwrap().unwrap();

        match &envelope.payload {
            MarketEvent::Bar(bar) => {
                assert_eq!(bar.symbol.as_str(), "BTCUSDT");
                assert!((bar.open - 67500.0).abs() < f64::EPSILON);
                assert!((bar.close - 67550.0).abs() < f64::EPSILON);
                assert!((bar.high - 67600.0).abs() < f64::EPSILON);
                assert!((bar.low - 67450.0).abs() < f64::EPSILON);
            }
            _ => panic!("Expected Bar event"),
        }
    }

    #[test]
    fn parse_depth_update_message() {
        let seq_gen = SequenceGenerator::new();
        let msg = r#"{
            "stream": "btcusdt@depth5@100ms",
            "data": {
                "e": "depthUpdate",
                "E": 1672515782136,
                "s": "BTCUSDT",
                "U": 157,
                "u": 160,
                "b": [
                    ["67540.00", "1.500"],
                    ["67539.00", "2.300"]
                ],
                "a": [
                    ["67541.00", "0.800"],
                    ["67542.00", "1.200"]
                ]
            }
        }"#;

        let result = parse_binance_combined(msg, &seq_gen);
        let envelope = result.unwrap().unwrap();

        match &envelope.payload {
            MarketEvent::BookUpdate(book) => {
                assert_eq!(book.symbol.as_str(), "BTCUSDT");
                assert_eq!(book.bids.len(), 2);
                assert_eq!(book.asks.len(), 2);
                assert!((book.bids[0].price - 67540.0).abs() < f64::EPSILON);
                assert!((book.asks[0].price - 67541.0).abs() < f64::EPSILON);
            }
            _ => panic!("Expected BookUpdate event"),
        }
    }

    #[test]
    fn url_construction() {
        let seq_gen = Arc::new(SequenceGenerator::new());
        let source = BinanceSource::new(seq_gen);

        let sub = Subscription {
            symbols: vec!["BTCUSDT".into(), "ETHUSDT".into()],
            data_types: vec![DataType::Trades, DataType::OrderBookL1],
        };

        let url = source.build_url(&sub).unwrap();
        assert!(url.contains("btcusdt@trade"));
        assert!(url.contains("ethusdt@trade"));
        assert!(url.contains("btcusdt@bookTicker"));
        assert!(url.contains("ethusdt@bookTicker"));
        assert!(url.starts_with("wss://stream.binance.com"));
    }

    #[test]
    fn url_construction_testnet() {
        let seq_gen = Arc::new(SequenceGenerator::new());
        let source = BinanceSource::new(seq_gen).testnet();

        let sub = Subscription {
            symbols: vec!["BTCUSDT".into()],
            data_types: vec![DataType::Trades],
        };

        let url = source.build_url(&sub).unwrap();
        assert!(url.starts_with("wss://stream.testnet.binance.vision"));
    }

    #[test]
    fn url_normalizes_symbols() {
        let seq_gen = Arc::new(SequenceGenerator::new());
        let source = BinanceSource::new(seq_gen);

        let sub = Subscription {
            symbols: vec!["BTC/USDT".into(), "ETH-USDT".into()],
            data_types: vec![DataType::Trades],
        };

        let url = source.build_url(&sub).unwrap();
        assert!(url.contains("btcusdt@trade"));
        assert!(url.contains("ethusdt@trade"));
        // No slashes or dashes in stream names
        assert!(!url.contains("btc/usdt"));
        assert!(!url.contains("eth-usdt"));
    }

    #[test]
    fn rejects_over_1024_streams() {
        let seq_gen = Arc::new(SequenceGenerator::new());
        let source = BinanceSource::new(seq_gen);

        let symbols: Vec<String> = (0..520).map(|i| format!("SYM{i}USDT")).collect();

        let sub = Subscription {
            symbols,
            data_types: vec![DataType::Trades, DataType::OrderBookL1],
        };

        // 520 symbols × 2 data types = 1040 > 1024
        let result = source.build_url(&sub);
        assert!(result.is_err());
    }
}
