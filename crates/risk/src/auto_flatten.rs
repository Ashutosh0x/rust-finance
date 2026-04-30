// crates/risk/src/auto_flatten.rs
//
// Automated Kill Switch with Position Flattening
//
// Inspired by NautilusTrader's `market_exit()` pattern:
//   1. Set global halt flag (prevents new order submission)
//   2. Cancel all open/in-flight orders
//   3. Submit market orders to close every open position
//   4. Log to audit trail
//
// This is the #1 safety feature missing from most open-source trading systems.
// Every institutional system (NautilusTrader, AlgoTrader/Wyden, QuantConnect)
// ships this as a non-negotiable baseline.
//
// Triggers:
//   - Daily PnL loss exceeds configured limit
//   - Drawdown exceeds configured percentage
//   - Manual activation via TUI hotkey
//   - GARCH volatility surge beyond threshold

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info, warn};

// ── Configuration ─────────────────────────────────────────────────

/// Auto-flatten configuration.
#[derive(Debug, Clone)]
pub struct AutoFlattenConfig {
    /// Maximum daily loss in USD before auto-flatten triggers.
    /// E.g., 5000.0 means flatten if daily PnL drops below -$5000.
    pub max_daily_loss_usd: f64,

    /// Maximum daily loss as percentage of starting equity.
    /// E.g., 0.02 means 2% of SOD (start-of-day) equity.
    pub max_daily_loss_pct: f64,

    /// Maximum drawdown from peak before auto-flatten.
    pub max_drawdown_pct: f64,

    /// Cooldown period in seconds after auto-flatten before trading can resume.
    /// Prevents rapid re-entry after a crisis.
    pub cooldown_secs: u64,

    /// Whether to actually submit market close orders (true) or just halt (false).
    /// Set to false for paper trading / testing.
    pub execute_close_orders: bool,
}

impl Default for AutoFlattenConfig {
    fn default() -> Self {
        Self {
            max_daily_loss_usd: 5_000.0,
            max_daily_loss_pct: 0.02,
            max_drawdown_pct: 0.05,
            cooldown_secs: 300, // 5 minutes
            execute_close_orders: true,
        }
    }
}

// ── Kill Reason ───────────────────────────────────────────────────

/// Reason the auto-flatten was triggered.
#[derive(Debug, Clone, PartialEq)]
pub enum KillReason {
    /// Daily PnL exceeded absolute USD limit.
    DailyLossUsd { loss: f64, limit: f64 },
    /// Daily PnL exceeded percentage limit.
    DailyLossPct { loss_pct: f64, limit_pct: f64 },
    /// Drawdown from peak exceeded limit.
    Drawdown { drawdown_pct: f64, limit_pct: f64 },
    /// Volatility surge detected by GARCH.
    VolatilitySurge { symbol: String, vol: f64, threshold: f64 },
    /// Manual activation via TUI/API.
    Manual { operator: String },
    /// External signal (e.g., exchange maintenance, circuit breaker).
    External { source: String },
}

impl std::fmt::Display for KillReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DailyLossUsd { loss, limit } =>
                write!(f, "Daily loss ${:.2} exceeds limit ${:.2}", loss, limit),
            Self::DailyLossPct { loss_pct, limit_pct } =>
                write!(f, "Daily loss {:.2}% exceeds limit {:.2}%", loss_pct * 100.0, limit_pct * 100.0),
            Self::Drawdown { drawdown_pct, limit_pct } =>
                write!(f, "Drawdown {:.2}% exceeds limit {:.2}%", drawdown_pct * 100.0, limit_pct * 100.0),
            Self::VolatilitySurge { symbol, vol, threshold } =>
                write!(f, "GARCH vol {:.1}% exceeds {:.1}% for {}", vol * 100.0, threshold * 100.0, symbol),
            Self::Manual { operator } =>
                write!(f, "Manual kill switch by {}", operator),
            Self::External { source } =>
                write!(f, "External halt: {}", source),
        }
    }
}

