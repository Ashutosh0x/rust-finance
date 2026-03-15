// crates/oms/src/sebi.rs
//
// SEBI pre-trade compliance enforcement.
// Implements SEBI circular requirements:
// - Maximum order value limits
// - Intraday position limits (MIS/BO/CO)
// - Short-sell uptick rule enforcement
// - Bracket order validation
// - Daily traded value caps per client
// - Price band checks (circuit filters)

use std::collections::HashMap;
use chrono::{DateTime, Utc, Timelike};
use tracing::warn;

/// SEBI-mandated compliance configuration.
/// Values based on SEBI Circular SEBI/HO/MRD/DP/CIR/P/2019/116.
#[derive(Debug, Clone)]
pub struct SebiConfig {
    /// Max value of a single order (INR). SEBI default: no absolute cap,
    /// but brokers typically enforce ₹25 Cr.
    pub max_single_order_value: f64,
    /// MIS (Margin Intraday Square-off) leverage multiplier limit.
    pub mis_leverage_cap: f64,
    /// Intraday position must be squared off by this time (IST).
    pub squareoff_time_hour: u32,
    pub squareoff_time_minute: u32,
    /// Maximum allowed order quantity per scrip per day.
    pub max_daily_qty_per_scrip: f64,
    /// Maximum daily traded value per client (INR).
    pub max_daily_turnover: f64,
    /// Price band percentage — orders outside ±N% of reference price rejected.
    /// Set to 0.0 to disable.
    pub price_band_pct: f64,
}

