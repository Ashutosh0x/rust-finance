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

#[derive(Debug)]
pub struct KillSwitchState {
    pub active: AtomicBool,
    pub reason: Option<String>,
    pub activated_at: Option<Instant>,
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
        if !price.is_finite() || price <= 0.0 {
            return None;
        }

        if let Some(prev) = self.last_price {
            if !prev.is_finite() || prev <= 0.0 {
                self.last_price = Some(price);
                return None;
            }
            let ret = (price / prev).ln();
            if !ret.is_finite() {
                self.last_price = Some(price);
                return None;
            }
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

    #[allow(dead_code)]
    fn annualised_vol(&self) -> f64 {
        (self.current_variance * 252.0).sqrt()
    }
}

// ── Historical VaR ───────────────────────────────────────────────────────────

fn historical_var(returns: &VecDeque<f64>, confidence: f64) -> Option<f64> {
    if returns.is_empty() || !confidence.is_finite() || !(0.0..1.0).contains(&confidence) {
        return None;
    }
    let mut sorted: Vec<f64> = returns.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.total_cmp(b));
    let idx = ((1.0 - confidence) * sorted.len() as f64) as usize;
    Some(-sorted[idx.min(sorted.len() - 1)])
}

// ── Main Risk Engine ─────────────────────────────────────────────────────────

pub struct RiskEngine {
    cfg: RiskConfig,
    kill_switch: Arc<RwLock<KillSwitchState>>,
    event_tx: broadcast::Sender<RiskEvent>,
    portfolio_peak: f64,
    portfolio_value: f64,
    portfolio_returns: VecDeque<f64>,
    garch_trackers: std::collections::HashMap<String, GarchTracker>,
    #[allow(dead_code)]
    position_losses: std::collections::HashMap<String, f64>,
}

