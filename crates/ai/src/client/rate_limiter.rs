use anyhow::{bail, Result};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

pub struct TokenBucketLimiter {
    capacity: usize,
    tokens: Mutex<usize>,
    refill_rate_per_sec: f64,
    last_refill: Mutex<Instant>,
}

impl TokenBucketLimiter {
    pub fn new(capacity: usize, refill_rate_per_sec: f64) -> Arc<Self> {
        Arc::new(Self {
            capacity,
            tokens: Mutex::new(capacity),
            refill_rate_per_sec,
            last_refill: Mutex::new(Instant::now()),
        })
    }

    pub async fn acquire(&self, amount: usize) -> Result<()> {
        let mut tokens_guard = self.tokens.lock().await;
        let mut time_guard = self.last_refill.lock().await;

        // Calculate refill based on elapsed time elapsed * rate
        let now = Instant::now();
        let elapsed_secs = now.duration_since(*time_guard).as_secs_f64();
        let add_tokens = (elapsed_secs * self.refill_rate_per_sec) as usize;

        if add_tokens > 0 {
            *tokens_guard = std::cmp::min(self.capacity, *tokens_guard + add_tokens);
            // Only bump the clock forward by the *full fractional tokens* we actually credited,
            // but for a trading bot approximation, snapping to now is fine.
            *time_guard = now;
        }

        if *tokens_guard >= amount {
            *tokens_guard -= amount;
            Ok(())
        } else {
            // Need to wait. Real implementation might sleep or just reject immediately.
            tracing::warn!(
                "Rate limit exceeded. Required: {}, Available: {}",
                amount,
                *tokens_guard
            );
            bail!("Anthropic API rate limit exceeded");
        }
    }
}