impl Default for SebiConfig {
    fn default() -> Self {
        Self {
            max_single_order_value: 250_000_000.0, // ₹25 Cr
            mis_leverage_cap: 5.0,
            squareoff_time_hour: 15,
            squareoff_time_minute: 15,
            max_daily_qty_per_scrip: 1_000_000.0,
            max_daily_turnover: 500_000_000.0,     // ₹50 Cr
            price_band_pct: 0.20,                  // ±20% circuit filter
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OrderVariety {
    /// Regular CNC (Delivery)
    Cnc,
    /// Margin Intraday Squareoff
    Mis,
    /// Bracket Order
    Bo { target_pct: f64, stoploss_pct: f64 },
    /// Cover Order
    Co { stoploss_pct: f64 },
}

#[derive(Debug, thiserror::Error)]
pub enum SebiViolation {
    #[error("Order value ₹{value:.0} exceeds SEBI max ₹{limit:.0}")]
    OrderValueExceeded { value: f64, limit: f64 },

    #[error("Short sell rejected: uptick rule — last price {last:.2} ≤ previous {prev:.2}")]
    UptickRuleViolation { last: f64, prev: f64 },

    #[error("MIS order blocked: past square-off time {hour:02}:{minute:02} IST")]
    PastSquareoffTime { hour: u32, minute: u32 },

    #[error("Daily qty for {symbol} ({qty:.0}) exceeds limit {limit:.0}")]
    DailyQtyExceeded { symbol: String, qty: f64, limit: f64 },

    #[error("Daily turnover ₹{turnover:.0} exceeds cap ₹{cap:.0}")]
    DailyTurnoverExceeded { turnover: f64, cap: f64 },

    #[error("Price {price:.2} outside ±{band:.0}% band (ref: {reference:.2})")]
    PriceBandViolation { price: f64, reference: f64, band: f64 },

    #[error("Bracket order target {target_pct:.1}% must be greater than stoploss {sl_pct:.1}%")]
    InvalidBracketOrder { target_pct: f64, sl_pct: f64 },

    #[error("Cover order stoploss {sl_pct:.1}% must be between 0.1% and 10%")]
    InvalidCoverOrder { sl_pct: f64 },
}

/// Per-client intraday trading state.
#[derive(Debug, Default)]
struct ClientState {
    daily_turnover: f64,
    daily_qty: HashMap<String, f64>,
}

/// SEBI compliance engine.
pub struct SebiCompliance {
    cfg: SebiConfig,
    /// Last known prices: symbol → (prev_price, last_price).
    price_tape: HashMap<String, (f64, f64)>,
    /// Reference prices for circuit filters (e.g., previous close).
    reference_prices: HashMap<String, f64>,
    client_state: ClientState,
}

impl SebiCompliance {
    pub fn new(cfg: SebiConfig) -> Self {
        Self {
            cfg,
            price_tape: HashMap::new(),
            reference_prices: HashMap::new(),
            client_state: ClientState::default(),
        }
    }

    /// Update the price tape — call on every market tick.
    pub fn on_price_tick(&mut self, symbol: &str, price: f64) {
        let entry = self.price_tape.entry(symbol.to_string()).or_insert((price, price));
        entry.0 = entry.1; // prev = last
        entry.1 = price;   // last = new
    }

    /// Set the reference price (previous close) for circuit filter checks.
    pub fn set_reference_price(&mut self, symbol: &str, price: f64) {
        self.reference_prices.insert(symbol.to_string(), price);
    }

    /// Reset daily counters — call at market open (09:15 IST).
    pub fn reset_daily_counters(&mut self) {
        self.client_state = ClientState::default();
        tracing::info!("SEBI daily counters reset");
    }

    /// Run all pre-trade SEBI checks. Returns `Ok(())` if the order passes.
    pub fn check(
        &self,
        symbol: &str,
        is_sell: bool,
        quantity: f64,
        price: f64,
        variety: &OrderVariety,
        now: DateTime<Utc>,
    ) -> Result<(), SebiViolation> {
        let order_value = quantity * price;

        // ── 1. Order value cap ──────────────────────────────────────────────
        if order_value > self.cfg.max_single_order_value {
            return Err(SebiViolation::OrderValueExceeded {
                value: order_value,
                limit: self.cfg.max_single_order_value,
            });
        }

        // ── 2. MIS squareoff time check ─────────────────────────────────────
        // IST = UTC + 5:30
        let ist_hour = (now.hour() + 5) % 24;
        let ist_minute = (now.minute() + 30) % 60;
        if matches!(variety, OrderVariety::Mis | OrderVariety::Bo { .. } | OrderVariety::Co { .. }) {
            if ist_hour > self.cfg.squareoff_time_hour
                || (ist_hour == self.cfg.squareoff_time_hour
                    && ist_minute >= self.cfg.squareoff_time_minute)
            {
                return Err(SebiViolation::PastSquareoffTime {
                    hour: self.cfg.squareoff_time_hour,
                    minute: self.cfg.squareoff_time_minute,
                });
            }
        }

        // ── 3. Short sell uptick rule ───────────────────────────────────────
        if is_sell {
            if let Some((prev, last)) = self.price_tape.get(symbol) {
                if price <= *prev && *last <= *prev {
                    return Err(SebiViolation::UptickRuleViolation {
                        last: *last,
                        prev: *prev,
                    });
                }
            }
        }

        // ── 4. Daily quantity cap ───────────────────────────────────────────
        let current_daily_qty = self
            .client_state
            .daily_qty
            .get(symbol)
            .copied()
            .unwrap_or(0.0);
        if current_daily_qty + quantity > self.cfg.max_daily_qty_per_scrip {
            return Err(SebiViolation::DailyQtyExceeded {
                symbol: symbol.to_string(),
                qty: current_daily_qty + quantity,
                limit: self.cfg.max_daily_qty_per_scrip,
            });
        }

        // ── 5. Daily turnover cap ───────────────────────────────────────────
        if self.client_state.daily_turnover + order_value > self.cfg.max_daily_turnover {
            return Err(SebiViolation::DailyTurnoverExceeded {
                turnover: self.client_state.daily_turnover + order_value,
                cap: self.cfg.max_daily_turnover,
            });
        }

        // ── 6. Price band / circuit filter ──────────────────────────────────
        if self.cfg.price_band_pct > 0.0 {
            if let Some(&ref_price) = self.reference_prices.get(symbol) {
                let band = ref_price * self.cfg.price_band_pct;
                if price < ref_price - band || price > ref_price + band {
                    return Err(SebiViolation::PriceBandViolation {
                        price,
                        reference: ref_price,
                        band: self.cfg.price_band_pct * 100.0,
                    });
                }
            }
        }

        // ── 7. Bracket order validation ─────────────────────────────────────
        if let OrderVariety::Bo { target_pct, stoploss_pct } = variety {
            if target_pct <= stoploss_pct {
                return Err(SebiViolation::InvalidBracketOrder {
                    target_pct: *target_pct,
                    sl_pct: *stoploss_pct,
                });
            }
        }

        // ── 8. Cover order stoploss range ───────────────────────────────────
        if let OrderVariety::Co { stoploss_pct } = variety {
            if *stoploss_pct < 0.001 || *stoploss_pct > 0.10 {
                return Err(SebiViolation::InvalidCoverOrder {
                    sl_pct: *stoploss_pct,
                });
            }
        }

        Ok(())
    }

    /// Record an accepted order's contribution to daily limits.
    pub fn record_order(&mut self, symbol: &str, quantity: f64, price: f64) {
        self.client_state.daily_turnover += quantity * price;
        *self.client_state.daily_qty.entry(symbol.to_string()).or_insert(0.0) += quantity;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc_time(hour: u32, minute: u32) -> DateTime<Utc> {
        // Convert IST to UTC: IST = UTC+5:30
        let h = if hour >= 6 { hour - 6 } else { hour + 18 };
        Utc.with_ymd_and_hms(2024, 1, 15, h, minute, 0).unwrap()
    }

    #[test]
    fn test_order_value_exceeded() {
        let compliance = SebiCompliance::new(SebiConfig {
            max_single_order_value: 1_000.0,
            ..Default::default()
        });
        let result = compliance.check(
            "RELIANCE", false, 100.0, 20.0,
            &OrderVariety::Cnc, utc_time(10, 0),
        );
        assert!(matches!(result, Err(SebiViolation::OrderValueExceeded { .. })));
    }

    #[test]
    fn test_mis_past_squareoff_rejected() {
        let compliance = SebiCompliance::new(SebiConfig::default());
        // 15:20 IST — past 15:15 cutoff
        let result = compliance.check(
            "INFY", false, 10.0, 1500.0,
            &OrderVariety::Mis, utc_time(15, 20),
        );
        assert!(matches!(result, Err(SebiViolation::PastSquareoffTime { .. })));
    }

    #[test]
    fn test_price_band_violation() {
        let mut compliance = SebiCompliance::new(SebiConfig {
            price_band_pct: 0.10,
            ..Default::default()
        });
        compliance.set_reference_price("TATAMOTORS", 500.0);
        // Price 560 is +12% → outside ±10% band
        let result = compliance.check(
            "TATAMOTORS", false, 10.0, 560.0,
            &OrderVariety::Cnc, utc_time(10, 0),
        );
        assert!(matches!(result, Err(SebiViolation::PriceBandViolation { .. })));
    }

    #[test]
    fn test_valid_order_passes() {
        let compliance = SebiCompliance::new(SebiConfig::default());
        let result = compliance.check(
            "SBIN", false, 100.0, 700.0,
            &OrderVariety::Cnc, utc_time(10, 30),
        );
        assert!(result.is_ok());
    }
}
