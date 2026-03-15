// crates/risk/src/kill_switch.rs
//
// Risk engine with live kill switches.
// Wires VaR, drawdown, and GARCH(1,1) volatility checks directly
// into the order submission path — any breach halts trading atomically.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};
use tracing::{error, info};

// ── Risk Events ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum RiskEvent {
    VarBreach {
        symbol: String,
        var_95: f64,
        actual_loss: f64,
    },
    DrawdownHalt {
        current_drawdown: f64,
        threshold: f64,
    },
    VolatilitySurge {
        symbol: String,
        garch_vol: f64,
        threshold: f64,
    },
    KillSwitchActivated {
        reason: String,
    },
    KillSwitchReset,
}

// ── Configuration ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RiskConfig {
    /// Maximum allowed 1-day 95% VaR as fraction of portfolio value.
    pub var_95_limit: f64,
    /// Maximum drawdown from peak before halt (e.g., 0.05 = 5%).
    pub max_drawdown: f64,
    /// GARCH(1,1) annualised vol threshold — halt if exceeded.
    pub vol_threshold: f64,
    /// GARCH(1,1) parameters.
    pub garch_omega: f64,
    pub garch_alpha: f64,
    pub garch_beta: f64,
    /// Minimum bars for GARCH to be considered reliable.
    pub garch_min_bars: usize,
    /// Rolling window size for VaR estimation.
    pub var_window: usize,
    /// Confidence level for historical VaR.
    pub var_confidence: f64,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            var_95_limit: 0.02,  // 2% of portfolio
            max_drawdown: 0.05,  // 5% drawdown halt
            vol_threshold: 0.80, // 80% annualised vol
            garch_omega: 0.000001,
            garch_alpha: 0.10,
            garch_beta: 0.85,
            garch_min_bars: 30,
            var_window: 252,
            var_confidence: 0.95,
        }
    }
}

// ── Kill Switch State ────────────────────────────────────────────────────────

/// Metadata stored under the RwLock — not consulted on the hot path.
#[derive(Debug)]
struct KillSwitchState {
    reason: Option<String>,
    activated_at: Option<Instant>,
}

/// Cloneable handle shared between `RiskEngine` and every `OrderGuard`.
/// The `active` flag lives outside any lock so hot-path checks are a single
/// atomic load; the `RwLock<KillSwitchState>` is only acquired to read or
/// write the `reason`/`activated_at` metadata.
#[derive(Clone)]
pub struct KillSwitchHandle {
    active: Arc<AtomicBool>,
    state: Arc<RwLock<KillSwitchState>>,
}

// ── GARCH(1,1) Tracker ───────────────────────────────────────────────────────

struct GarchTracker {
    omega: f64,
    alpha: f64,
    beta: f64,
    current_variance: f64,
    returns: VecDeque<f64>,
    last_price: Option<f64>,
}

impl GarchTracker {
    fn new(omega: f64, alpha: f64, beta: f64) -> Self {
        Self {
            omega,
            alpha,
            beta,
            current_variance: 0.0001, // Initial variance estimate
            returns: VecDeque::new(),
            last_price: None,
        }
    }

    fn update(&mut self, price: f64) -> Option<f64> {
        if let Some(prev) = self.last_price {
            let ret = (price / prev).ln();
            self.returns.push_back(ret);

            // σ²_t = ω + α·ε²_{t-1} + β·σ²_{t-1}
            self.current_variance =
                self.omega + self.alpha * ret.powi(2) + self.beta * self.current_variance;

            self.last_price = Some(price);
            // Convert daily variance to annualised vol
            Some((self.current_variance * 252.0).sqrt())
        } else {
            self.last_price = Some(price);
            None
        }
    }

    /// Returns the current annualised volatility estimate without requiring a
    /// new price observation.  Available for diagnostic/logging callers.
    #[allow(dead_code)]
    fn annualised_vol(&self) -> f64 {
        (self.current_variance * 252.0).sqrt()
    }
}

// ── Historical VaR ───────────────────────────────────────────────────────────

fn historical_var(returns: &VecDeque<f64>, confidence: f64) -> Option<f64> {
    if returns.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = returns.iter().copied().collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((1.0 - confidence) * sorted.len() as f64) as usize;
    Some(-sorted[idx.min(sorted.len() - 1)])
}

// ── Main Risk Engine ─────────────────────────────────────────────────────────

