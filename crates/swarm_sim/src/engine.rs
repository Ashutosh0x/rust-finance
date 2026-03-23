use std::sync::Arc;
use std::time::Duration;

use rayon::prelude::*;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use tracing::{debug, info, warn};

use crate::agent::{Agent, AgentAction, AgentId, TraderType};
use crate::config::SwarmConfig;
use crate::market::MarketState;
use crate::persistence::ActionLog;
use crate::signal::SwarmSignal;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmStep {
    pub round: u64,
    pub signal: SwarmSignal,
    pub actions_count: usize,
    pub net_flow_usd: f64,
    pub buy_count: usize,
    pub sell_count: usize,
    pub hold_count: usize,
    pub price_after: f64,
    pub realized_pnl_total: f64,
}

pub struct SwarmEngine {
    pub config: SwarmConfig,
    pub market: MarketState,
    agents: Vec<Agent>,
    action_log: ActionLog,
    pub round: u64,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

impl SwarmEngine {
    pub fn new(config: SwarmConfig, initial_market: MarketState) -> Self {
        config.validate().expect("SwarmConfig validation failed");

        info!("Initializing SwarmEngine: {} agents, symbol={}", config.agent_count, initial_market.symbol);

        let agents = Self::spawn_agents(&config, initial_market.mid_price);
        let action_log = ActionLog::new(&config.db_path, config.db_batch_size);

        Self {
            config,
            market: initial_market,
            agents,
            action_log,
            round: 0,
            shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn spawn_agents(config: &SwarmConfig, initial_price: f64) -> Vec<Agent> {
        let n = config.agent_count;
        let cash_per_agent = 500_000.0 / n as f64 * 1000.0; 

        let counts = AgentCounts::from_config(config);
        let mut agents = Vec::with_capacity(n);
        let mut id = 0u64;

        macro_rules! spawn_type {
            ($typ:expr, $count:expr) => {
                for _ in 0..$count {
                    agents.push(Agent::new(id, $typ, initial_price, cash_per_agent, config.max_position_usd));
                    id += 1;
                }
            };
        }

        spawn_type!(TraderType::Retail, counts.retail);
        spawn_type!(TraderType::HedgeFund, counts.hedge_fund);
        spawn_type!(TraderType::MarketMaker, counts.market_maker);
        spawn_type!(TraderType::ArbitrageBot, counts.arb);
        spawn_type!(TraderType::MomentumTrader, counts.momentum);
        spawn_type!(TraderType::NewsTrader, counts.news);

        info!("Spawned {} agents: {:?}", agents.len(), counts);
        agents
    }

    pub async fn run_forever(mut self, step_tx: broadcast::Sender<SwarmStep>) {
        info!("SwarmEngine started — {} agents, round_delay={}ms", self.agents.len(), self.config.round_delay_ms);

        loop {
            if self.shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                info!("SwarmEngine shutting down gracefully at round {}", self.round);
                self.action_log.flush().await;
                break;
            }

            let step = self.step_round();
            debug!("Round {}: flow={:.0} dir={:?} conv={:?}", step.round, step.net_flow_usd, step.signal.direction, step.signal.conviction);

            if step.round % self.config.signal_emit_interval as u64 == 0 {
                if let Err(e) = step_tx.send(step.clone()) {
                    warn!("SwarmEngine: no receivers on step channel: {}", e);
                }
            }

            self.action_log.flush_if_ready().await;

            if self.config.round_delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.config.round_delay_ms)).await;
            } else {
                tokio::task::yield_now().await;
            }
        }
    }

    pub fn step_round(&mut self) -> SwarmStep {
        let round = self.round;
        let activation_prob = self.activation_probability(round);

        let market_snapshot = self.market.clone();

        let action_results: Vec<(usize, AgentAction)> = self.agents.par_iter_mut().enumerate().filter_map(|(idx, agent)| {
            let mut rng = SmallRng::seed_from_u64(round.wrapping_mul(100_003).wrapping_add(idx as u64));
            let prob = rand_distr::Bernoulli::new(activation_prob).unwrap();
            if !rand_distr::Distribution::sample(&prob, &mut rng) { return None; }

            let action = agent.decide(&market_snapshot, &mut rng);
            Some((idx, action))
        }).collect();

        let mut net_flow_usd = 0.0_f64;
        let mut buy_count = 0usize;
        let mut sell_count = 0usize;
        let mut hold_count = 0usize;
        let mut log_entries = Vec::with_capacity(action_results.len());

        for (idx, action) in &action_results {
            let agent = &mut self.agents[*idx];

            match action {
                AgentAction::Buy { notional_usd, .. } => {
                    net_flow_usd += notional_usd;
                    buy_count += 1;
                    agent.state.record_fill(*notional_usd, market_snapshot.ask, true);
                    agent.state.last_action_round = round;
                }
                AgentAction::Sell { notional_usd, .. } => {
                    net_flow_usd -= notional_usd;
                    sell_count += 1;
                    agent.state.record_fill(*notional_usd, market_snapshot.bid, false);
                    agent.state.last_action_round = round;
                }
                AgentAction::Hold { .. } => { hold_count += 1; }
                AgentAction::CrossSpread { .. } => { sell_count += 1; buy_count += 1; }
                AgentAction::ProvideLiquidity { .. } => {}
            }

            log_entries.push(crate::persistence::ActionEntry {
                round,
                agent_id: *idx as AgentId,
                trader_type: format!("{:?}", agent.state.trader_type),
                action_json: serde_json::to_string(action).unwrap_or_default(),
                price: market_snapshot.mid_price,
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
            });
        }

        let mut rng = SmallRng::seed_from_u64(round.wrapping_mul(7_919));
        self.market.advance(net_flow_usd, self.config.price_impact_lambda, self.config.round_vol(), &mut rng);

        let total_actions = buy_count + sell_count + 1;
        let buy_fraction = buy_count as f64 / total_actions as f64;
        let sell_fraction = sell_count as f64 / total_actions as f64;

        let signal = SwarmSignal::from_round(round, &self.market, buy_fraction, sell_fraction, net_flow_usd);

        self.action_log.push_batch(log_entries);

        let realized_pnl_total: f64 = self.agents.iter().map(|a| a.state.realized_pnl).sum();

        self.round += 1;

        SwarmStep { round, signal, actions_count: action_results.len(), net_flow_usd, buy_count, sell_count, hold_count, price_after: self.market.mid_price, realized_pnl_total }
    }

    pub fn inject_sentiment(&mut self, sentiment: f64) {
        for agent in &mut self.agents {
            if matches!(agent.state.trader_type, TraderType::NewsTrader) {
                agent.state.current_sentiment = sentiment;
            }
            if matches!(agent.state.trader_type, TraderType::Retail) {
                agent.state.current_sentiment = sentiment * 0.3;
            }
        }
    }

    pub fn shutdown_handle(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.shutdown.clone()
    }

    fn activation_probability(&self, round: u64) -> f64 {
        let round_of_day = round % self.config.rounds_per_day as u64;
        let is_peak = round_of_day < 120 || round_of_day > 330;
        if is_peak { (self.config.activation_prob * self.config.peak_hour_multiplier).min(1.0) } else { self.config.activation_prob }
    }

    pub fn stats(&self) -> EngineStats {
        let long_agents = self.agents.iter().filter(|a| a.state.position_usd > 0.0).count();
        let short_agents = self.agents.iter().filter(|a| a.state.position_usd < 0.0).count();
        let total_long_usd: f64 = self.agents.iter().filter(|a| a.state.position_usd > 0.0).map(|a| a.state.position_usd).sum();
        let total_short_usd: f64 = self.agents.iter().filter(|a| a.state.position_usd < 0.0).map(|a| a.state.position_usd.abs()).sum();

        EngineStats {
            round: self.round,
            agent_count: self.agents.len(),
            long_agents,
            short_agents,
            flat_agents: self.agents.len() - long_agents - short_agents,
            total_long_usd,
            total_short_usd,
            mid_price: self.market.mid_price,
            volatility: self.market.volatility_realized,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineStats {
    pub round: u64,
    pub agent_count: usize,
    pub long_agents: usize,
    pub short_agents: usize,
    pub flat_agents: usize,
    pub total_long_usd: f64,
    pub total_short_usd: f64,
    pub mid_price: f64,
    pub volatility: f64,
}

#[derive(Debug)]
struct AgentCounts {
    retail: usize,
    hedge_fund: usize,
    market_maker: usize,
    arb: usize,
    momentum: usize,
    news: usize,
}

impl AgentCounts {
    fn from_config(cfg: &SwarmConfig) -> Self {
        let n = cfg.agent_count;
        let retail = (n as f64 * cfg.retail_fraction) as usize;
        let hf = (n as f64 * cfg.hedge_fund_fraction) as usize;
        let mm = (n as f64 * cfg.market_maker_fraction) as usize;
        let arb = (n as f64 * cfg.arbitrage_fraction) as usize;
        let mom = (n as f64 * cfg.momentum_fraction) as usize;
        let news = n.saturating_sub(retail + hf + mm + arb + mom);
        Self { retail, hedge_fund: hf, market_maker: mm, arb, momentum: mom, news }
    }
}
