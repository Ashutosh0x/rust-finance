//! Core event types with nanosecond timestamps and sequence IDs.
//! Every event that flows through the system is stamped for deterministic replay.

use crate::time::{SequenceId, UnixNanos};
use compact_str::CompactString;

// ─── Event Envelope ──────────────────────────────────────────────────────────

/// Universal envelope wrapping every event in the system.
/// Provides total ordering via (ts_event, sequence_id).
#[derive(Debug, Clone)]
pub struct Envelope<T> {
    /// When the real-world event occurred (exchange timestamp).
    pub ts_event: UnixNanos,
    /// When this envelope was created locally.
    pub ts_init: UnixNanos,
    /// Monotonic sequence for deterministic ordering.
    pub sequence_id: SequenceId,
    /// The actual payload.
    pub payload: T,
}

impl<T> Envelope<T> {
    pub fn new(ts_event: UnixNanos, ts_init: UnixNanos, sequence_id: SequenceId, payload: T) -> Self {
        Self {
            ts_event,
            ts_init,
            sequence_id,
            payload,
        }
    }
}

impl<T> PartialEq for Envelope<T> {
    fn eq(&self, other: &Self) -> bool {
        self.sequence_id == other.sequence_id
    }
}

impl<T> Eq for Envelope<T> {}

impl<T> PartialOrd for Envelope<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Envelope<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ts_event
            .cmp(&other.ts_event)
            .then(self.sequence_id.cmp(&other.sequence_id))
    }
}

// ─── Market Events ───────────────────────────────────────────────────────────

/// Normalized market data event. Zero-allocation on hot path via CompactString.
#[derive(Debug, Clone)]
pub enum MarketEvent {
    Trade(TradeEvent),
    Quote(QuoteEvent),
    BookUpdate(BookUpdateEvent),
    Bar(BarEvent),
}

impl MarketEvent {
    pub fn symbol(&self) -> &str {
        match self {
            Self::Trade(e) => e.symbol.as_str(),
            Self::Quote(e) => e.symbol.as_str(),
            Self::BookUpdate(e) => e.symbol.as_str(),
            Self::Bar(e) => e.symbol.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradeEvent {
    pub symbol: CompactString,
    pub price: f64,
    pub quantity: f64,
    pub side: TradeSide,
}

#[derive(Debug, Clone)]
pub struct QuoteEvent {
    pub symbol: CompactString,
    pub bid: f64,
    pub bid_size: f64,
    pub ask: f64,
    pub ask_size: f64,
}

#[derive(Debug, Clone)]
pub struct BookUpdateEvent {
    pub symbol: CompactString,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
}

#[derive(Debug, Clone)]
pub struct BarEvent {
    pub symbol: CompactString,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct PriceLevel {
    pub price: f64,
    pub quantity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeSide {
    Buy,
    Sell,
    Unknown,
}

// ─── Order/Execution Events ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum OrderEvent {
    Submitted(OrderSubmitted),
    Accepted(OrderAccepted),
    Filled(OrderFilled),
    Rejected(OrderRejected),
    Cancelled(OrderCancelled),
}

#[derive(Debug, Clone)]
pub struct OrderSubmitted {
    pub client_order_id: CompactString,
    pub symbol: CompactString,
    pub side: OrderSide,
    pub quantity: f64,
    pub order_type: OrderType,
    pub limit_price: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct OrderAccepted {
    pub client_order_id: CompactString,
    pub venue_order_id: CompactString,
}

#[derive(Debug, Clone)]
pub struct OrderFilled {
    pub client_order_id: CompactString,
    pub fill_price: f64,
    pub fill_quantity: f64,
    pub commission: f64,
}

#[derive(Debug, Clone)]
pub struct OrderRejected {
    pub client_order_id: CompactString,
    pub reason: CompactString,
}

#[derive(Debug, Clone)]
pub struct OrderCancelled {
    pub client_order_id: CompactString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    Market,
    Limit,
}

// ─── Unified Engine Event ────────────────────────────────────────────────────

/// Top-level event enum for the engine. Everything flows through this.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    Market(MarketEvent),
    Order(OrderEvent),
    Signal(SignalEvent),
    System(SystemEvent),
}

#[derive(Debug, Clone)]
pub struct SignalEvent {
    pub symbol: CompactString,
    pub direction: OrderSide,
    pub confidence: f64,
    pub strategy_id: CompactString,
}

#[derive(Debug, Clone)]
pub enum SystemEvent {
    Heartbeat,
    Shutdown,
    ClockSync(UnixNanos),
}

// ─── Audit Trail ─────────────────────────────────────────────────────────────

/// Every state transition emits an AuditTick. Enables full system replay.
/// Inspired by Barter-rs EngineAudit pattern.
#[derive(Debug, Clone)]
pub struct AuditTick {
    pub ts: UnixNanos,
    pub sequence_id: SequenceId,
    pub event: AuditEvent,
}

#[derive(Debug, Clone)]
pub enum AuditEvent {
    MarketDataReceived {
        symbol: CompactString,
        source: CompactString,
    },
    OrderSubmitted {
        client_order_id: CompactString,
    },
    OrderFilled {
        client_order_id: CompactString,
        price: f64,
        quantity: f64,
    },
    RiskCheckPassed {
        client_order_id: CompactString,
    },
    RiskCheckBlocked {
        client_order_id: CompactString,
        reason: CompactString,
    },
    StrategySignal {
        strategy_id: CompactString,
        symbol: CompactString,
        direction: OrderSide,
        confidence: f64,
    },
    EngineShutdown,
}

// ─── Legacy v1 Event Types (backward compatibility) ──────────────────────────
// These types are used by event_bus, tui, ingestion (alpaca_ws/finnhub_ws),
// strategy, and daemon (hybrid_pipeline, ai_pipeline). They coexist with
// the v2 types above until migration is complete.

/// v1 event bus wire type. Serialized via postcard (daemon→TUI) and JSON (subscriber).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BotEvent {
    MarketEvent {
        symbol: String,
        price: f64,
        volume: Option<f64>,
        event_type: String,
    },
    QuoteEvent {
        symbol: String,
        bid_price: f64,
        bid_size: u64,
        ask_price: f64,
        ask_size: u64,
    },
    Feed(String),
    AISignal {
        symbol: String,
        action: String,
        confidence: f64,
        reason: String,
    },
    PositionUpdate {
        token: String,
        size: f64,
    },
    WalletUpdate {
        sol_balance: f64,
    },
    ExchangeHeartbeat {
        exchange: String,
        status: String,
        latency_ms: f64,
    },
    TradeSignal(String),
    Heartbeat,
}

/// v1 control commands sent from TUI to daemon via event bus.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ControlCommand {
    Pause,
    Resume,
    KillSwitch,
    ToggleMode,
    CloseAllPositions,
    AdjustRisk { delta: f64 },
    Shutdown,
}
