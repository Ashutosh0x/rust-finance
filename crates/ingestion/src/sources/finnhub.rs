use crate::source::{DataType, IngestionError, MarketDataSource, MarketStream, Subscription};
use async_trait::async_trait;
use common::events::{Envelope, MarketEvent, TradeEvent, TradeSide};
use common::time::{SequenceGenerator, UnixNanos};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

/// Finnhub live data source — supports US, NSE (India), BSE (India), LSE, etc.
///
/// Symbol format for international exchanges:
/// - US stocks: "AAPL", "MSFT"
/// - NSE (India): "NSE:RELIANCE", "NSE:TCS", "NSE:INFY"
/// - BSE (India): "BSE:500325", "BSE:532540"
/// - LSE (London): "LSE:VOD", "LSE:HSBA"
/// - Crypto (Binance): "BINANCE:BTCUSDT"
#[derive(Clone)]
pub struct FinnhubSource {
    api_key: String,
    seq_gen: Arc<SequenceGenerator>,
}

impl FinnhubSource {
    pub fn from_env(seq_gen: Arc<SequenceGenerator>) -> Result<Self, IngestionError> {
        let api_key = std::env::var("FINNHUB_API_KEY")
            .map_err(|_| IngestionError::ConnectionFailed("FINNHUB_API_KEY not set".into()))?;

        Ok(Self { api_key, seq_gen })
    }

    /// Create from explicit key
    pub fn new(api_key: String, seq_gen: Arc<SequenceGenerator>) -> Self {
        Self { api_key, seq_gen }
    }
}

#[async_trait]
impl MarketDataSource for FinnhubSource {
    fn name(&self) -> &str {
        "Finnhub"
    }

    fn supported_data_types(&self) -> &[DataType] {
        &[DataType::Trades, DataType::Quotes]
    }

    async fn connect(&self, sub: &Subscription) -> Result<MarketStream, IngestionError> {
        let url = format!("wss://ws.finnhub.io?token={}", self.api_key);
        info!(
            "Connecting to Finnhub WebSocket for {} symbols...",
            sub.symbols.len()
        );

        let (ws_stream, _) = connect_async(&url)
            .await
            .map_err(|e| IngestionError::ConnectionFailed(format!("Finnhub WS: {}", e)))?;

        info!("Finnhub WS connected.");
        let (mut write, mut read) = ws_stream.split();

        // Subscribe to all symbols (supports NSE:, BSE:, etc. prefixes)
        for symbol in &sub.symbols {
            let msg = format!(r#"{{"type":"subscribe","symbol":"{}"}}"#, symbol);
            write.send(Message::Text(msg)).await.map_err(|e| {
                IngestionError::ConnectionFailed(format!("Subscribe failed: {}", e))
            })?;
            info!("Finnhub subscribed: {}", symbol);
        }

        let seq_gen = self.seq_gen.clone();

        // Transform WS messages into MarketEvent envelopes
        let stream = async_stream::stream! {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        for event in parse_finnhub_message(&text, &seq_gen) {
                            yield event;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error!("Finnhub WS error: {:?}", e);
                        yield Err(IngestionError::StreamClosed);
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn is_healthy(&self) -> bool {
        !self.api_key.is_empty()
    }
}

/// Parse a Finnhub WebSocket message into an Envelope<MarketEvent>
fn parse_finnhub_message(
    text: &str,
    seq_gen: &SequenceGenerator,
) -> Vec<Result<Envelope<MarketEvent>, IngestionError>> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) => return vec![Err(IngestionError::Deserialize(e.to_string()))],
    };

    let Some(msg_type) = value.get("type").and_then(|v| v.as_str()) else {
        return Vec::new();
    };

    if msg_type != "trade" {
        if msg_type == "ping" {
            return Vec::new();
        }
        warn!("Finnhub unknown message type: {}", msg_type);
        return Vec::new();
    }

    let Some(data) = value.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    let mut events = Vec::with_capacity(data.len());
    for trade in data {
        let Some(symbol) = trade.get("s").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(price) = trade.get("p").and_then(|v| v.as_f64()) else {
            continue;
        };
        if !price.is_finite() || price <= 0.0 {
            continue;
        }
        let volume = trade.get("v").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if !volume.is_finite() || volume < 0.0 {
            continue;
        }
        let Some(timestamp_ms) = trade.get("t").and_then(|v| v.as_i64()) else {
            continue;
        };
        if timestamp_ms < 0 {
            continue;
        }

        let ts_event = UnixNanos::from_millis(timestamp_ms as u64);
        let ts_init = UnixNanos::now();

        let event = MarketEvent::Trade(TradeEvent {
            symbol: symbol.into(),
            price,
            quantity: volume,
            side: TradeSide::Unknown,
        });

        let envelope = Envelope::new(ts_event, ts_init, seq_gen.next_id(), event);

        events.push(Ok(envelope));
    }

    events
}