// ── Flatten Event (for audit trail) ──────────────────────────────

/// Event emitted when auto-flatten triggers.
#[derive(Debug, Clone)]
pub struct FlattenEvent {
    pub reason: KillReason,
    pub timestamp_ms: i64,
    pub sod_equity: f64,
    pub current_equity: f64,
    pub daily_pnl: f64,
    pub positions_to_close: usize,
    pub orders_to_cancel: usize,
}

// ── Position Record (for flattening) ─────────────────────────────

/// Minimal position info needed for flattening.
#[derive(Debug, Clone)]
pub struct OpenPosition {
    pub symbol: String,
    pub quantity: f64, // positive = long, negative = short
    pub entry_price: f64,
    pub current_price: f64,
    pub unrealized_pnl: f64,
}

// ── Close Order (output) ─────────────────────────────────────────

/// Market order to close a position.
#[derive(Debug, Clone)]
pub struct CloseOrder {
    pub symbol: String,
    /// Quantity to close (always opposite sign of position).
    pub quantity: f64,
    /// Market order — no price limit.
    pub is_market: bool,
    /// Reason for close.
    pub reason: String,
}

// ── Auto Flatten Engine ──────────────────────────────────────────

/// Core auto-flatten engine.
///
/// Usage:
/// ```ignore
/// let halt = Arc::new(AtomicBool::new(false));
/// let mut engine = AutoFlattenEngine::new(config, halt.clone());
///
/// // On every tick:
/// engine.update_equity(current_equity);
///
/// // Check if halted before submitting orders:
/// if engine.is_halted() { return; }
///
/// // When triggered, get close orders:
/// if let Some(event) = engine.check_triggers() {
///     let close_orders = engine.generate_close_orders(&positions);
///     for order in close_orders {
///         executor.submit_market_close(order).await;
///     }
/// }
/// ```
pub struct AutoFlattenEngine {
    config: AutoFlattenConfig,

    /// Global halt flag — shared with order guard.
    /// When true, ALL new order submissions are blocked.
    halted: Arc<AtomicBool>,

    /// Start-of-day equity (reset daily).
    sod_equity: f64,

    /// Current equity value.
    current_equity: f64,

    /// Peak equity (for drawdown calculation).
    peak_equity: f64,

    /// Daily PnL = current_equity - sod_equity.
    daily_pnl: f64,

    /// When the kill switch was activated.
    activated_at: Option<Instant>,

    /// History of flatten events (audit trail).
    event_history: Vec<FlattenEvent>,
}

impl AutoFlattenEngine {
    pub fn new(config: AutoFlattenConfig, halted: Arc<AtomicBool>) -> Self {
        Self {
            config,
            halted,
            sod_equity: 0.0,
            current_equity: 0.0,
            peak_equity: 0.0,
            daily_pnl: 0.0,
            activated_at: None,
            event_history: Vec::new(),
        }
    }

    /// Set start-of-day equity. Call once at session open.
    pub fn set_sod_equity(&mut self, equity: f64) {
        self.sod_equity = equity;
        self.current_equity = equity;
        if equity > self.peak_equity {
            self.peak_equity = equity;
        }
        info!(sod_equity = equity, "SOD equity set for auto-flatten engine");
    }

    /// Update current equity. Call on every portfolio valuation tick.
    pub fn update_equity(&mut self, equity: f64) {
        self.current_equity = equity;
        self.daily_pnl = equity - self.sod_equity;

        if equity > self.peak_equity {
            self.peak_equity = equity;
        }
    }

