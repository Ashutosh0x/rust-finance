// ============================================================
// crates/risk/src/gate.rs
//
// Risk Gate — evaluates fused signal before any execution.
// This is the critical safety layer between AI and real capital.
//
// Checks (in order, all must pass):
//   1. VaR check (position won't breach daily VaR limit)
//   2. Drawdown halt (if max drawdown hit, no new longs)
//   3. Swarm–Dexter alignment (both must agree on direction)
//   4. Confidence gate (both must exceed minimums)
//   5. Volatility circuit breaker (no new positions in vol spike)
//   6. Position size normalisation (Kelly criterion cap)
// ============================================================

#[allow(unused_imports)]
use ai::dexter::Recommendation;
use ai::dexter::{DexterSignal, TradeDirection};
use swarm_sim::signal::{Conviction, SignalDirection, SwarmSignal};

#[derive(Debug, Clone)]
pub struct OrderRequest {
    pub symbol: String,
    pub side: OrderSide,
    pub notional_usd: f64,
    pub limit_price: Option<f64>,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub time_in_force: TimeInForce,
    pub source: String,
}

#[derive(Debug, Clone)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub enum TimeInForce {
    Day,
}

pub trait QuantSnapshotLike {
    fn garch_vol_forecast(&self) -> f64;
}

#[derive(Debug)]
pub enum RiskVerdict {
    Approved(OrderRequest),
    Rejected(String),
    Hedge(OrderRequest),
}

#[derive(Debug, Clone)]
pub struct RiskConfig {
    pub max_daily_var_pct: f64,     // e.g. 0.02 = 2% of NAV
    pub max_drawdown_halt: f64,     // e.g. 0.08 = halt at -8% drawdown
    pub min_dexter_confidence: f64, // e.g. 0.65
    pub min_swarm_confidence: f64,  // e.g. 0.60
    pub vol_circuit_breaker: f64,   // e.g. 0.04 = 4% daily vol = no new positions
    pub max_position_pct: f64,      // e.g. 0.05 = max 5% per position
    pub kelly_fraction: f64,        // e.g. 0.25 = quarter-Kelly
    pub portfolio_nav: f64,         // current portfolio NAV in USD
    pub current_drawdown: f64,      // current drawdown from peak
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_daily_var_pct: 0.02,
            max_drawdown_halt: 0.08,
            min_dexter_confidence: 0.65,
            min_swarm_confidence: 0.60,
            vol_circuit_breaker: 0.04,
            max_position_pct: 0.05,
            kelly_fraction: 0.25,
            portfolio_nav: 100_000.0,
            current_drawdown: 0.0,
        }
    }
}

pub fn evaluate<Q: QuantSnapshotLike>(
    dexter: &DexterSignal,
    swarm: &SwarmSignal,
    quant: &Q,
) -> RiskVerdict {
    let config = RiskConfig::default(); // In production: inject from daemon state
    evaluate_with_config(dexter, swarm, quant, &config)
}

pub fn evaluate_with_config<Q: QuantSnapshotLike>(
    dexter: &DexterSignal,
    swarm: &SwarmSignal,
    quant: &Q,
    config: &RiskConfig,
) -> RiskVerdict {
    // ── Check 1: Drawdown halt ─────────────────────────────────────────────
    if config.current_drawdown >= config.max_drawdown_halt {
        return RiskVerdict::Rejected(format!(
            "Drawdown halt: {:.1}% drawdown exceeds {:.1}% limit",
            config.current_drawdown * 100.0,
            config.max_drawdown_halt * 100.0
        ));
    }

    // ── Check 2: Volatility circuit breaker ───────────────────────────────
    // GARCH forecast is annualized; convert to daily
    let daily_vol = quant.garch_vol_forecast() / (252_f64).sqrt();
    if daily_vol > config.vol_circuit_breaker {
        // Don't reject — hedge instead
        return RiskVerdict::Hedge(build_hedge_order(dexter, config));
    }

    // ── Check 3: Neutral signals → hold ───────────────────────────────────
    if dexter.direction == TradeDirection::Neutral || swarm.direction == SignalDirection::Neutral {
        return RiskVerdict::Rejected("Both signals neutral — no trade".to_string());
    }

    // ── Check 4: Dexter–Swarm alignment ───────────────────────────────────
    let aligned = matches!(
        (&dexter.direction, &swarm.direction),
        (TradeDirection::Long, SignalDirection::Long)
            | (TradeDirection::Short, SignalDirection::Short)
    );

    if !aligned {
        return RiskVerdict::Rejected(format!(
            "Signal conflict: Dexter={:?} vs Swarm={:?}",
            dexter.direction, swarm.direction
        ));
    }

    // ── Check 5: Confidence gates ──────────────────────────────────────────
    if dexter.confidence < config.min_dexter_confidence {
        return RiskVerdict::Rejected(format!(
            "Dexter confidence {:.2} < minimum {:.2}",
            dexter.confidence, config.min_dexter_confidence
        ));
    }

    if swarm.confidence < config.min_swarm_confidence {
        return RiskVerdict::Rejected(format!(
            "Swarm confidence {:.2} < minimum {:.2}",
            swarm.confidence, config.min_swarm_confidence
        ));
    }

    // ── Check 6: Low swarm conviction → reduce size ────────────────────────
    let conviction_scalar = match swarm.conviction {
        Conviction::High => 1.0,
        Conviction::Medium => 0.6,
        Conviction::Low => 0.3,
    };

    // ── Check 7: Kelly criterion position sizing ───────────────────────────
    // f* = (p*b - q) / b  where b = take_profit/stop ratio, p = win_prob
    let win_prob = dexter.confidence;
    let lose_prob = 1.0 - win_prob;
    let reward_risk = (dexter.take_profit - dexter.entry_price).abs()
        / (dexter.entry_price - dexter.stop_loss).abs().max(0.001);
    let kelly_f = (win_prob * reward_risk - lose_prob) / reward_risk;
    let kelly_capped = (kelly_f * config.kelly_fraction)
        .min(config.max_position_pct)
        .max(0.0);

    // Apply conviction scalar
    let final_size_pct = (kelly_capped * conviction_scalar).min(dexter.position_size_pct);
    let notional = config.portfolio_nav * final_size_pct;

    if notional < 100.0 {
        return RiskVerdict::Rejected(format!(
            "Position too small after risk adjustments: ${:.2}",
            notional
        ));
    }

    // ── Check 8: VaR check ─────────────────────────────────────────────────
    // 1-day 95% VaR = notional * daily_vol * 1.645
    let var_1d = notional * daily_vol * 1.645;
    let var_limit = config.portfolio_nav * config.max_daily_var_pct;
    if var_1d > var_limit {
        let scaled_notional = var_limit / (daily_vol * 1.645);
        let var_size_pct = scaled_notional / config.portfolio_nav;
        return RiskVerdict::Approved(build_order(dexter, var_size_pct, config));
    }

    RiskVerdict::Approved(build_order(dexter, final_size_pct, config))
}

