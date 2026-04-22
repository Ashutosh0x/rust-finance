use crate::market::MarketState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmSignal {
    pub round: u64,
    pub symbol: String,
    pub direction: SignalDirection,
    pub conviction: Conviction,
    pub bullish_prob: f64,
    pub bearish_prob: f64,
    pub neutral_prob: f64,
    pub net_flow_usd: f64,
    pub regime: MarketRegime,
    pub confidence: f64,
    pub price: f64,
    pub volatility: f64,
    pub rsi: f64,
    pub momentum_1h: f64,
    pub momentum_1d: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SignalDirection {
    Long,
    Short,
    Neutral,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Conviction {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketRegime {
    Trending,
    MeanReverting,
    HighVolatility,
    LowVolatility,
    OrderImbalance,
}

impl SwarmSignal {
    pub fn from_round(
        round: u64,
        market: &MarketState,
        buy_fraction: f64,
        sell_fraction: f64,
        net_flow_usd: f64,
    ) -> Self {
        let neutral_prob = (1.0 - buy_fraction - sell_fraction).max(0.0);

        let direction = if buy_fraction > sell_fraction + 0.15 {
            SignalDirection::Long
        } else if sell_fraction > buy_fraction + 0.15 {
            SignalDirection::Short
        } else {
            SignalDirection::Neutral
        };

        let margin = (buy_fraction - sell_fraction).abs();
        let conviction = if margin > 0.30 {
            Conviction::High
        } else if margin > 0.15 {
            Conviction::Medium
        } else {
            Conviction::Low
        };

        let regime = detect_regime(market, net_flow_usd);

        // ── Calibrated confidence (v3.0) ─────────────────────────────────────
        // Multi-factor weighted formula with anti-herding and RSI penalties.
        // Produces 35%–92%, never 100%, never below 35%.
        //
        // Factor 1: Agent agreement (30%) — with anti-herding penalty
        //   Too-uniform agreement (>95%) is suspicious (likely herding).
        //   margin=0.0 (50/50 split) → 0.0, margin=0.5 (75/25) → high
        let raw_agreement = (margin / 0.50).min(1.0);
        let agreement_score = if raw_agreement > 0.95 {
            0.7 // suspiciously uniform — penalize
        } else if raw_agreement > 0.80 {
            raw_agreement * 0.95
        } else if raw_agreement > 0.55 {
            raw_agreement * 0.9
        } else {
            raw_agreement * 0.7 // weak consensus
        };

        // Factor 2: Flow strength (20%) — direction consistency + magnitude
        let recent_flows: Vec<f64> = market.flow_history.iter().cloned().collect();
        let flow_consistency = if recent_flows.len() >= 5 {
            let positive = recent_flows.iter().filter(|&&f| f > 0.0).count();
            let negative = recent_flows.iter().filter(|&&f| f < 0.0).count();
            let dominant = positive.max(negative) as f64;
            dominant / recent_flows.len() as f64
        } else {
            0.5
        };
        let flow_magnitude = (net_flow_usd.abs() / (market.liquidity_usd * 0.001)).min(1.0);
        let flow_score = 0.6 * flow_consistency + 0.4 * flow_magnitude;

        // Factor 3: Drift stability (15%) — lower drift = more trustworthy
        let drift_score = (1.0 - (market.cumulative_drift_pct.abs() / 5.0)).clamp(0.0, 1.0);

        // Factor 4: Volatility penalty (15%) — high vol = less certain
        let vol_ann = market.volatility_realized * (252_f64).sqrt();
        let vol_score = (1.0 - vol_ann.min(1.0)).max(0.2);

        // Factor 5: RSI mean-reversion penalty (20%) — NEW
        //   Extreme RSI reduces confidence in trend continuation.
        //   RSI near 50 = neutral (highest score). RSI > 70 or < 30 = penalized.
        let rsi = market.rsi_14();
        let rsi_score = if rsi > 70.0 || rsi < 30.0 {
            0.5 // overbought/oversold = lower confidence in trend continuation
        } else if rsi > 60.0 || rsi < 40.0 {
            0.7 // mild extremes
        } else {
            0.8 + (50.0 - (rsi - 50.0).abs()) / 250.0 // near 50 = best
        };

        let raw_confidence = 0.30 * agreement_score
            + 0.20 * flow_score
            + 0.15 * drift_score
            + 0.15 * vol_score
            + 0.20 * rsi_score;

        // Clamp to [0.35, 0.92] — never 100%, never below 35%
        let confidence = raw_confidence.clamp(0.35, 0.92);

        SwarmSignal {
            round,
            symbol: market.symbol.clone(),
            direction,
            conviction,
            bullish_prob: buy_fraction,
            bearish_prob: sell_fraction,
            neutral_prob,
            net_flow_usd,
            regime,
            confidence,
            price: market.mid_price,
            volatility: market.volatility_realized,
            rsi: market.rsi_14(),
            momentum_1h: market.momentum_1h,
            momentum_1d: market.momentum_1d,
        }
    }

    pub fn to_prompt_context(&self) -> String {
        format!(
            "[SwarmSim Round {}] {} | Direction: {:?} ({:?}) | Bulls: {:.0}% Bears: {:.0}% | Net flow: ${:.0}K | Regime: {:?} | Confidence: {:.0}% | RSI: {:.1} | Mom1H: {:.2}% | Vol: {:.2}%",
            self.round, self.symbol, self.direction, self.conviction, self.bullish_prob * 100.0, self.bearish_prob * 100.0, self.net_flow_usd / 1000.0, self.regime, self.confidence * 100.0, self.rsi, self.momentum_1h * 100.0, self.volatility * 100.0,
        )
    }

    pub fn is_actionable(&self) -> bool {
        self.conviction != Conviction::Low
            && !matches!(self.regime, MarketRegime::HighVolatility)
            && self.confidence > 0.40
    }
}

fn detect_regime(market: &MarketState, net_flow: f64) -> MarketRegime {
    let flow_fraction = (net_flow.abs() / 1_000_000.0).min(1.0);

    if market.is_high_vol() {
        MarketRegime::HighVolatility
    } else if flow_fraction > 0.7 {
        MarketRegime::OrderImbalance
    } else if market.momentum_1h.abs() > 0.01 {
        MarketRegime::Trending
    } else if market.volatility_realized < 0.003 {
        MarketRegime::LowVolatility
    } else {
        MarketRegime::MeanReverting
    }
}
