use crate::agent::{ActionReason, Agent, AgentId, TraderType};
use crate::signal::SwarmSignal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeReason {
    pub agent_id: AgentId,
    pub trader_type: String,
    pub action_summary: String,
    pub primary_reason: String,
    pub contributing_factors: Vec<String>,
    pub position_usd: f64,
    pub unrealized_pnl: f64,
    pub risk_status: RiskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskStatus {
    Healthy,
    NearLimit { pct_of_limit: f64 },
    AtLimit,
}

pub struct InterviewEngine;

impl InterviewEngine {
    pub fn interview_agent(
        agent: &Agent,
        reason: &ActionReason,
        max_position_usd: f64,
    ) -> TradeReason {
        let primary = Self::explain_reason(reason);
        let factors = Self::contributing_factors(&agent, reason);
        let risk_status = Self::assess_risk(&agent, max_position_usd);

        let action_summary = match reason {
            ActionReason::RsiOversold { rsi } => {
                format!("Bought on RSI oversold signal (RSI={:.1})", rsi)
            }
            ActionReason::RsiOverbought { rsi } => {
                format!("Sold on RSI overbought signal (RSI={:.1})", rsi)
            }
            ActionReason::MomentumSignal { strength } => {
                format!("Momentum trade, strength={:.3}", strength)
            }
            ActionReason::MeanReversion {
                fair_value,
                current,
            } => format!(
                "Mean reversion: FV={:.2} current={:.2}",
                fair_value, current
            ),
            ActionReason::PanicSell => "Capitulation sell after loss streak".to_string(),
            ActionReason::FomoEntry => {
                "FOMO buy — price breaking out in short-term memory".to_string()
            }
            ActionReason::NewsShock { sentiment } => {
                format!("News-driven trade, sentiment={:.2}", sentiment)
            }
            ActionReason::ArbitrageOpportunity { spread_bps } => {
                format!("Arbitrage: {:.2}bps spread captured", spread_bps)
            }
            ActionReason::SpreadCapture => "Market making: spread capture".to_string(),
            ActionReason::VolatilityBreakout => "Volatility breakout trade".to_string(),
            ActionReason::RiskLimitHit {
                position_usd,
                limit_usd,
            } => format!(
                "FORCED REDUCE: position ${:.0} exceeded limit ${:.0}",
                position_usd, limit_usd
            ),
            ActionReason::Random => "Random action (retail noise)".to_string(),
        };

        TradeReason {
            agent_id: agent.state.agent_id,
            trader_type: format!("{:?}", agent.state.trader_type),
            action_summary,
            primary_reason: primary,
            contributing_factors: factors,
            position_usd: agent.state.position_usd,
            unrealized_pnl: agent.state.unrealized_pnl,
            risk_status,
        }
    }

    pub fn summarise_swarm(signal: &SwarmSignal, agent_count: usize) -> String {
        let direction_str = match &signal.direction {
            crate::signal::SignalDirection::Long => "bullish",
            crate::signal::SignalDirection::Short => "bearish",
            crate::signal::SignalDirection::Neutral => "neutral",
        };

        let conviction_str = match &signal.conviction {
            crate::signal::Conviction::High => "high conviction",
            crate::signal::Conviction::Medium => "moderate conviction",
            crate::signal::Conviction::Low => "low conviction",
        };

        format!(
            "Swarm summary (round {}, {} agents): The {} agents are {}-biased with {} — {:.0}% bulls, {:.0}% bears. Net dollar flow this round: ${:+.0}K. Detected regime: {:?}. Confidence in this signal: {:.0}%. Current RSI: {:.1}. 1-hour momentum: {:+.2}%.",
            signal.round, agent_count, agent_count, direction_str, conviction_str, signal.bullish_prob * 100.0, signal.bearish_prob * 100.0, signal.net_flow_usd / 1_000.0, signal.regime, signal.confidence * 100.0, signal.rsi, signal.momentum_1h * 100.0,
        )
    }

    pub fn interview_batch(
        agents: &[Agent],
        max_position_usd: f64,
        limit: usize,
    ) -> Vec<TradeReason> {
        let dummy_reason = ActionReason::Random;
        let mut reasons: Vec<TradeReason> = agents
            .iter()
            .take(limit * 10)
            .map(|a| Self::interview_agent(a, &dummy_reason, max_position_usd))
            .collect();
        reasons.sort_by(|a, b| {
            b.unrealized_pnl
                .abs()
                .partial_cmp(&a.unrealized_pnl.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        reasons.truncate(limit);
        reasons
    }

    fn explain_reason(reason: &ActionReason) -> String {
        match reason {
            ActionReason::MomentumSignal { strength } => format!("Momentum factor fired with strength {:.3}. CTA-style trend following logic determined the directional bias from recent price history exceeded threshold.", strength),
            ActionReason::MeanReversion { fair_value, current } => { let pct = (fair_value - current) / current * 100.0; format!("Fundamental model estimates fair value at ${:.2}, representing a {:.1}% {:} from current price. Triggered mean-reversion entry.", fair_value, pct.abs(), if pct > 0.0 { "premium" } else { "discount" }) },
            ActionReason::RsiOversold { rsi } => format!("RSI({:.1}) breached the oversold threshold (35). Agent interpreted this as a short-term mean-reversion opportunity.", rsi),
            ActionReason::RsiOverbought { rsi } => format!("RSI({:.1}) exceeded overbought threshold (65). Agent trimmed position expecting near-term pullback.", rsi),
            ActionReason::PanicSell => "Loss streak threshold exceeded. Retail panic selling: 3+ consecutive declining rounds triggered capitulation.".to_string(),
            ActionReason::FomoEntry => "Price has risen >2% within the agent's short-term memory window. FOMO entry: retail agent chasing momentum without fundamental basis.".to_string(),
            ActionReason::NewsShock { sentiment } => format!("News sentiment signal = {:.2}. Agent classified as NewsTrader reacted proportionally to the injected event sentiment.", sentiment),
            ActionReason::ArbitrageOpportunity { spread_bps } => format!("Detected {:.2}bps price discrepancy across venues. ArbitrageBot crossed spread to capture risk-free profit.", spread_bps),
            ActionReason::SpreadCapture => "Market maker providing two-sided liquidity. Earning bid-ask spread as compensation for bearing inventory risk.".to_string(),
            ActionReason::RiskLimitHit { position_usd, limit_usd } => format!("RISK OVERRIDE: position of ${:.0} is {:.1}x the maximum allowed ${:.0}. Forced partial liquidation to restore compliance.", position_usd, position_usd / limit_usd, limit_usd),
            ActionReason::VolatilityBreakout => "Realized volatility breakout detected. Agent interpreted vol expansion as directional signal.".to_string(),
            ActionReason::Random => "Random action — retail noise trader with no signal conviction.".to_string(),
        }
    }

    fn contributing_factors(agent: &Agent, _reason: &ActionReason) -> Vec<String> {
        let mut factors = Vec::new();
        if agent.state.position_usd.abs() > 0.0 {
            factors.push(format!(
                "Existing position: ${:.0} ({})",
                agent.state.position_usd.abs(),
                if agent.state.position_usd > 0.0 {
                    "long"
                } else {
                    "short"
                }
            ));
        }
        if agent.state.unrealized_pnl != 0.0 {
            factors.push(format!(
                "Unrealized PnL: ${:.2} ({:.2}%)",
                agent.state.unrealized_pnl,
                agent.state.unrealized_pnl / agent.state.position_usd.abs().max(1.0) * 100.0
            ));
        }
        if agent.state.loss_streak > 0 {
            factors.push(format!(
                "Loss streak: {} consecutive losing rounds",
                agent.state.loss_streak
            ));
        }
        match &agent.state.trader_type {
            TraderType::HedgeFund => {
                factors.push(format!(
                    "Fair value estimate: ${:.2}",
                    agent.state.fair_value_estimate
                ));
            }
            TraderType::NewsTrader => {
                factors.push(format!(
                    "Current sentiment loading: {:.2}",
                    agent.state.current_sentiment
                ));
            }
            TraderType::Retail => {
                factors.push(format!(
                    "Price memory depth: {} observations",
                    agent.state.price_memory.len()
                ));
            }
            _ => {}
        }
        factors
    }

    fn assess_risk(agent: &Agent, max_position_usd: f64) -> RiskStatus {
        let pct = agent.state.position_usd.abs() / max_position_usd;
        if pct >= 1.0 {
            RiskStatus::AtLimit
        } else if pct >= 0.75 {
            RiskStatus::NearLimit { pct_of_limit: pct }
        } else {
            RiskStatus::Healthy
        }
    }
}
