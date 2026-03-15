// crates/daemon/src/reconnect.rs
//
// WebSocket reconnection manager with exponential backoff + jitter.
// Wraps any async producer fn that returns a WebSocket stream and
// automatically re-establishes the connection on failure.

use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// Configuration for reconnection behaviour.
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Initial backoff delay.
    pub initial_delay: Duration,
    /// Maximum backoff delay (cap).
    pub max_delay: Duration,
    /// Backoff multiplier applied after each failure.
    pub multiplier: f64,
    /// Maximum number of consecutive failures before giving up.
    /// `None` = retry forever.
    pub max_attempts: Option<u32>,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            max_attempts: None,
        }
    }
}

/// State machine tracking current backoff state.
pub struct ReconnectState {
    cfg: ReconnectConfig,
    current_delay: Duration,
    attempt: u32,
}

impl ReconnectState {
    pub fn new(cfg: ReconnectConfig) -> Self {
        let initial = cfg.initial_delay;
        Self {
            cfg,
            current_delay: initial,
            attempt: 0,
        }
    }

    /// Returns `false` if max_attempts exceeded.
    pub async fn wait(&mut self) -> bool {
        self.attempt += 1;

        if let Some(max) = self.cfg.max_attempts {
            if self.attempt > max {
                error!(
                    attempt = self.attempt,
                    max, "Max reconnect attempts exceeded — giving up"
                );
                return false;
            }
        }

        // Exponential backoff with ±25% jitter
        let jitter_range = self.current_delay.as_millis() / 4;
        let jitter = rand_jitter(jitter_range as u64);
        let sleep_ms = self.current_delay.as_millis() as u64 + jitter;

        warn!(
            attempt = self.attempt,
            delay_ms = sleep_ms,
            "WebSocket disconnected — reconnecting"
        );

        sleep(Duration::from_millis(sleep_ms)).await;

        // Advance delay, capped at max
        let next = Duration::from_secs_f64(
            self.current_delay.as_secs_f64() * self.cfg.multiplier,
        );
        self.current_delay = next.min(self.cfg.max_delay);

        true
    }

    /// Reset after a successful connection.
    pub fn reset(&mut self) {
        self.attempt = 0;
        self.current_delay = self.cfg.initial_delay;
        info!("WebSocket reconnected successfully — backoff reset");
    }
}

/// Run `producer` in a loop, reconnecting on error.
///
/// ```rust,ignore
/// reconnect_loop(config, || async {
///     let ws = connect_finnhub().await?;
///     ingest_stream(ws, tx.clone()).await
/// }).await;
/// ```
pub async fn reconnect_loop<F, Fut, E>(cfg: ReconnectConfig, mut producer: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    let mut state = ReconnectState::new(cfg);

    loop {
        match producer().await {
            Ok(()) => {
                info!("WebSocket producer exited cleanly — reconnecting");
                state.reset();
            }
            Err(e) => {
                error!(error = %e, "WebSocket producer error");
            }
        }

        if !state.wait().await {
            break;
        }
    }
}

/// Simple xorshift-based jitter (no rand dependency required).
fn rand_jitter(max_ms: u64) -> u64 {
    if max_ms == 0 {
        return 0;
    }
    // Use current time nanos as seed for lightweight jitter
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    let mut x = seed ^ 0xdeadbeef_cafebabe;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x % max_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_reconnect_attempts_counted() {
        let cfg = ReconnectConfig {
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(4),
            multiplier: 2.0,
            max_attempts: Some(3),
        };

        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();

        reconnect_loop(cfg, move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<(), &str>("simulated failure")
            }
        })
        .await;

        // 1 initial + 3 retries = 4 total calls
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn test_state_reset_on_success() {
        let cfg = ReconnectConfig::default();
        let mut state = ReconnectState::new(cfg);
        state.attempt = 10;
        state.current_delay = Duration::from_secs(30);
        state.reset();
        assert_eq!(state.attempt, 0);
        assert_eq!(state.current_delay, Duration::from_millis(500));
    }
}
