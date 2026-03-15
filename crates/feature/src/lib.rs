use common::SwapEvent;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

pub struct FeatureEngine {
    // Thread-safe store for per-token metrics
    metrics: Arc<DashMap<String, TokenMetrics>>,
}

#[derive(Default)]
pub struct TokenMetrics {
    pub last_price: f64,
    pub volume_24h: u128,
    pub buy_count_5m: u32,
    pub last_5m_reset: Option<SystemTime>,
}

impl FeatureEngine {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(DashMap::new()),
        }
    }

    pub fn process_event(&self, event: &SwapEvent) {
        let mut entry = self.metrics.entry(event.token_out.clone()).or_default();

        let now = event.timestamp;
        let should_reset = entry
            .last_5m_reset
            .and_then(|last| now.duration_since(last).ok())
            .map(|d| d >= Duration::from_secs(300))
            .unwrap_or(true);
        if should_reset {
            entry.buy_count_5m = 0;
            entry.last_5m_reset = Some(now);
        }

        entry.buy_count_5m += 1;
        entry.volume_24h += event.amount_in;
        entry.last_price = if event.amount_in > 0 {
            event.amount_out as f64 / event.amount_in as f64
        } else {
            0.0
        };
    }

    pub fn get_features(&self, token: &str) -> Option<TokenMetrics> {
        self.metrics.get(token).map(|v| TokenMetrics {
            last_price: v.last_price,
            volume_24h: v.volume_24h,
            buy_count_5m: v.buy_count_5m,
            last_5m_reset: v.last_5m_reset,
        })
    }
}
