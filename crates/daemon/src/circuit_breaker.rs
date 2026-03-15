// crates/daemon/src/circuit_breaker.rs
//
// Half-open / Open / Closed circuit breaker for protecting
// downstream calls (RPC nodes, AI APIs, exchange WebSockets).

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Debug, Clone, PartialEq)]
pub enum BreakerState {
    /// All calls allowed through.
    Closed,
    /// All calls blocked — waiting for recovery window.
    Open { opened_at: Instant },
    /// One probe call allowed to test if service recovered.
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct BreakerConfig {
    /// Number of consecutive failures to trip the breaker.
    pub failure_threshold: u32,
    /// How long to wait before moving to HalfOpen.
    pub recovery_timeout: Duration,
    /// Consecutive successes in HalfOpen to close again.
    pub success_threshold: u32,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            success_threshold: 2,
        }
    }
}

struct BreakerInner {
    state: BreakerState,
    consecutive_failures: u32,
    consecutive_successes: u32,
    cfg: BreakerConfig,
    name: String,
}

/// Thread-safe circuit breaker.
#[derive(Clone)]
pub struct CircuitBreaker {
    inner: Arc<Mutex<BreakerInner>>,
}

#[derive(Debug)]
pub enum BreakerError<E> {
    /// The breaker is open — call was not attempted.
    Open,
    /// The call was attempted but failed.
    CallFailed(E),
}

impl<E: std::fmt::Display> std::fmt::Display for BreakerError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BreakerError::Open => write!(f, "circuit breaker open"),
            BreakerError::CallFailed(e) => write!(f, "call failed: {e}"),
        }
    }
}

impl CircuitBreaker {
    pub fn new(name: impl Into<String>, cfg: BreakerConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BreakerInner {
                state: BreakerState::Closed,
                consecutive_failures: 0,
                consecutive_successes: 0,
                cfg,
                name: name.into(),
            })),
        }
    }

    /// Execute `f` through the circuit breaker.
    pub async fn call<F, Fut, T, E>(&self, f: F) -> Result<T, BreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        // Check if we should allow the call
        {
            let mut inner = self.inner.lock().await;
            match &inner.state {
                BreakerState::Open { opened_at } => {
                    if opened_at.elapsed() >= inner.cfg.recovery_timeout {
                        warn!(name = %inner.name, "Circuit breaker → HalfOpen");
                        inner.state = BreakerState::HalfOpen;
                    } else {
                        return Err(BreakerError::Open);
                    }
                }
                BreakerState::Closed | BreakerState::HalfOpen => {}
            }
        }

        // Attempt the call
        match f().await {
            Ok(result) => {
                self.on_success().await;
                Ok(result)
            }
            Err(e) => {
                self.on_failure().await;
                Err(BreakerError::CallFailed(e))
            }
        }
    }

    async fn on_success(&self) {
        let mut inner = self.inner.lock().await;
        inner.consecutive_failures = 0;
        match &inner.state {
            BreakerState::HalfOpen => {
                inner.consecutive_successes += 1;
                if inner.consecutive_successes >= inner.cfg.success_threshold {
                    info!(name = %inner.name, "Circuit breaker → Closed");
                    inner.state = BreakerState::Closed;
                    inner.consecutive_successes = 0;
                }
            }
            _ => {}
        }
    }

    async fn on_failure(&self) {
        let mut inner = self.inner.lock().await;
        inner.consecutive_successes = 0;
        inner.consecutive_failures += 1;

        let should_open = match &inner.state {
            BreakerState::Closed => inner.consecutive_failures >= inner.cfg.failure_threshold,
            BreakerState::HalfOpen => true, // Any failure in HalfOpen re-opens
            BreakerState::Open { .. } => false,
        };

        if should_open {
            warn!(
                name = %inner.name,
                failures = inner.consecutive_failures,
                "Circuit breaker → Open"
            );
            inner.state = BreakerState::Open {
                opened_at: Instant::now(),
            };
            inner.consecutive_failures = 0;
        }
    }

    pub async fn is_open(&self) -> bool {
        matches!(self.inner.lock().await.state, BreakerState::Open { .. })
    }

    pub async fn force_close(&self) {
        let mut inner = self.inner.lock().await;
        inner.state = BreakerState::Closed;
        inner.consecutive_failures = 0;
        inner.consecutive_successes = 0;
        info!(name = %inner.name, "Circuit breaker force-closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_trips_after_threshold() {
        let cb = CircuitBreaker::new(
            "test",
            BreakerConfig {
                failure_threshold: 3,
                recovery_timeout: Duration::from_secs(60),
                success_threshold: 1,
            },
        );

        for _ in 0..3 {
            let _ = cb.call(|| async { Err::<(), &str>("fail") }).await;
        }

        // Should be open now
        assert!(cb.is_open().await);

        // Next call should return Open without executing
        let result = cb.call(|| async { Ok::<(), &str>(()) }).await;
        assert!(matches!(result, Err(BreakerError::Open)));
    }

    #[tokio::test]
    async fn test_passes_through_when_closed() {
        let cb = CircuitBreaker::new("test", BreakerConfig::default());
        let result = cb.call(|| async { Ok::<i32, &str>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }
}