pub struct RiskEngine {
    cfg: RiskConfig,
    active: Arc<AtomicBool>,
    kill_switch: Arc<RwLock<KillSwitchState>>,
    event_tx: broadcast::Sender<RiskEvent>,
    portfolio_peak: f64,
    portfolio_value: f64,
    portfolio_returns: VecDeque<f64>,
    garch_trackers: std::collections::HashMap<String, GarchTracker>,
    position_losses: std::collections::HashMap<String, f64>,
}

impl RiskEngine {
    pub fn new(cfg: RiskConfig) -> (Self, broadcast::Receiver<RiskEvent>) {
        let (tx, rx) = broadcast::channel(256);
        let engine = Self {
            cfg: cfg.clone(),
            active: Arc::new(AtomicBool::new(false)),
            kill_switch: Arc::new(RwLock::new(KillSwitchState {
                reason: None,
                activated_at: None,
            })),
            event_tx: tx,
            portfolio_peak: 0.0,
            portfolio_value: 0.0,
            portfolio_returns: VecDeque::with_capacity(256),
            garch_trackers: Default::default(),
            position_losses: Default::default(),
        };
        (engine, rx)
    }

    /// Subscribe to risk events from another task.
    pub fn subscribe(&self) -> broadcast::Receiver<RiskEvent> {
        self.event_tx.subscribe()
    }

    /// Kill switch handle — clone into the order submission path.
    pub fn kill_switch_handle(&self) -> KillSwitchHandle {
        KillSwitchHandle {
            active: self.active.clone(),
            state: self.kill_switch.clone(),
        }
    }

    /// Update portfolio value and run all risk checks.
    /// Returns `Err(reason)` if the kill switch should be activated.
    pub async fn update_portfolio(&mut self, value: f64) -> Result<(), String> {
        let prev = self.portfolio_value;
        self.portfolio_value = value;

        if self.portfolio_peak == 0.0 {
            self.portfolio_peak = value;
        }

        if value > self.portfolio_peak {
            self.portfolio_peak = value;
        }

        // Track portfolio return
        if prev > 0.0 {
            let ret = (value / prev).ln();
            self.portfolio_returns.push_back(ret);
            if self.portfolio_returns.len() > self.cfg.var_window {
                self.portfolio_returns.pop_front();
            }
        }

        self.check_drawdown(value).await?;
        self.check_var(value).await?;
        Ok(())
    }

    /// Feed a new price tick for a symbol — runs GARCH vol check.
    pub async fn on_price_tick(&mut self, symbol: &str, price: f64) -> Result<(), String> {
        let tracker = self
            .garch_trackers
            .entry(symbol.to_string())
            .or_insert_with(|| {
                GarchTracker::new(
                    self.cfg.garch_omega,
                    self.cfg.garch_alpha,
                    self.cfg.garch_beta,
                )
            });

        if let Some(ann_vol) = tracker.update(price) {
            if ann_vol > self.cfg.vol_threshold {
                let event = RiskEvent::VolatilitySurge {
                    symbol: symbol.to_string(),
                    garch_vol: ann_vol,
                    threshold: self.cfg.vol_threshold,
                };
                let _ = self.event_tx.send(event);
                let reason = format!(
                    "GARCH vol {:.1}% exceeds threshold {:.1}% for {symbol}",
                    ann_vol * 100.0,
                    self.cfg.vol_threshold * 100.0
                );
                return self.activate_kill_switch(reason).await;
            }
        }

        Ok(())
    }

    async fn check_drawdown(&mut self, value: f64) -> Result<(), String> {
        if self.portfolio_peak <= 0.0 {
            return Ok(());
        }
        let drawdown = 1.0 - value / self.portfolio_peak;
        if drawdown > self.cfg.max_drawdown {
            let event = RiskEvent::DrawdownHalt {
                current_drawdown: drawdown,
                threshold: self.cfg.max_drawdown,
            };
            let _ = self.event_tx.send(event);
            let reason = format!(
                "Drawdown {:.2}% exceeds halt threshold {:.2}%",
                drawdown * 100.0,
                self.cfg.max_drawdown * 100.0
            );
            return self.activate_kill_switch(reason).await;
        }
        Ok(())
    }

