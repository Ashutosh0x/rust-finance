// crates/compliance/src/pre_trade.rs
// SEBI 2026 mandatory pre-trade risk checks — every order MUST pass before touching exchange

use crate::errors::ComplianceError;
use common::models::order::Order;

#[derive(Debug, Clone)]
pub struct PreTradeConfig {
    /// Maximum single order size as multiple of average order size
    pub max_order_size_multiplier: f64, // e.g. 2.0 = reject if 2× normal size
    /// Maximum price deviation from last trade price (fraction, e.g. 0.05 = 5%)
    pub max_price_deviation_pct: f64,
    /// Maximum daily notional value across all orders
    pub max_daily_notional_usd: f64,
    /// Maximum single order notional value
    pub max_single_order_notional_usd: f64,
    /// Maximum orders per second (rate limit)
    pub max_orders_per_second: u32,
    /// Block orders when market is in auction / pre-market
    pub block_outside_market_hours: bool,
}

impl Default for PreTradeConfig {
    fn default() -> Self {
        Self {
            max_order_size_multiplier: 2.0,
            max_price_deviation_pct: 0.05,
            max_daily_notional_usd: 100_000.0,
            max_single_order_notional_usd: 25_000.0,
            max_orders_per_second: 10,
            block_outside_market_hours: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct PreTradeState {
    pub daily_notional_used: f64,
    pub orders_this_second: u32,
    pub last_second_ts: u64,
    pub average_order_size: f64, // rolling average, updated per fill
}

pub struct PreTradeGuard {
    config: PreTradeConfig,
    state: PreTradeState,
}

impl PreTradeGuard {
    pub fn new(config: PreTradeConfig) -> Self {
        Self {
            config,
            state: PreTradeState::default(),
        }
    }

    /// Run every check. Returns Ok(()) or the first violation found.
    pub fn check(
        &mut self,
        order: &Order,
        last_price: f64,
        now_ts_secs: u64,
    ) -> Result<(), ComplianceError> {
        self.check_basic_inputs(order, last_price)?;
        self.check_notional_single(order, last_price)?;
        self.check_daily_notional(order, last_price)?;
        self.check_price_deviation(order, last_price)?;
        self.check_fat_finger_size(order)?;
        self.check_rate_limit(now_ts_secs)?;
        Ok(())
    }

    fn check_basic_inputs(&self, order: &Order, last_price: f64) -> Result<(), ComplianceError> {
        if order.symbol.trim().is_empty() {
            return Err(ComplianceError::FatFinger("Order symbol is empty".into()));
        }
        if !order.quantity.is_finite() || order.quantity <= 0.0 {
            return Err(ComplianceError::FatFinger(format!(
                "Invalid order quantity {}",
                order.quantity
            )));
        }
        if !last_price.is_finite() || last_price <= 0.0 {
            return Err(ComplianceError::FatFinger(format!(
                "Invalid last price {}",
                last_price
            )));
        }
        if let Some(limit) = order.limit_price {
            if !limit.is_finite() || limit <= 0.0 {
                return Err(ComplianceError::FatFinger(format!(
                    "Invalid limit price {}",
                    limit
                )));
            }
        }
        if let Some(stop) = order.stop_price {
            if !stop.is_finite() || stop <= 0.0 {
                return Err(ComplianceError::FatFinger(format!(
                    "Invalid stop price {}",
                    stop
                )));
            }
        }
        Ok(())
    }

    fn check_notional_single(&self, order: &Order, last_price: f64) -> Result<(), ComplianceError> {
        let price = order.limit_price.unwrap_or(last_price);
        let notional = price * order.quantity;
        if notional > self.config.max_single_order_notional_usd {
            return Err(ComplianceError::FatFinger(format!(
                "Single order notional ${:.0} exceeds limit ${:.0}",
                notional, self.config.max_single_order_notional_usd
            )));
        }
        Ok(())
    }

    fn check_daily_notional(&self, order: &Order, last_price: f64) -> Result<(), ComplianceError> {
        let price = order.limit_price.unwrap_or(last_price);
        let notional = price * order.quantity;
        if self.state.daily_notional_used + notional > self.config.max_daily_notional_usd {
            return Err(ComplianceError::DailyLimitBreached(format!(
                "Daily notional would reach ${:.0}, limit is ${:.0}",
                self.state.daily_notional_used + notional,
                self.config.max_daily_notional_usd
            )));
        }
        Ok(())
    }

    fn check_price_deviation(&self, order: &Order, last_price: f64) -> Result<(), ComplianceError> {
        if let Some(limit) = order.limit_price {
            let deviation = ((limit - last_price) / last_price).abs();
            if deviation > self.config.max_price_deviation_pct {
                return Err(ComplianceError::FatFinger(format!(
                    "Limit price ${:.2} deviates {:.1}% from last ${:.2} (max {:.1}%)",
                    limit,
                    deviation * 100.0,
                    last_price,
                    self.config.max_price_deviation_pct * 100.0
                )));
            }
        }
        Ok(())
    }

    fn check_fat_finger_size(&self, order: &Order) -> Result<(), ComplianceError> {
        if self.state.average_order_size > 0.0 {
            let ratio = order.quantity / self.state.average_order_size;
            if ratio > self.config.max_order_size_multiplier {
                return Err(ComplianceError::FatFinger(format!(
                    "Order size {} is {:.1}× average {:.0} (max {:.1}×)",
                    order.quantity,
                    ratio,
                    self.state.average_order_size,
                    self.config.max_order_size_multiplier
                )));
            }
        }
        Ok(())
    }

    fn check_rate_limit(&mut self, now_ts_secs: u64) -> Result<(), ComplianceError> {
        if now_ts_secs == self.state.last_second_ts {
            self.state.orders_this_second += 1;
        } else {
            self.state.orders_this_second = 1;
            self.state.last_second_ts = now_ts_secs;
        }
        if self.state.orders_this_second > self.config.max_orders_per_second {
            return Err(ComplianceError::RateLimitBreached(format!(
                "{} orders/sec exceeds limit of {}",
                self.state.orders_this_second, self.config.max_orders_per_second
            )));
        }
        Ok(())
    }

    /// Call after a fill is confirmed to track daily notional + update average size
    pub fn record_fill(&mut self, quantity: u64, fill_price: f64) {
        if quantity == 0 || !fill_price.is_finite() || fill_price <= 0.0 {
            return;
        }
        let notional = fill_price * quantity as f64;
        self.state.daily_notional_used += notional;
        // Exponential moving average of order size (α = 0.1)
        self.state.average_order_size = 0.9 * self.state.average_order_size + 0.1 * quantity as f64;
    }

    /// Call at midnight / session reset
    pub fn reset_daily(&mut self) {
        self.state.daily_notional_used = 0.0;
    }
}
