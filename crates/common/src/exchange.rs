// crates/common/src/exchange.rs
//
// Multi-Exchange Abstraction Layer
//
// Inspired by barter-rs's IndexedMultiExchangeMarketStream pattern:
//   - All venues produce the SAME normalized types
//   - Each instrument gets a pre-assigned numeric index for O(1) lookup
//   - Exchange-specific details are abstracted behind a trait
//
// This closes the gap identified in the competitive analysis where
// RustForge only supports Binance, while NautilusTrader has 40+ adapters
// and barter-rs has its multi-exchange normalization layer.

use compact_str::CompactString;

// ── Exchange Identifiers ─────────────────────────────────────────

/// Supported exchange venues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Exchange {
    Binance,
    BinanceUs,
    Coinbase,
    Alpaca,
    Okx,
    Bybit,
    Kraken,
    /// For paper/simulation
    Simulated,
}

impl Exchange {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Binance => "binance",
            Self::BinanceUs => "binance_us",
            Self::Coinbase => "coinbase",
            Self::Alpaca => "alpaca",
            Self::Okx => "okx",
            Self::Bybit => "bybit",
            Self::Kraken => "kraken",
            Self::Simulated => "simulated",
        }
    }
}

impl std::fmt::Display for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ── Instrument ID ────────────────────────────────────────────────

/// Globally unique instrument identifier: (exchange, symbol).
/// Enables cross-exchange arbitrage detection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstrumentId {
    pub exchange: Exchange,
    pub symbol: CompactString,
}

impl InstrumentId {
    pub fn new(exchange: Exchange, symbol: &str) -> Self {
        Self {
            exchange,
            symbol: CompactString::new(symbol),
        }
    }
}

impl std::fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.exchange, self.symbol)
    }
}

// ── Normalized Market Data ───────────────────────────────────────

/// Normalized trade event — identical structure regardless of source venue.
/// This is the barter-rs pattern: all exchanges produce the same type.
#[derive(Debug, Clone)]
pub struct NormalizedTrade {
    pub instrument: InstrumentId,
    pub price: f64,
    pub quantity: f64,
    pub side: NormalizedSide,
    /// Exchange timestamp in milliseconds since epoch.
    pub exchange_ts_ms: i64,
    /// Local receive timestamp in milliseconds since epoch.
    pub local_ts_ms: i64,
    /// Exchange-specific trade ID (for dedup).
    pub trade_id: CompactString,
}

/// Normalized order book snapshot.
#[derive(Debug, Clone)]
pub struct NormalizedBook {
    pub instrument: InstrumentId,
    pub bids: Vec<NormalizedLevel>,
    pub asks: Vec<NormalizedLevel>,
    pub exchange_ts_ms: i64,
    pub local_ts_ms: i64,
    /// Sequence number for gap detection.
    pub sequence: u64,
}

/// A single price level in the order book.
#[derive(Debug, Clone, Copy)]
pub struct NormalizedLevel {
    pub price: f64,
    pub quantity: f64,
}

/// Normalized quote (BBO — Best Bid/Offer).
#[derive(Debug, Clone)]
pub struct NormalizedQuote {
    pub instrument: InstrumentId,
    pub bid: f64,
    pub bid_size: f64,
    pub ask: f64,
    pub ask_size: f64,
    pub exchange_ts_ms: i64,
}

/// Normalized funding rate event (perpetual futures).
#[derive(Debug, Clone)]
pub struct NormalizedFundingRate {
    pub instrument: InstrumentId,
    pub rate: f64,
    pub next_funding_ts_ms: i64,
}

/// Trade side (buyer/seller aggressor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedSide {
    Buy,
    Sell,
    Unknown,
}

/// Top-level normalized market event enum.
/// All exchange adapters produce this single type.
#[derive(Debug, Clone)]
pub enum NormalizedMarketEvent {
    Trade(NormalizedTrade),
    Book(NormalizedBook),
    Quote(NormalizedQuote),
    FundingRate(NormalizedFundingRate),
}

impl NormalizedMarketEvent {
    pub fn instrument(&self) -> &InstrumentId {
        match self {
            Self::Trade(e) => &e.instrument,
            Self::Book(e) => &e.instrument,
            Self::Quote(e) => &e.instrument,
            Self::FundingRate(e) => &e.instrument,
        }
    }

    pub fn exchange(&self) -> Exchange {
        self.instrument().exchange
    }

    pub fn symbol(&self) -> &str {
        self.instrument().symbol.as_str()
    }
}

// ── Indexed Instrument Registry ──────────────────────────────────

/// Maps InstrumentId → contiguous usize index for O(1) hot-path lookups.
/// Inspired by barter-rs's data-oriented state pattern.
///
/// Usage:
/// ```ignore
/// let mut registry = InstrumentRegistry::new();
/// let btc_idx = registry.register(InstrumentId::new(Exchange::Binance, "BTCUSDT"));
/// let eth_idx = registry.register(InstrumentId::new(Exchange::Binance, "ETHUSDT"));
///
/// // In hot path: O(1) lookup
/// prices[btc_idx] = 60000.0;
/// signals[eth_idx] = compute_signal(eth_idx);
/// ```
pub struct InstrumentRegistry {
    instruments: Vec<InstrumentId>,
    index_map: std::collections::HashMap<InstrumentId, usize>,
}