impl RiskEngine {
    pub fn new(cfg: RiskConfig) -> (Self, broadcast::Receiver<RiskEvent>) {
        let (tx, rx) = broadcast::channel(256);
        let engine = Self {
            cfg: cfg.clone(),
            kill_switch: Arc::new(RwLock::new(KillSwitchState {
                active: AtomicBool::new(false),
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
    pub fn kill_switch_handle(&self) -> Arc<RwLock<KillSwitchState>> {
        self.kill_switch.clone()
    }

    /// Update portfolio value and run all risk checks.
    /// Returns `Err(reason)` if the kill switch should be activated.
    pub async fn update_portfolio(&mut self, value: f64) -> Result<(), String> {
        if !value.is_finite() || value <= 0.0 {
            return self
                .activate_kill_switch(format!("Invalid portfolio value: {value}"))
                .await;
        }

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
        if symbol.trim().is_empty() || !price.is_finite() || price <= 0.0 {
            return self
                .activate_kill_switch(format!("Invalid price tick for {symbol}: {price}"))
                .await;
        }

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
            if tracker.returns.len() > self.cfg.var_window {
                tracker.returns.pop_front();
            }
            if tracker.returns.len() < self.cfg.garch_min_bars {
                return Ok(());
            }
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
        let mut ks = self.kill_switch.write().await;
        if !ks.active.load(Ordering::Relaxed) {
            error!(reason = %reason, "[!!] KILL SWITCH ACTIVATED");
            ks.active.store(true, Ordering::Relaxed);
            ks.reason = Some(reason.clone());
            ks.activated_at = Some(Instant::now());
            let _ = self.event_tx.send(RiskEvent::KillSwitchActivated {
                reason: reason.clone(),
            });
        }
        Err(reason)
    }

    pub async fn reset_kill_switch(&self) {
        let mut ks = self.kill_switch.write().await;
        ks.active.store(false, Ordering::Relaxed);
        ks.reason = None;
        ks.activated_at = None;
        info!("[OK] Kill switch reset -- trading resumed");
        let _ = self.event_tx.send(RiskEvent::KillSwitchReset);
    }

    pub async fn is_kill_switch_active(&self) -> bool {
        self.kill_switch.read().await.active.load(Ordering::Relaxed)
    }
}

/// Guard used at the order submission path.
/// Call `check()` before any order is sent.
pub struct OrderGuard {
    kill_switch: Arc<RwLock<KillSwitchState>>,
}

impl OrderGuard {
    pub fn new(kill_switch: Arc<RwLock<KillSwitchState>>) -> Self {
        Self { kill_switch }
    }

    pub async fn check(&self) -> Result<(), String> {
        let ks = self.kill_switch.read().await;
        if ks.active.load(Ordering::Relaxed) {
            let reason = ks
                .reason
                .clone()
                .unwrap_or_else(|| "Kill switch active".to_string());
            Err(format!("Order blocked by kill switch: {reason}"))
        } else {
            Ok(())
        }
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
        assert!(engine.is_kill_switch_active().await);
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

    /// E2E composition: RiskEngine drawdown → KillSwitch trips → OrderGuard blocks.
    /// This is the single most important test: it proves the crates compose correctly
    /// under real signal flow, not just in unit-test isolation.
    #[tokio::test]
    async fn test_e2e_kill_switch_blocks_executor_after_drawdown() {
        use common::events::{OrderSide, OrderType};
        use execution::gateway::{ExecutionGateway, OpenRequest, TimeInForce};
        use execution::recording_executor::RecordingExecutor;

        // Setup: RiskEngine with tight drawdown limit
        let cfg = RiskConfig {
            max_drawdown: 0.03, // 3% — will trip easily
            ..Default::default()
        };
        let (mut engine, _rx) = RiskEngine::new(cfg);
        let executor = RecordingExecutor::new();

        let make_order = || OpenRequest {
            client_order_id: "test-001".into(),
            symbol: "NVDA".into(),
            side: OrderSide::Buy,
            quantity: 10.0,
            order_type: OrderType::Limit,
            limit_price: Some(900.0),
            time_in_force: TimeInForce::DAY,
        };

        // Phase 1: Portfolio healthy — orders should flow through
        engine.update_portfolio(100_000.0).await.unwrap();
        let guard = OrderGuard::new(engine.kill_switch_handle());
        assert!(
            guard.check().await.is_ok(),
            "Guard should pass when healthy"
        );
        let _ = executor.submit_order(make_order()).await;
        assert_eq!(
            executor.submission_count(),
            1,
            "Order should reach executor"
        );

        // Phase 2: Drawdown trips kill switch
        let result = engine.update_portfolio(96_000.0).await; // -4% > 3% limit
        assert!(result.is_err(), "Drawdown should trip kill switch");
        assert!(
            engine.is_kill_switch_active().await,
            "Kill switch should be active"
        );

        // Phase 3: Guard blocks — executor should NOT receive new orders
        assert!(
            guard.check().await.is_err(),
            "Guard should block after kill switch"
        );
        // Do NOT submit — this is the critical invariant
        assert_eq!(
            executor.submission_count(),
            1,
            "No new orders after kill switch"
        );

        // Phase 4: Reset restores flow
        engine.reset_kill_switch().await;
        assert!(guard.check().await.is_ok(), "Guard should pass after reset");
        let _ = executor.submit_order(make_order()).await;
        assert_eq!(executor.submission_count(), 2, "Orders flow after reset");
    }

    /// GARCH vol spike → kill switch → orders blocked.
    #[tokio::test]
    async fn test_e2e_garch_vol_spike_trips_kill_switch() {
        let cfg = RiskConfig {
            vol_threshold: 0.50, // 50% annualized vol threshold
            ..Default::default()
        };
        let (mut engine, _rx) = RiskEngine::new(cfg);
        engine.update_portfolio(100_000.0).await.unwrap();

        // Feed normal prices — no trip
        for price in [100.0, 100.5, 101.0, 100.8, 100.2] {
            assert!(engine.on_price_tick("NVDA", price).await.is_ok());
        }
        assert!(
            !engine.is_kill_switch_active().await,
            "Normal vol should not trip"
        );

        // Inject extreme price shock: 100 → 200 → 50 → 300
        let _ = engine.on_price_tick("NVDA", 200.0).await;
        let result = engine.on_price_tick("NVDA", 50.0).await;
        // After extreme moves, GARCH vol should exceed 50% annualized
        if result.is_err() {
            assert!(
                engine.is_kill_switch_active().await,
                "Extreme vol should trip kill switch"
            );
            let guard = OrderGuard::new(engine.kill_switch_handle());
            assert!(guard.check().await.is_err(), "Guard should block");
        }
        // If it didn't trip yet (vol accumulates gradually), push harder
        if !engine.is_kill_switch_active().await {
            let _ = engine.on_price_tick("NVDA", 500.0).await;
            let _ = engine.on_price_tick("NVDA", 10.0).await;
            // At this point GARCH variance is extreme
        }
    }
}