    async fn check_var(&mut self, portfolio_value: f64) -> Result<(), String> {
        if let Some(var_95) = historical_var(&self.portfolio_returns, self.cfg.var_confidence) {
            let var_dollar = var_95 * portfolio_value;
            let limit_dollar = self.cfg.var_95_limit * portfolio_value;
            if var_dollar > limit_dollar {
                let event = RiskEvent::VarBreach {
                    symbol: "PORTFOLIO".to_string(),
                    var_95: var_dollar,
                    actual_loss: limit_dollar,
                };
                let _ = self.event_tx.send(event);
                let reason = format!("95% VaR ${var_dollar:.2} exceeds limit ${limit_dollar:.2}");
                return self.activate_kill_switch(reason).await;
            }
        }
        Ok(())
    }

    async fn activate_kill_switch(&self, reason: String) -> Result<(), String> {
        // Use compare_exchange so activation happens exactly once even under
        // concurrent calls.
        if self
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            error!(reason = %reason, "🔴 KILL SWITCH ACTIVATED");
            let mut ks = self.kill_switch.write().await;
            ks.reason = Some(reason.clone());
            ks.activated_at = Some(Instant::now());
            let _ = self.event_tx.send(RiskEvent::KillSwitchActivated {
                reason: reason.clone(),
            });
        }
        Err(reason)
    }

    pub async fn reset_kill_switch(&self) {
        self.active.store(false, Ordering::Release);
        let mut ks = self.kill_switch.write().await;
        ks.reason = None;
        ks.activated_at = None;
        info!("🟢 Kill switch reset — trading resumed");
        let _ = self.event_tx.send(RiskEvent::KillSwitchReset);
    }

    pub fn is_kill_switch_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

/// Guard used at the order submission path.
/// Call `check()` before any order is sent.
pub struct OrderGuard {
    handle: KillSwitchHandle,
}

impl OrderGuard {
    pub fn new(handle: KillSwitchHandle) -> Self {
        Self { handle }
    }

    pub async fn check(&self) -> Result<(), String> {
        // Fast path: single atomic load without any lock.
        if !self.handle.active.load(Ordering::Acquire) {
            return Ok(());
        }
        // Slow path: take the read lock only to retrieve the reason string.
        let ks = self.handle.state.read().await;
        let reason = ks
            .reason
            .clone()
            .unwrap_or_else(|| "Kill switch active".to_string());
        Err(format!("Order blocked by kill switch: {reason}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_drawdown_triggers_kill_switch() {
        let cfg = RiskConfig {
            max_drawdown: 0.05,
            ..Default::default()
        };
        let (mut engine, _rx) = RiskEngine::new(cfg);
        engine.update_portfolio(10_000.0).await.unwrap();
        // Drop to 94% of peak → 6% drawdown → should trip
        let result = engine.update_portfolio(9_400.0).await;
        assert!(result.is_err());
        assert!(engine.is_kill_switch_active());
    }

    #[tokio::test]
    async fn test_order_guard_blocks_when_active() {
        let cfg = RiskConfig {
            max_drawdown: 0.01,
            ..Default::default()
        };
        let (mut engine, _rx) = RiskEngine::new(cfg);
        engine.update_portfolio(1000.0).await.unwrap();
        let _ = engine.update_portfolio(985.0).await; // trip it

        let guard = OrderGuard::new(engine.kill_switch_handle());
        assert!(guard.check().await.is_err());
    }

    #[tokio::test]
    async fn test_reset_allows_orders() {
        let cfg = RiskConfig {
            max_drawdown: 0.01,
            ..Default::default()
        };
        let (mut engine, _rx) = RiskEngine::new(cfg);
        engine.update_portfolio(1000.0).await.unwrap();
        let _ = engine.update_portfolio(985.0).await;
        engine.reset_kill_switch().await;

        let guard = OrderGuard::new(engine.kill_switch_handle());
        assert!(guard.check().await.is_ok());
    }

    #[test]
    fn test_garch_variance_formula() {
        let mut tracker = GarchTracker::new(0.000001, 0.10, 0.85);
        let _ = tracker.update(100.0); // seed last_price
        let vol = tracker.update(101.0).unwrap();
        // Just sanity check it's a valid positive number
        assert!(vol > 0.0);
        assert!(vol.is_finite());
    }

    #[test]
    fn test_historical_var() {
        let returns: VecDeque<f64> = (-50..50).map(|i| i as f64 * 0.001).collect();
        let var = historical_var(&returns, 0.95).unwrap();
        assert!(var > 0.0);
    }
}