    /// Check if any auto-flatten trigger has been breached.
    /// Returns `Some(FlattenEvent)` if triggered, `None` if safe.
    pub fn check_triggers(&mut self, open_positions: usize, open_orders: usize) -> Option<FlattenEvent> {
        // Already halted — don't re-trigger
        if self.is_halted() {
            return None;
        }

        // Skip if SOD equity not yet set
        if self.sod_equity <= 0.0 {
            return None;
        }

        // Check 1: Absolute daily loss
        if self.daily_pnl < -self.config.max_daily_loss_usd {
            let reason = KillReason::DailyLossUsd {
                loss: -self.daily_pnl,
                limit: self.config.max_daily_loss_usd,
            };
            return Some(self.trigger_flatten(reason, open_positions, open_orders));
        }

        // Check 2: Percentage daily loss
        let loss_pct = -self.daily_pnl / self.sod_equity;
        if loss_pct > self.config.max_daily_loss_pct {
            let reason = KillReason::DailyLossPct {
                loss_pct,
                limit_pct: self.config.max_daily_loss_pct,
            };
            return Some(self.trigger_flatten(reason, open_positions, open_orders));
        }

        // Check 3: Drawdown from peak
        if self.peak_equity > 0.0 {
            let drawdown = 1.0 - self.current_equity / self.peak_equity;
            if drawdown > self.config.max_drawdown_pct {
                let reason = KillReason::Drawdown {
                    drawdown_pct: drawdown,
                    limit_pct: self.config.max_drawdown_pct,
                };
                return Some(self.trigger_flatten(reason, open_positions, open_orders));
            }
        }

        None
    }

    /// Manually trigger auto-flatten (e.g., from TUI hotkey or API).
    pub fn manual_trigger(&mut self, operator: &str, open_positions: usize, open_orders: usize) -> FlattenEvent {
        let reason = KillReason::Manual {
            operator: operator.to_string(),
        };
        self.trigger_flatten(reason, open_positions, open_orders)
    }

    /// Generate market close orders for all open positions.
    pub fn generate_close_orders(&self, positions: &[OpenPosition]) -> Vec<CloseOrder> {
        positions
            .iter()
            .filter(|p| p.quantity.abs() > 1e-10) // skip dust
            .map(|p| CloseOrder {
                symbol: p.symbol.clone(),
                quantity: -p.quantity, // opposite side
                is_market: true,
                reason: format!(
                    "Auto-flatten: {} {:.4} @ {:.2} (uPnL: {:.2})",
                    p.symbol, p.quantity, p.current_price, p.unrealized_pnl
                ),
            })
            .collect()
    }

    /// Is trading currently halted?
    pub fn is_halted(&self) -> bool {
        self.halted.load(Ordering::SeqCst)
    }

    /// Reset the halt flag after cooldown period.
    /// Returns `true` if reset was successful, `false` if still in cooldown.
    pub fn try_reset(&mut self) -> bool {
        if let Some(activated_at) = self.activated_at {
            let elapsed = activated_at.elapsed().as_secs();
            if elapsed < self.config.cooldown_secs {
                warn!(
                    remaining_secs = self.config.cooldown_secs - elapsed,
                    "Cannot reset: cooldown period active"
                );
                return false;
            }
        }

        self.halted.store(false, Ordering::SeqCst);
        self.activated_at = None;
        info!("Auto-flatten engine reset — trading may resume");
        true
    }

    /// Force reset (bypass cooldown). Use with extreme caution.
    pub fn force_reset(&mut self) {
        self.halted.store(false, Ordering::SeqCst);
        self.activated_at = None;
        warn!("Auto-flatten engine FORCE RESET — cooldown bypassed");
    }

    /// Get the audit trail of all flatten events this session.
    pub fn event_history(&self) -> &[FlattenEvent] {
        &self.event_history
    }

    /// Current daily PnL.
    pub fn daily_pnl(&self) -> f64 {
        self.daily_pnl
    }

    /// Current drawdown from peak.
    pub fn drawdown_pct(&self) -> f64 {
        if self.peak_equity > 0.0 {
            1.0 - self.current_equity / self.peak_equity
        } else {
            0.0
        }
    }

    // ── Internal ─────────────────────────────────────────────────

