use rand::Rng;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Asset class for liquidity scaling — prevents ETFs and bonds
/// from behaving identically to mega-cap equities.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssetClass {
    MegaCap,   // AAPL, MSFT, NVDA, AMZN
    LargeCap,  // TSLA
    LargeEtf,  // SPY, QQQ
    MidEtf,    // IWM
    SectorEtf, // XLK, XLF, XLE
    IntlEtf,   // EEM, FXI
    Bond,      // TLT
    Commodity, // GLD
}

impl AssetClass {
    /// Estimate daily dollar volume (liquidity depth) by asset class.
    /// Used to normalise price impact — $900K flow against $500M SPY liquidity
    /// is 0.18% impact, but same flow against $30M FXI = 3%.
    pub fn liquidity_usd(&self) -> f64 {
        match self {
            AssetClass::MegaCap => 500_000_000.0,
            AssetClass::LargeCap => 200_000_000.0,
            AssetClass::LargeEtf => 300_000_000.0,
            AssetClass::MidEtf => 100_000_000.0,
            AssetClass::SectorEtf => 50_000_000.0,
            AssetClass::IntlEtf => 30_000_000.0,
            AssetClass::Bond => 200_000_000.0,
            AssetClass::Commodity => 80_000_000.0,
        }
    }

    pub fn from_symbol(symbol: &str) -> Self {
        match symbol {
            "AAPL" | "MSFT" | "AMZN" | "NVDA" => AssetClass::MegaCap,
            "TSLA" => AssetClass::LargeCap,
            "SPY" | "QQQ" => AssetClass::LargeEtf,
            "IWM" => AssetClass::MidEtf,
            "XLK" | "XLF" | "XLE" => AssetClass::SectorEtf,
            "EEM" | "FXI" => AssetClass::IntlEtf,
            "TLT" => AssetClass::Bond,
            "GLD" => AssetClass::Commodity,
            _ => AssetClass::MidEtf, // default fallback
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketState {
    pub symbol: String,
    pub initial_price: f64,
    pub mid_price: f64,
    pub bid: f64,
    pub ask: f64,
    pub spread: f64,
    pub volume_24h: f64,
    pub volatility_realized: f64,
    pub momentum_1h: f64,
    pub momentum_1d: f64,
    pub vwap: f64,
    pub order_imbalance: f64,
    pub round: u64,
    pub timestamp_ms: i64,

    /// Liquidity depth in USD — scales price impact per asset class
    pub liquidity_usd: f64,
    /// Cumulative drift from initial price (percentage)
    pub cumulative_drift_pct: f64,
    /// Number of consecutive rounds within ±5% drift
    pub consecutive_stable_rounds: u32,

    #[serde(skip)]
    pub price_history: VecDeque<f64>,
    #[serde(skip)]
    pub volume_history: VecDeque<f64>,
    #[serde(skip)]
    pub flow_history: VecDeque<f64>,
}

impl MarketState {
    pub fn new(symbol: impl Into<String>, initial_price: f64) -> Self {
        let sym: String = symbol.into();
        let asset_class = AssetClass::from_symbol(&sym);
        let spread = initial_price * 0.0005;

        Self {
            symbol: sym,
            initial_price,
            mid_price: initial_price,
            bid: initial_price - spread / 2.0,
            ask: initial_price + spread / 2.0,
            spread,
            volume_24h: 0.0,
            volatility_realized: 0.20 / (252_f64).sqrt(),
            momentum_1h: 0.0,
            momentum_1d: 0.0,
            vwap: initial_price,
            order_imbalance: 0.0,
            round: 0,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            liquidity_usd: asset_class.liquidity_usd(),
            cumulative_drift_pct: 0.0,
            consecutive_stable_rounds: 0,
            price_history: {
                let mut dq = VecDeque::with_capacity(500);
                dq.push_back(initial_price);
                dq
            },
            volume_history: VecDeque::with_capacity(500),
            flow_history: VecDeque::with_capacity(200),
        }
    }

    /// Advance market state by one round.
    ///
    /// v2.1 changes:
    ///  - Price impact is multiplicative and liquidity-scaled
    ///  - Mean reversion anchored to rolling VWAP (not just initial price)
    ///  - Hard ±max_drift_pct cap on cumulative drift
    ///  - Spread friction applied to net_flow before impact
    pub fn advance(
        &mut self,
        raw_net_flow: f64,
        lambda: f64,
        round_vol: f64,
        mean_reversion_speed: f64,
        rng: &mut impl Rng,
        max_drift_pct: f64,
        spread_bps: f64,
    ) {
        // ── Spread friction — reduces effective flow ──
        let net_flow = apply_spread_friction(raw_net_flow, self.mid_price, spread_bps);

        // ── Diffusion (Geometric Brownian Motion) ──
        let normal = Normal::new(0.0, 1.0).unwrap();
        let epsilon = normal.sample(rng);
        let diffusion = self.mid_price * round_vol * epsilon;

        // ── Multiplicative price impact — liquidity-scaled ──
        // rel_impact is a percentage. $900K against $300M liq × lambda ≈ 0.003%/round.
        let rel_impact = lambda * net_flow / self.liquidity_usd;
        let clamped_impact = rel_impact.clamp(-0.03, 0.03); // hard cap ±3% per round
        let impact = self.mid_price * clamped_impact;

        // ── Mean reversion anchored to VWAP ──
        // Percentage-based: pulls price toward rolling VWAP, not just initial seed.
        let vwap_drift = (self.mid_price - self.vwap) / self.vwap;
        let reversion = -mean_reversion_speed * vwap_drift * self.mid_price;

        let new_mid = (self.mid_price + diffusion + impact + reversion).max(0.01);

        // ── Spread adjustment ──
        let vol_scalar = (self.volatility_realized / 0.01).max(1.0);
        let imbalance_scalar = (1.0 + self.order_imbalance.abs() * 0.5).min(3.0);
        let new_spread = self.mid_price * 0.0005 * vol_scalar * imbalance_scalar;

        // ── VWAP update — exponential decay (fast response) ──
        let round_vol_usd = net_flow.abs().max(1.0);
        self.vwap = 0.95 * self.vwap + 0.05 * new_mid;
        self.volume_24h += round_vol_usd;

        // ── Momentum ──
        if self.price_history.len() >= 60 {
            let old_1h = self.price_history[self.price_history.len() - 60];
            self.momentum_1h = (new_mid - old_1h) / old_1h;
        }
        if self.price_history.len() >= 390 {
            let old_1d = self.price_history[self.price_history.len() - 390];
            self.momentum_1d = (new_mid - old_1d) / old_1d;
        }

        // ── Realized volatility ──
        if self.price_history.len() >= 2 {
            let returns: Vec<f64> = self
                .price_history
                .iter()
                .rev()
                .take(20)
                .collect::<Vec<_>>()
                .windows(2)
                .map(|w| (w[0] / w[1]).ln())
                .collect();
            if !returns.is_empty() {
                let mean = returns.iter().sum::<f64>() / returns.len() as f64;
                let variance =
                    returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
                self.volatility_realized = variance.sqrt();
            }
        }

        // ── Apply new price ──
        self.mid_price = new_mid;

        // ── Cumulative drift cap ±max_drift_pct (default ±20%) ──
        self.cumulative_drift_pct = (self.mid_price / self.initial_price - 1.0) * 100.0;
        if self.cumulative_drift_pct.abs() > max_drift_pct {
            self.mid_price = self.initial_price
                * (1.0 + self.cumulative_drift_pct.signum() * max_drift_pct / 100.0);
            self.cumulative_drift_pct = self.cumulative_drift_pct.signum() * max_drift_pct;
        }

        // ── Stability tracking ──
        if self.cumulative_drift_pct.abs() < 5.0 {
            self.consecutive_stable_rounds += 1;
        } else {
            self.consecutive_stable_rounds = 0;
        }

        self.bid = self.mid_price - new_spread / 2.0;
        self.ask = self.mid_price + new_spread / 2.0;
        self.spread = new_spread;
        self.order_imbalance = net_flow / (round_vol_usd + 1.0);
        self.round += 1;
        self.timestamp_ms = chrono::Utc::now().timestamp_millis();

        self.price_history.push_back(self.mid_price);
        if self.price_history.len() > 500 {
            self.price_history.pop_front();
        }
        self.volume_history.push_back(round_vol_usd);
        if self.volume_history.len() > 500 {
            self.volume_history.pop_front();
        }
        self.flow_history.push_back(net_flow);
        if self.flow_history.len() > 200 {
            self.flow_history.pop_front();
        }
    }

    pub fn rsi_14(&self) -> f64 {
        if self.price_history.len() < 15 {
            return 50.0;
        }
        let prices: Vec<f64> = self.price_history.iter().rev().take(15).cloned().collect();
        let mut gains = 0.0_f64;
        let mut losses = 0.0_f64;

        for i in 0..14 {
            let change = prices[i] - prices[i + 1];
            if change > 0.0 {
                gains += change;
            } else {
                losses += change.abs();
            }
        }

        if losses == 0.0 {
            return 100.0;
        }
        let rs = (gains / 14.0) / (losses / 14.0);
        100.0 - (100.0 / (1.0 + rs))
    }

    pub fn is_high_vol(&self) -> bool {
        self.volatility_realized > 0.02 / (390_f64).sqrt()
    }

    /// Compute flow slope over the last N rounds (linear regression).
    /// Positive slope = flow still accelerating (bad). Negative = mean-reverting (good).
    pub fn flow_slope(&self, window: usize) -> f64 {
        let w = self.flow_history.len().min(window);
        if w < 3 {
            return 0.0;
        }
        let slice: Vec<f64> = self.flow_history.iter().rev().take(w).cloned().collect();
        let n = slice.len() as f64;
        let mean_x = (n - 1.0) / 2.0;
        let mean_y: f64 = slice.iter().sum::<f64>() / n;
        let num: f64 = slice
            .iter()
            .enumerate()
            .map(|(i, &y)| (i as f64 - mean_x) * (y - mean_y))
            .sum();
        let den: f64 = (0..w).map(|i| (i as f64 - mean_x).powi(2)).sum();
        if den == 0.0 {
            0.0
        } else {
            num / den
        }
    }
}

/// Spread + slippage friction — reduces effective net_flow magnitude.
/// Both buy and sell flows lose a fraction to spread + slippage.
fn apply_spread_friction(flow: f64, _price: f64, spread_bps: f64) -> f64 {
    let spread_frac = spread_bps / 10_000.0;
    let slippage_pct = (flow.abs() / 1_000_000.0).min(0.001); // max 0.1% slippage
    let friction_frac = spread_frac + slippage_pct;

    // Reduce magnitude regardless of direction
    // Buy: $1M becomes $998.5K. Sell: -$1M becomes -$998.5K.
    flow * (1.0 - friction_frac)
}

#[derive(Debug, Clone)]
pub struct OrderBook {
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: f64,
    pub size: f64,
}

impl OrderBook {
    pub fn imbalance(&self, depth: usize) -> f64 {
        let bid_vol: f64 = self.bids.iter().take(depth).map(|l| l.size).sum();
        let ask_vol: f64 = self.asks.iter().take(depth).map(|l| l.size).sum();
        let total = bid_vol + ask_vol;
        if total == 0.0 {
            return 0.0;
        }
        (bid_vol - ask_vol) / total
    }

    pub fn cost_to_buy(&self, notional: f64) -> f64 {
        let mut remaining = notional;
        let mut total_cost = 0.0;
        for level in &self.asks {
            if remaining <= 0.0 {
                break;
            }
            let fill = remaining.min(level.size);
            total_cost += fill * level.price;
            remaining -= fill;
        }
        total_cost
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::SmallRng;
    use rand::SeedableRng;

    #[test]
    fn test_liquidity_by_asset_class() {
        assert!(
            AssetClass::from_symbol("SPY").liquidity_usd()
                > AssetClass::from_symbol("FXI").liquidity_usd()
        );
        assert!(
            AssetClass::from_symbol("AAPL").liquidity_usd()
                > AssetClass::from_symbol("XLE").liquidity_usd()
        );
    }

    #[test]
    fn test_drift_cap_clamps_price() {
        let mut market = MarketState::new("TEST", 100.0);
        let mut rng = SmallRng::seed_from_u64(42);

        // Push price way up with huge flow
        for _ in 0..100 {
            market.advance(10_000_000.0, 0.001, 0.001, 0.01, &mut rng, 20.0, 2.0);
        }

        // Price should be capped at +20% of initial
        assert!(
            market.mid_price <= 120.5,
            "Price {} should be capped at ~120",
            market.mid_price
        );
        assert!(market.cumulative_drift_pct <= 20.1);
    }

    #[test]
    fn test_spread_friction_reduces_flow() {
        // 5bps spread + 0.1% slippage on $1M flow = 0.15% total friction
        let friction_buy = apply_spread_friction(1_000_000.0, 100.0, 5.0);
        assert!(
            friction_buy < 1_000_000.0,
            "Buy should be reduced by friction"
        );
        assert!(
            friction_buy > 997_000.0,
            "Friction should be small: got {}",
            friction_buy
        );

        let friction_sell = apply_spread_friction(-1_000_000.0, 100.0, 5.0);
        assert!(
            friction_sell > -1_000_000.0,
            "Sell magnitude should be reduced"
        );
        assert!(
            friction_sell < -997_000.0,
            "Friction should be small: got {}",
            friction_sell
        );
    }

    #[test]
    fn test_vwap_tracks_price() {
        let mut market = MarketState::new("SPY", 100.0);
        let mut rng = SmallRng::seed_from_u64(42);

        for _ in 0..50 {
            market.advance(0.0, 0.0, 0.001, 0.0, &mut rng, 20.0, 2.0);
        }

        // VWAP should be close to current price with zero flow
        let vwap_diff = (market.vwap - market.mid_price).abs() / market.mid_price;
        assert!(
            vwap_diff < 0.05,
            "VWAP {} should track price {} closely",
            market.vwap,
            market.mid_price
        );
    }
}