fn build_order(signal: &DexterSignal, size_pct: f64, config: &RiskConfig) -> OrderRequest {
    OrderRequest {
        symbol: signal.symbol.clone(),
        side: match signal.direction {
            TradeDirection::Long => OrderSide::Buy,
            TradeDirection::Short => OrderSide::Sell,
            TradeDirection::Neutral => OrderSide::Buy,
        },
        notional_usd: config.portfolio_nav * size_pct,
        limit_price: None, // market order for momentum; add limit for swing
        stop_loss: Some(signal.stop_loss),
        take_profit: Some(signal.take_profit),
        time_in_force: TimeInForce::Day,
        source: "HybridPipeline".to_string(),
    }
}

fn build_hedge_order(signal: &DexterSignal, config: &RiskConfig) -> OrderRequest {
    // Reduce existing position by 50% in vol spike
    OrderRequest {
        symbol: signal.symbol.clone(),
        side: OrderSide::Sell,
        notional_usd: config.portfolio_nav * 0.025, // half of max position
        limit_price: None,
        stop_loss: None,
        take_profit: None,
        time_in_force: TimeInForce::Day,
        source: "VolCircuitBreaker".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai::dexter::{DexterSignal, Recommendation, TimeHorizon, TradeDirection};
    use swarm_sim::signal::{SignalDirection, SwarmSignal};

    struct MockQuant {
        garch_vol_forecast: f64,
    }

    impl QuantSnapshotLike for MockQuant {
        fn garch_vol_forecast(&self) -> f64 {
            self.garch_vol_forecast
        }
    }

    fn make_long_signal() -> DexterSignal {
        DexterSignal {
            symbol: "NVDA".into(),
            direction: TradeDirection::Long,
            confidence: 0.78,
            entry_price: 900.0,
            stop_loss: 865.0,
            take_profit: 970.0,
            position_size_pct: 0.04,
            time_horizon: TimeHorizon::Swing,
            thesis: "NVDA RSI oversold at 28.3, swarm 71% bullish, TSMC supply intact.".into(),
            key_risks: vec!["China export controls".into()],
            catalyst: Some("H200 demand announcement".into()),
            valuation: None,
            recommendation: Recommendation::Buy,
        }
    }

    fn make_bullish_swarm() -> SwarmSignal {
        let market = swarm_sim::market::MarketState::new("NVDA", 900.0);
        SwarmSignal::from_round(42, &market, 0.71, 0.15, 800_000.0)
    }

    fn make_quant() -> MockQuant {
        MockQuant {
            garch_vol_forecast: 0.018, // 1.8% daily vol
        }
    }

    #[test]
    fn aligned_signals_approved() {
        let verdict = evaluate(&make_long_signal(), &make_bullish_swarm(), &make_quant());
        assert!(matches!(verdict, RiskVerdict::Approved(_)));
    }

    #[test]
    fn vol_spike_triggers_hedge() {
        let mut quant = make_quant();
        // garch_vol_forecast is annualized; gate converts to daily via / √252.
        // Need daily_vol > 0.04 breaker, so annualized > 0.04 * √252 ≈ 0.635.
        quant.garch_vol_forecast = 0.80; // 80% annualized → 5.0% daily > 4% breaker
        let verdict = evaluate(&make_long_signal(), &make_bullish_swarm(), &quant);
        assert!(matches!(verdict, RiskVerdict::Hedge(_)));
    }

    #[test]
    fn drawdown_halt_blocks_all() {
        let config = RiskConfig {
            current_drawdown: 0.10, // 10% drawdown, above 8% halt
            ..RiskConfig::default()
        };
        let verdict = evaluate_with_config(
            &make_long_signal(),
            &make_bullish_swarm(),
            &make_quant(),
            &config,
        );
        assert!(matches!(verdict, RiskVerdict::Rejected(_)));
    }

    #[test]
    fn kelly_sizing_is_bounded() {
        let order = build_order(&make_long_signal(), 0.04, &RiskConfig::default());
        assert!(order.notional_usd <= 100_000.0 * 0.05); // max 5% of NAV
    }

    /// When stop_loss == entry_price (zero risk distance), the `.max(0.001)` guard
    /// prevents division by zero. Position size should still be bounded.
    #[test]
    fn test_kelly_zero_variance() {
        let mut signal = make_long_signal();
        signal.stop_loss = signal.entry_price; // zero distance
        let config = RiskConfig::default();
        let quant = make_quant();
        let swarm = make_bullish_swarm();
        let verdict = evaluate_with_config(&signal, &swarm, &quant, &config);
        // Should not panic; should produce Approved or Rejected with bounded size
        match verdict {
            RiskVerdict::Approved(order) => {
                assert!(
                    order.notional_usd.is_finite() && order.notional_usd >= 0.0,
                    "Zero-variance Kelly should produce finite size: {}",
                    order.notional_usd
                );
                assert!(
                    order.notional_usd <= config.portfolio_nav * config.max_position_pct,
                    "Must respect max position cap"
                );
            }
            RiskVerdict::Rejected(_) => {} // Also acceptable
            RiskVerdict::Hedge(_) => {}
        }
    }

    /// When win probability is below breakeven, Kelly fraction should be ≤ 0,
    /// which the `.max(0.0)` clamp catches. Position should be rejected or zero-sized.
    #[test]
    fn test_kelly_negative_edge() {
        let mut signal = make_long_signal();
        signal.confidence = 0.20; // way below breakeven
        signal.take_profit = 910.0; // tiny reward
        signal.stop_loss = 850.0; // big risk
        let config = RiskConfig::default();
        let quant = make_quant();
        let swarm = make_bullish_swarm();
        let verdict = evaluate_with_config(&signal, &swarm, &quant, &config);
        match verdict {
            RiskVerdict::Rejected(reason) => {
                // Expected — negative edge should produce tiny/rejected position
                assert!(
                    reason.contains("confidence") || reason.contains("too small"),
                    "Should reject for low confidence or zero size, got: {}",
                    reason
                );
            }
            RiskVerdict::Approved(order) => {
                // If it somehow passes, the notional must be very small (kelly_f ≤ 0 → clamped to 0)
                assert!(
                    order.notional_usd < 1000.0,
                    "Negative edge should produce minimal position: ${}",
                    order.notional_usd
                );
            }
            _ => {}
        }
    }

    /// At extreme confidence (0.99), position size must still respect max_position_pct (5%).
    #[test]
    fn test_kelly_extreme_confidence() {
        let mut signal = make_long_signal();
        signal.confidence = 0.99; // near certainty
        signal.position_size_pct = 0.50; // asking for 50%
        let config = RiskConfig::default();
        let quant = make_quant();
        let swarm = make_bullish_swarm();
        let verdict = evaluate_with_config(&signal, &swarm, &quant, &config);
        if let RiskVerdict::Approved(order) = verdict {
            let position_pct = order.notional_usd / config.portfolio_nav;
            assert!(
                position_pct <= config.max_position_pct + 0.001,
                "Position {:.2}% exceeds max {:.2}%",
                position_pct * 100.0,
                config.max_position_pct * 100.0
            );
        }
    }

    /// Signal conflict between Dexter and Swarm must be rejected.
    #[test]
    fn test_signal_conflict_rejected() {
        let signal = make_long_signal(); // Dexter says Long
        let market = swarm_sim::market::MarketState::new("NVDA", 900.0);
        let mut swarm = SwarmSignal::from_round(42, &market, 0.10, 0.75, 800_000.0);
        swarm.direction = SignalDirection::Short; // Swarm says Short
        let quant = make_quant();
        let config = RiskConfig::default();
        let verdict = evaluate_with_config(&signal, &swarm, &quant, &config);
        assert!(
            matches!(verdict, RiskVerdict::Rejected(_)),
            "Signal conflict should be rejected"
        );
    }

    /// Neutral signal should be rejected.
    #[test]
    fn test_neutral_signal_rejected() {
        let mut signal = make_long_signal();
        signal.direction = TradeDirection::Neutral;
        let swarm = make_bullish_swarm();
        let quant = make_quant();
        let verdict = evaluate(&signal, &swarm, &quant);
        assert!(matches!(verdict, RiskVerdict::Rejected(_)));
    }
}