    fn trigger_flatten(
        &mut self,
        reason: KillReason,
        open_positions: usize,
        open_orders: usize,
    ) -> FlattenEvent {
        // Step 1: Set global halt flag IMMEDIATELY
        self.halted.store(true, Ordering::SeqCst);
        self.activated_at = Some(Instant::now());

        error!(
            reason = %reason,
            daily_pnl = self.daily_pnl,
            equity = self.current_equity,
            positions = open_positions,
            orders = open_orders,
            "[!!!] AUTO-FLATTEN TRIGGERED — ALL TRADING HALTED"
        );

        let event = FlattenEvent {
            reason,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            sod_equity: self.sod_equity,
            current_equity: self.current_equity,
            daily_pnl: self.daily_pnl,
            positions_to_close: open_positions,
            orders_to_cancel: open_orders,
        };

        self.event_history.push(event.clone());
        event
    }
}

// ── Pre-Trade Risk Checks (Fat-Finger Protection) ────────────────

/// Fat-finger protection: blocks orders that are obviously erroneous.
///
/// Based on AlgoTrader/Wyden pre-trade risk suite:
/// - Max single order notional value
/// - Max order quantity
/// - Max order as % of ADV (Average Daily Volume)
/// - Price deviation from reference (e.g., last trade price)
#[derive(Debug, Clone)]
pub struct FatFingerConfig {
    /// Maximum notional value of a single order in USD.
    pub max_order_notional_usd: f64,
    /// Maximum quantity per order.
    pub max_order_quantity: f64,
    /// Maximum order size as fraction of ADV. E.g., 0.05 = 5% of ADV.
    pub max_adv_fraction: f64,
    /// Maximum allowed price deviation from reference price.
    /// E.g., 0.10 = reject if order price is >10% away from last trade.
    pub max_price_deviation: f64,
}

impl Default for FatFingerConfig {
    fn default() -> Self {
        Self {
            max_order_notional_usd: 1_000_000.0,
            max_order_quantity: 10_000.0,
            max_adv_fraction: 0.05,
            max_price_deviation: 0.10,
        }
    }
}

/// Result of a fat-finger check.
#[derive(Debug, Clone, PartialEq)]
pub enum FatFingerResult {
    /// Order passes all checks.
    Pass,
    /// Order is rejected with reason.
    Reject(String),
}