impl InstrumentRegistry {
    pub fn new() -> Self {
        Self {
            instruments: Vec::new(),
            index_map: std::collections::HashMap::new(),
        }
    }

    /// Register an instrument and return its index.
    /// If already registered, returns the existing index.
    pub fn register(&mut self, instrument: InstrumentId) -> usize {
        if let Some(&idx) = self.index_map.get(&instrument) {
            return idx;
        }
        let idx = self.instruments.len();
        self.index_map.insert(instrument.clone(), idx);
        self.instruments.push(instrument);
        idx
    }

    /// Look up index for an instrument. O(1) via HashMap.
    pub fn get_index(&self, instrument: &InstrumentId) -> Option<usize> {
        self.index_map.get(instrument).copied()
    }

    /// Look up instrument by index. O(1) via Vec.
    pub fn get_instrument(&self, index: usize) -> Option<&InstrumentId> {
        self.instruments.get(index)
    }

    /// Total number of registered instruments.
    pub fn len(&self) -> usize {
        self.instruments.len()
    }

    /// Is the registry empty?
    pub fn is_empty(&self) -> bool {
        self.instruments.is_empty()
    }

    /// Iterator over all registered instruments with their indices.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &InstrumentId)> {
        self.instruments.iter().enumerate()
    }
}

impl Default for InstrumentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instrument_id_display() {
        let id = InstrumentId::new(Exchange::Binance, "BTCUSDT");
        assert_eq!(id.to_string(), "binance:BTCUSDT");
    }

    #[test]
    fn test_instrument_id_equality() {
        let a = InstrumentId::new(Exchange::Binance, "BTCUSDT");
        let b = InstrumentId::new(Exchange::Binance, "BTCUSDT");
        let c = InstrumentId::new(Exchange::Coinbase, "BTC-USD");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_registry_register_and_lookup() {
        let mut reg = InstrumentRegistry::new();
        let idx0 = reg.register(InstrumentId::new(Exchange::Binance, "BTCUSDT"));
        let idx1 = reg.register(InstrumentId::new(Exchange::Binance, "ETHUSDT"));
        let idx2 = reg.register(InstrumentId::new(Exchange::Coinbase, "BTC-USD"));

        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);
        assert_eq!(reg.len(), 3);
    }

    #[test]
    fn test_registry_idempotent() {
        let mut reg = InstrumentRegistry::new();
        let id = InstrumentId::new(Exchange::Binance, "BTCUSDT");
        let idx1 = reg.register(id.clone());
        let idx2 = reg.register(id);
        assert_eq!(idx1, idx2, "Re-registering should return same index");
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_registry_reverse_lookup() {
        let mut reg = InstrumentRegistry::new();
        let id = InstrumentId::new(Exchange::Okx, "BTC-USDT");
        let idx = reg.register(id.clone());

        let found = reg.get_instrument(idx).unwrap();
        assert_eq!(*found, id);
    }

    #[test]
    fn test_normalized_market_event_accessors() {
        let trade = NormalizedMarketEvent::Trade(NormalizedTrade {
            instrument: InstrumentId::new(Exchange::Binance, "BTCUSDT"),
            price: 60000.0,
            quantity: 0.5,
            side: NormalizedSide::Buy,
            exchange_ts_ms: 1000,
            local_ts_ms: 1001,
            trade_id: CompactString::new("12345"),
        });

        assert_eq!(trade.exchange(), Exchange::Binance);
        assert_eq!(trade.symbol(), "BTCUSDT");
    }

    #[test]
    fn test_normalized_book_event() {
        let book = NormalizedMarketEvent::Book(NormalizedBook {
            instrument: InstrumentId::new(Exchange::Coinbase, "ETH-USD"),
            bids: vec![
                NormalizedLevel { price: 3000.0, quantity: 10.0 },
                NormalizedLevel { price: 2999.0, quantity: 20.0 },
            ],
            asks: vec![
                NormalizedLevel { price: 3001.0, quantity: 5.0 },
                NormalizedLevel { price: 3002.0, quantity: 15.0 },
            ],
            exchange_ts_ms: 2000,
            local_ts_ms: 2001,
            sequence: 42,
        });

        assert_eq!(book.symbol(), "ETH-USD");
        assert_eq!(book.exchange(), Exchange::Coinbase);
    }

    #[test]
    fn test_cross_exchange_instruments() {
        let mut reg = InstrumentRegistry::new();

        // Same asset on different exchanges
        let btc_binance = reg.register(InstrumentId::new(Exchange::Binance, "BTCUSDT"));
        let btc_coinbase = reg.register(InstrumentId::new(Exchange::Coinbase, "BTC-USD"));
        let btc_okx = reg.register(InstrumentId::new(Exchange::Okx, "BTC-USDT"));

        // Each gets a unique index
        assert_ne!(btc_binance, btc_coinbase);
        assert_ne!(btc_coinbase, btc_okx);
        assert_eq!(reg.len(), 3);

        // Can look up each independently
        assert!(reg.get_index(&InstrumentId::new(Exchange::Binance, "BTCUSDT")).is_some());
        assert!(reg.get_index(&InstrumentId::new(Exchange::Bybit, "BTCUSDT")).is_none());
    }
}
