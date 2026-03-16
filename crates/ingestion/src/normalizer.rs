use common::events::BotEvent;
use serde_json::Value;

pub struct Normalizer;

impl Default for Normalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Normalizer {
    pub fn new() -> Self {
        Self
    }
    
    /// Parses raw JSON string messages into BotEvent
    pub fn normalize(raw: &str) -> Option<BotEvent> {
        let v: Value = serde_json::from_str(raw).ok()?;

        // If it's a Finnhub trade event
        if v["type"] == "trade" {
            if let Some(r_array) = v["data"].as_array() {
                if let Some(data) = r_array.first() {
                    let symbol = data["s"].as_str()?.to_string();
                    let price = data["p"].as_f64()?;
                    let timestamp = data["t"].as_i64()?;
                    let volume = data["v"].as_f64();
                    
                    return Some(BotEvent::MarketEvent {
                        symbol: symbol.into(),
                        price,
                        timestamp,
                        event_type: "trade".into(),
                        volume,
                    });
                }
            }
        }
        
        // Simple generic format for other sources
        if let (Some(s), Some(p), Some(t)) = (
            v["symbol"].as_str(),
            v["price"].as_f64(),
            v["timestamp"].as_i64(),
        ) {
            return Some(BotEvent::MarketEvent {
                symbol: s.into(),
                price: p,
                timestamp: t,
                event_type: v["event_type"].as_str().unwrap_or("trade").into(),
                volume: v["volume"].as_f64(),
            });
        }
        
        None
    }
}