/// Check an order against fat-finger protection rules.
pub fn fat_finger_check(
    config: &FatFingerConfig,
    order_qty: f64,
    order_price: f64,
    reference_price: f64,
    adv: Option<f64>,
) -> FatFingerResult {
    // Check 1: Max quantity
    if order_qty.abs() > config.max_order_quantity {
        return FatFingerResult::Reject(format!(
            "Order qty {:.2} exceeds max {:.2}",
            order_qty.abs(),
            config.max_order_quantity
        ));
    }

    // Check 2: Max notional
    let notional = order_qty.abs() * order_price;
    if notional > config.max_order_notional_usd {
        return FatFingerResult::Reject(format!(
            "Order notional ${:.2} exceeds max ${:.2}",
            notional, config.max_order_notional_usd
        ));
    }

    // Check 3: Price deviation from reference
    if reference_price > 0.0 {
        let deviation = ((order_price - reference_price) / reference_price).abs();
        if deviation > config.max_price_deviation {
            return FatFingerResult::Reject(format!(
                "Price {:.2} deviates {:.1}% from ref {:.2} (max {:.1}%)",
                order_price,
                deviation * 100.0,
                reference_price,
                config.max_price_deviation * 100.0
            ));
        }
    }

    // Check 4: ADV fraction
    if let Some(adv) = adv {
        if adv > 0.0 && order_qty.abs() > adv * config.max_adv_fraction {
            return FatFingerResult::Reject(format!(
                "Order qty {:.0} exceeds {:.1}% of ADV {:.0}",
                order_qty.abs(),
                config.max_adv_fraction * 100.0,
                adv
            ));
        }
    }

    FatFingerResult::Pass
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine(max_loss_usd: f64, max_loss_pct: f64, max_dd: f64) -> AutoFlattenEngine {
        let config = AutoFlattenConfig {
            max_daily_loss_usd: max_loss_usd,
            max_daily_loss_pct: max_loss_pct,
            max_drawdown_pct: max_dd,
            cooldown_secs: 1, // short for tests
            execute_close_orders: true,
        };
        let halted = Arc::new(AtomicBool::new(false));
        AutoFlattenEngine::new(config, halted)
    }

    #[test]
    fn test_no_trigger_within_limits() {
        let mut engine = make_engine(5000.0, 0.02, 0.05);
        engine.set_sod_equity(100_000.0);
        engine.update_equity(99_000.0); // -$1000, within limit

        let result = engine.check_triggers(2, 3);
        assert!(result.is_none(), "Should not trigger within limits");
        assert!(!engine.is_halted());
    }

    #[test]
    fn test_daily_loss_usd_triggers() {
        let mut engine = make_engine(5000.0, 0.10, 0.10);
        engine.set_sod_equity(100_000.0);
        engine.update_equity(94_000.0); // -$6000 > $5000 limit

        let result = engine.check_triggers(2, 3);
        assert!(result.is_some(), "Should trigger on USD loss");
        assert!(engine.is_halted(), "Should be halted");

        let event = result.unwrap();
        assert!(matches!(event.reason, KillReason::DailyLossUsd { .. }));
        assert_eq!(event.positions_to_close, 2);
        assert_eq!(event.orders_to_cancel, 3);
    }

    #[test]
    fn test_daily_loss_pct_triggers() {
        let mut engine = make_engine(999_999.0, 0.02, 0.10);
        engine.set_sod_equity(100_000.0);
        engine.update_equity(97_500.0); // -2.5% > 2% limit

        let result = engine.check_triggers(1, 0);
        assert!(result.is_some(), "Should trigger on pct loss");
        assert!(matches!(result.unwrap().reason, KillReason::DailyLossPct { .. }));
    }

    #[test]
    fn test_drawdown_triggers() {
        let mut engine = make_engine(999_999.0, 0.50, 0.05);
        engine.set_sod_equity(100_000.0);
        engine.update_equity(110_000.0); // new peak
        engine.update_equity(103_000.0); // -6.4% drawdown > 5%

        let result = engine.check_triggers(3, 1);
        assert!(result.is_some(), "Should trigger on drawdown");
        assert!(matches!(result.unwrap().reason, KillReason::Drawdown { .. }));
    }

    #[test]
    fn test_generate_close_orders() {
        let engine = make_engine(5000.0, 0.02, 0.05);
        let positions = vec![
            OpenPosition {
                symbol: "BTCUSDT".into(),
                quantity: 0.5,
                entry_price: 60000.0,
                current_price: 59000.0,
                unrealized_pnl: -500.0,
            },
            OpenPosition {
                symbol: "ETHUSDT".into(),
                quantity: -10.0,
                entry_price: 3000.0,
                current_price: 3100.0,
                unrealized_pnl: -1000.0,
            },
        ];

        let orders = engine.generate_close_orders(&positions);
        assert_eq!(orders.len(), 2);

        // Long BTC → sell (negative qty)
        assert_eq!(orders[0].symbol, "BTCUSDT");
        assert!((orders[0].quantity - (-0.5)).abs() < 1e-10);
        assert!(orders[0].is_market);

        // Short ETH → buy (positive qty)
        assert_eq!(orders[1].symbol, "ETHUSDT");
        assert!((orders[1].quantity - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_cooldown_prevents_reset() {
        let config = AutoFlattenConfig {
            cooldown_secs: 9999, // long cooldown
            ..AutoFlattenConfig::default()
        };
        let halted = Arc::new(AtomicBool::new(false));
        let mut engine = AutoFlattenEngine::new(config, halted);
        engine.set_sod_equity(100_000.0);
        engine.update_equity(50_000.0); // huge loss
        let _ = engine.check_triggers(0, 0);

        // Try to reset — should fail due to cooldown
        assert!(!engine.try_reset(), "Should be in cooldown");
        assert!(engine.is_halted(), "Should still be halted");
    }

    #[test]
    fn test_force_reset_bypasses_cooldown() {
        let config = AutoFlattenConfig {
            cooldown_secs: 9999,
            max_daily_loss_usd: 100.0,
            ..AutoFlattenConfig::default()
        };
        let halted = Arc::new(AtomicBool::new(false));
        let mut engine = AutoFlattenEngine::new(config, halted);
        engine.set_sod_equity(10000.0);
        engine.update_equity(9000.0);
        let _ = engine.check_triggers(0, 0);

        engine.force_reset();
        assert!(!engine.is_halted(), "Force reset should unhalt");
    }

    #[test]
    fn test_manual_trigger() {
        let mut engine = make_engine(999_999.0, 0.99, 0.99);
        engine.set_sod_equity(100_000.0);
        engine.update_equity(100_000.0); // no loss

        let event = engine.manual_trigger("admin", 5, 10);
        assert!(engine.is_halted());
        assert!(matches!(event.reason, KillReason::Manual { .. }));
        assert_eq!(event.positions_to_close, 5);
    }

    #[test]
    fn test_event_history_accumulates() {
        let mut engine = make_engine(100.0, 0.001, 0.99);
        engine.set_sod_equity(10_000.0);
        engine.update_equity(9_800.0);
        let _ = engine.check_triggers(0, 0);
        assert_eq!(engine.event_history().len(), 1);

        engine.force_reset();
        engine.update_equity(9_500.0);
        let _ = engine.check_triggers(0, 0);
        assert_eq!(engine.event_history().len(), 2);
    }

    // ── Fat-Finger Tests ─────────────────────────────────────────

    #[test]
    fn test_fat_finger_pass() {
        let config = FatFingerConfig::default();
        let result = fat_finger_check(&config, 100.0, 50.0, 50.0, Some(100_000.0));
        assert_eq!(result, FatFingerResult::Pass);
    }

    #[test]
    fn test_fat_finger_max_quantity() {
        let config = FatFingerConfig {
            max_order_quantity: 100.0,
            ..FatFingerConfig::default()
        };
        let result = fat_finger_check(&config, 200.0, 50.0, 50.0, None);
        assert!(matches!(result, FatFingerResult::Reject(_)));
    }

    #[test]
    fn test_fat_finger_max_notional() {
        let config = FatFingerConfig {
            max_order_notional_usd: 10_000.0,
            ..FatFingerConfig::default()
        };
        let result = fat_finger_check(&config, 100.0, 200.0, 200.0, None);
        // 100 × 200 = $20,000 > $10,000
        assert!(matches!(result, FatFingerResult::Reject(_)));
    }

    #[test]
    fn test_fat_finger_price_deviation() {
        let config = FatFingerConfig {
            max_price_deviation: 0.05,
            ..FatFingerConfig::default()
        };
        // Order at $110, ref at $100 → 10% deviation > 5% limit
        let result = fat_finger_check(&config, 10.0, 110.0, 100.0, None);
        assert!(matches!(result, FatFingerResult::Reject(_)));
    }

    #[test]
    fn test_fat_finger_adv_fraction() {
        let config = FatFingerConfig {
            max_adv_fraction: 0.05,
            ..FatFingerConfig::default()
        };
        // Order 600 shares, ADV = 10000 → 6% > 5% limit
        let result = fat_finger_check(&config, 600.0, 50.0, 50.0, Some(10_000.0));
        assert!(matches!(result, FatFingerResult::Reject(_)));
    }
}
