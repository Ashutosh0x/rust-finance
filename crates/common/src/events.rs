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
    pub fn new(
        ts_event: UnixNanos,
        ts_init: UnixNanos,
        sequence_id: SequenceId,
        payload: T,
    ) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: postcard roundtrip
    fn roundtrip_bot_event(event: &BotEvent) {
        let bytes = postcard::to_allocvec(event).expect("serialize failed");
        let decoded: BotEvent = postcard::from_bytes(&bytes).expect("deserialize failed");
        // Verify field-level equality via debug strings (BotEvent doesn't derive PartialEq)
        assert_eq!(format!("{:?}", event), format!("{:?}", decoded));
    }

    fn roundtrip_control(cmd: &ControlCommand) {
        let bytes = postcard::to_allocvec(cmd).expect("serialize failed");
        let decoded: ControlCommand = postcard::from_bytes(&bytes).expect("deserialize failed");
        assert_eq!(format!("{:?}", cmd), format!("{:?}", decoded));
    }

    #[test]
    fn test_postcard_roundtrip_market_event() {
        roundtrip_bot_event(&BotEvent::MarketEvent {
            symbol: "NVDA".into(),
            price: 912.50,
            volume: Some(1_500_000.0),
            event_type: "trade".into(),
        });
    }

    #[test]
    fn test_postcard_roundtrip_market_event_no_volume() {
        roundtrip_bot_event(&BotEvent::MarketEvent {
            symbol: "AAPL".into(),
            price: 178.25,
            volume: None,
            event_type: "bar".into(),
        });
    }

    #[test]
    fn test_postcard_roundtrip_quote_event() {
        roundtrip_bot_event(&BotEvent::QuoteEvent {
            symbol: "TSLA".into(),
            bid_price: 180.00,
            bid_size: 500,
            ask_price: 180.05,
            ask_size: 300,
        });
    }

    #[test]
    fn test_postcard_roundtrip_feed() {
        roundtrip_bot_event(&BotEvent::Feed("raw_message_payload_bytes".into()));
    }

    #[test]
    fn test_postcard_roundtrip_ai_signal() {
        roundtrip_bot_event(&BotEvent::AISignal {
            symbol: "NVDA".into(),
            action: "BUY".into(),
            confidence: 0.87,
            reason: "GARCH vol below threshold, swarm 72% bullish".into(),
        });
    }

    #[test]
    fn test_postcard_roundtrip_position_update() {
        roundtrip_bot_event(&BotEvent::PositionUpdate {
            token: "SOL".into(),
            size: 42.5,
        });
    }

    #[test]
    fn test_postcard_roundtrip_wallet_update() {
        roundtrip_bot_event(&BotEvent::WalletUpdate {
            sol_balance: 98.123456,
        });
    }

    #[test]
    fn test_postcard_roundtrip_exchange_heartbeat() {
        roundtrip_bot_event(&BotEvent::ExchangeHeartbeat {
            exchange: "Alpaca".into(),
            status: "connected".into(),
            latency_ms: 12.5,
        });
    }

    #[test]
    fn test_postcard_roundtrip_trade_signal() {
        roundtrip_bot_event(&BotEvent::TradeSignal("BUY NVDA 0.04 conf=0.82".into()));
    }

    #[test]
    fn test_postcard_roundtrip_heartbeat() {
        roundtrip_bot_event(&BotEvent::Heartbeat);
    }

    #[test]
    fn test_postcard_roundtrip_all_control_commands() {
        roundtrip_control(&ControlCommand::Pause);
        roundtrip_control(&ControlCommand::Resume);
        roundtrip_control(&ControlCommand::KillSwitch);
        roundtrip_control(&ControlCommand::ToggleMode);
        roundtrip_control(&ControlCommand::CloseAllPositions);
        roundtrip_control(&ControlCommand::AdjustRisk { delta: -0.25 });
        roundtrip_control(&ControlCommand::Shutdown);
    }

    #[test]
    fn test_postcard_roundtrip_extreme_values() {
        // Test with edge-case float values
        roundtrip_bot_event(&BotEvent::MarketEvent {
            symbol: "".into(), // empty symbol
            price: f64::MAX,
            volume: Some(f64::MIN_POSITIVE),
            event_type: "edge".into(),
        });
        roundtrip_bot_event(&BotEvent::WalletUpdate { sol_balance: 0.0 });
        roundtrip_bot_event(&BotEvent::QuoteEvent {
            symbol: "X".into(),
            bid_price: 0.0001,
            bid_size: u64::MAX,
            ask_price: 999999.99,
            ask_size: 0,
        });
    }

    #[test]
    fn test_postcard_deterministic_encoding() {
        let event = BotEvent::AISignal {
            symbol: "NVDA".into(),
            action: "BUY".into(),
            confidence: 0.75,
            reason: "test".into(),
        };
        let bytes1 = postcard::to_allocvec(&event).unwrap();
        let bytes2 = postcard::to_allocvec(&event).unwrap();
        assert_eq!(bytes1, bytes2, "Postcard encoding must be deterministic");
    }

    /// Malformed bytes must return Err, never panic.
    /// This is critical for a binary protocol on a TCP socket — flaky networks
    /// or version mismatches produce garbage frames.
    #[test]
    fn test_postcard_malformed_bytes_dont_panic() {
        // Completely random garbage
        let garbage: &[u8] = &[0xFF, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x42];
        let result = postcard::from_bytes::<BotEvent>(garbage);
        assert!(result.is_err(), "Garbage bytes must return Err");

        // Empty payload
        let empty: &[u8] = &[];
        let result = postcard::from_bytes::<BotEvent>(empty);
        assert!(result.is_err(), "Empty bytes must return Err");

        // Truncated valid-looking payload (valid variant tag, but not enough data)
        let truncated: &[u8] = &[0x00, 0x04]; // MarketEvent variant with partial data
        let result = postcard::from_bytes::<BotEvent>(truncated);
        assert!(result.is_err(), "Truncated bytes must return Err");

        // All-zeros
        let zeros: &[u8] = &[0x00; 64];
        let result = postcard::from_bytes::<BotEvent>(zeros);
        // May succeed or fail — the point is it doesn't panic
        let _ = result;

        // Repeated variant for ControlCommand
        let garbage_cmd: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF];
        let result = postcard::from_bytes::<ControlCommand>(garbage_cmd);
        assert!(result.is_err(), "Invalid variant tag must return Err");
    }

    /// Verify postcard handles the full spectrum of Option<f64> values correctly.
    /// Optional fields and edge-case floats are where Postcard is most likely to silently mishandle.
    #[test]
    fn test_postcard_optional_field_edge_cases() {
        // volume = Some(0.0) — edge case float
        roundtrip_bot_event(&BotEvent::MarketEvent {
            symbol: "AAPL".into(),
            price: 150.0,
            volume: Some(0.0),
            event_type: "trade".into(),
        });

        // volume = Some(f64::INFINITY)
        roundtrip_bot_event(&BotEvent::MarketEvent {
            symbol: "AAPL".into(),
            price: 150.0,
            volume: Some(f64::INFINITY),
            event_type: "trade".into(),
        });

        // volume = Some(f64::NEG_INFINITY)
        roundtrip_bot_event(&BotEvent::MarketEvent {
            symbol: "AAPL".into(),
            price: 150.0,
            volume: Some(f64::NEG_INFINITY),
            event_type: "trade".into(),
        });

        // volume = Some(f64::NAN) — NaN != NaN, so compare via is_nan
        let event = BotEvent::MarketEvent {
            symbol: "X".into(),
            price: 1.0,
            volume: Some(f64::NAN),
            event_type: "nan_test".into(),
        };
        let bytes = postcard::to_allocvec(&event).expect("NaN serialize");
        let decoded: BotEvent = postcard::from_bytes(&bytes).expect("NaN deserialize");
        if let BotEvent::MarketEvent {
            volume: Some(v), ..
        } = decoded
        {
            assert!(v.is_nan(), "NaN should survive roundtrip");
        } else {
            panic!("Wrong variant after roundtrip");
        }
    }

    /// Test all BotEvent variants in a single sweep to ensure nothing is missed.
    #[test]
    fn test_postcard_roundtrip_all_variants_exhaustive() {
        let variants: Vec<BotEvent> = vec![
            BotEvent::MarketEvent {
                symbol: "A".into(),
                price: 1.0,
                volume: None,
                event_type: "t".into(),
            },
            BotEvent::QuoteEvent {
                symbol: "B".into(),
                bid_price: 1.0,
                bid_size: 1,
                ask_price: 2.0,
                ask_size: 2,
            },
            BotEvent::Feed("raw".into()),
            BotEvent::AISignal {
                symbol: "C".into(),
                action: "BUY".into(),
                confidence: 0.5,
                reason: "r".into(),
            },
            BotEvent::PositionUpdate {
                token: "SOL".into(),
                size: 1.0,
            },
            BotEvent::WalletUpdate { sol_balance: 0.0 },
            BotEvent::ExchangeHeartbeat {
                exchange: "E".into(),
                status: "ok".into(),
                latency_ms: 0.1,
            },
            BotEvent::TradeSignal("sig".into()),
            BotEvent::Heartbeat,
        ];
        for event in &variants {
            roundtrip_bot_event(event);
        }
        assert_eq!(variants.len(), 9, "All 9 BotEvent variants must be tested");
    }
}
