use rand::Rng;
use rand::SeedableRng;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::market::MarketState;

pub type AgentId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TraderType {
    Retail,
    HedgeFund,
    MarketMaker,
    ArbitrageBot,
    MomentumTrader,
    NewsTrader,
    /// Contrarian agents invert the majority signal.
    /// They buy when others sell and sell when others buy,
    /// providing adversarial tension that prevents herding.
    Contrarian,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentAction {
    Buy {
        agent_id: AgentId,
        notional_usd: f64,
        limit_price: Option<f64>,
        reason: ActionReason,
    },
    Sell {
        agent_id: AgentId,
        notional_usd: f64,
        limit_price: Option<f64>,
        reason: ActionReason,
    },
    Hold {
        agent_id: AgentId,
    },
    ProvideLiquidity {
        agent_id: AgentId,
        bid_size: f64,
        ask_size: f64,
        spread: f64,
    },
    CrossSpread {
        agent_id: AgentId,
        notional_usd: f64,
        venue_a_price: f64,
        venue_b_price: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionReason {
    MomentumSignal { strength: f64 },
    MeanReversion { fair_value: f64, current: f64 },
    NewsShock { sentiment: f64 },
    RsiOverbought { rsi: f64 },
    RsiOversold { rsi: f64 },
    SpreadCapture,
    ArbitrageOpportunity { spread_bps: f64 },
    PanicSell,
    FomoEntry,
    VolatilityBreakout,
    RiskLimitHit { position_usd: f64, limit_usd: f64 },
    Random,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub agent_id: AgentId,
    pub trader_type: TraderType,
    pub position_usd: f64,
    pub unrealized_pnl: f64,
    pub avg_entry_price: f64,
    pub realized_pnl: f64,
    pub cash: f64,
    pub price_memory: VecDeque<f64>,
    pub last_action_round: u64,
    pub fair_value_estimate: f64,
    pub current_sentiment: f64,
    pub loss_streak: u32,
    /// Per-agent perception noise — each agent perceives prices slightly
    /// differently, preventing uniform convergence. Sampled from N(0, 0.005)
    /// at initialization time.
    pub perception_noise: f64,
}

impl AgentState {
    pub fn new(
        agent_id: AgentId,
        trader_type: TraderType,
        initial_price: f64,
        cash: f64,
        seed: u64,
    ) -> Self {
        let memory_size = match trader_type {
            TraderType::Retail => 5,
            TraderType::HedgeFund => 100,
            TraderType::MarketMaker => 20,
            TraderType::ArbitrageBot => 3,
            TraderType::MomentumTrader => 50,
            TraderType::NewsTrader => 10,
            TraderType::Contrarian => 30,
        };

        let mut price_memory = VecDeque::with_capacity(memory_size + 1);
        price_memory.push_back(initial_price);

        let (fair_value, perception_noise) = {
            let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
            let fv = initial_price * (1.0 + rng.gen_range(-0.10..0.10));
            // Perception noise: different agents see slightly different prices
            let noise = rng.gen_range(-0.005..0.005);
            (fv, noise)
        };

        Self {
            agent_id,
            trader_type,
            position_usd: 0.0,
            unrealized_pnl: 0.0,
            avg_entry_price: initial_price,
            realized_pnl: 0.0,
            cash,
            price_memory,
            last_action_round: 0,
            fair_value_estimate: fair_value,
            current_sentiment: 0.0,
            loss_streak: 0,
            perception_noise,
        }
    }

    pub fn update_pnl(&mut self, current_price: f64) {
        if self.position_usd != 0.0 && self.avg_entry_price > 0.0 {
            let return_pct = (current_price - self.avg_entry_price) / self.avg_entry_price;
            self.unrealized_pnl = self.position_usd * return_pct;
        } else {
            self.unrealized_pnl = 0.0;
        }
    }

    pub fn record_fill(&mut self, notional: f64, price: f64, is_buy: bool) {
        if is_buy {
            let new_total = self.position_usd + notional;
            if self.position_usd >= 0.0 && new_total != 0.0 {
                self.avg_entry_price =
                    (self.avg_entry_price * self.position_usd + price * notional) / new_total;
            }
            self.position_usd = new_total;
            self.cash -= notional;
        } else {
            let closed = notional.min(self.position_usd.abs());
            if self.position_usd > 0.0 && closed > 0.0 {
                let pnl = closed * (price - self.avg_entry_price) / self.avg_entry_price;
                self.realized_pnl += pnl;
            }
            self.position_usd -= notional;
            self.cash += notional;
        }
    }

    fn memory_size(&self) -> usize {
        match self.trader_type {
            TraderType::Retail => 5,
            TraderType::HedgeFund => 100,
            TraderType::MarketMaker => 20,
            TraderType::ArbitrageBot => 3,
            TraderType::MomentumTrader => 50,
            TraderType::NewsTrader => 10,
            TraderType::Contrarian => 30,
        }
    }

    pub fn observe_price(&mut self, price: f64) {
        let max = self.memory_size();
        self.price_memory.push_back(price);
        if self.price_memory.len() > max {
            self.price_memory.pop_front();
        }
    }
}

pub struct Agent {
    pub state: AgentState,
    max_position_usd: f64,
    /// Max flow any single agent can deploy per round (USD).
    max_deploy_per_round: f64,
    /// Inventory decay rate per round. Position *= (1 - decay) each round.
    inventory_decay_rate: f64,
}

impl Agent {
    pub fn new(
        id: AgentId,
        trader_type: TraderType,
        initial_price: f64,
        cash: f64,
        max_position_usd: f64,
        seed: u64,
    ) -> Self {
        Self {
            state: AgentState::new(id, trader_type, initial_price, cash, seed),
            max_position_usd,
            max_deploy_per_round: 500.0, // default, overridden by config
            inventory_decay_rate: 0.005, // default 0.5%/round
        }
    }

    /// Create with explicit v2.1 calibration parameters.
    pub fn new_calibrated(
        id: AgentId,
        trader_type: TraderType,
        initial_price: f64,
        cash: f64,
        max_position_usd: f64,
        seed: u64,
        max_deploy_per_round: f64,
        inventory_decay_rate: f64,
    ) -> Self {
        Self {
            state: AgentState::new(id, trader_type, initial_price, cash, seed),
            max_position_usd,
            max_deploy_per_round,
            inventory_decay_rate,
        }
    }

    pub fn decide(&mut self, market: &MarketState, rng: &mut impl Rng) -> AgentAction {
        // Apply perception noise — each agent sees a slightly different price
        let perceived_price = market.mid_price * (1.0 + self.state.perception_noise);
        self.state.observe_price(perceived_price);
        self.state.update_pnl(market.mid_price);

        // ── v2.1: Inventory decay — shed position slowly each round ──
        if self.inventory_decay_rate > 0.0 && self.state.position_usd.abs() > 1.0 {
            let decay_amount = self.state.position_usd * self.inventory_decay_rate;
            self.state.position_usd -= decay_amount;
            self.state.cash += decay_amount.abs();
        }

        if self.state.position_usd.abs() > self.max_position_usd * 1.5 {
            let notional = self.state.position_usd.abs() * 0.5;
            let is_buy = self.state.position_usd < 0.0;
            return if is_buy {
                AgentAction::Buy {
                    agent_id: self.state.agent_id,
                    notional_usd: notional,
                    limit_price: None,
                    reason: ActionReason::RiskLimitHit {
                        position_usd: self.state.position_usd,
                        limit_usd: self.max_position_usd,
                    },
                }
            } else {
                AgentAction::Sell {
                    agent_id: self.state.agent_id,
                    notional_usd: notional,
                    limit_price: None,
                    reason: ActionReason::RiskLimitHit {
                        position_usd: self.state.position_usd,
                        limit_usd: self.max_position_usd,
                    },
                }
            };
        }

        let action = match &self.state.trader_type {
            TraderType::Retail => self.decide_retail(market, rng),
            TraderType::HedgeFund => self.decide_hedge_fund(market, rng),
            TraderType::MarketMaker => self.decide_market_maker(market, rng),
            TraderType::ArbitrageBot => self.decide_arb_bot(market, rng),
            TraderType::MomentumTrader => self.decide_momentum(market, rng),
            TraderType::NewsTrader => self.decide_news_trader(market, rng),
            TraderType::Contrarian => self.decide_contrarian(market, rng),
        };

        // ── v2.1: Cap per-round deploy — prevents buy avalanche ──
        self.cap_notional(action)
    }

    /// Clamp notional_usd on Buy/Sell actions to max_deploy_per_round.
    fn cap_notional(&self, action: AgentAction) -> AgentAction {
        let cap = self.max_deploy_per_round;
        match action {
            AgentAction::Buy {
                agent_id,
                notional_usd,
                limit_price,
                reason,
            } => AgentAction::Buy {
                agent_id,
                notional_usd: notional_usd.min(cap),
                limit_price,
                reason,
            },
            AgentAction::Sell {
                agent_id,
                notional_usd,
                limit_price,
                reason,
            } => AgentAction::Sell {
                agent_id,
                notional_usd: notional_usd.min(cap),
                limit_price,
                reason,
            },
            other => other,
        }
    }

    fn decide_retail(&mut self, market: &MarketState, rng: &mut impl Rng) -> AgentAction {
        let id = self.state.agent_id;
        let base_size = rng.gen_range(100.0..=(self.max_position_usd * 0.10));
        let rsi = market.rsi_14();
        let noise: f64 = rng.gen_range(-15.0..15.0);
        let noisy_rsi = (rsi + noise).clamp(0.0, 100.0);

        let is_fomo = self.state.price_memory.len() >= 3 && {
            let recent = *self.state.price_memory.back().unwrap();
            let older = self.state.price_memory[0];
            recent > older * 1.02
        };

        if self.state.loss_streak >= 3 && self.state.position_usd > 0.0 {
            return AgentAction::Sell {
                agent_id: id,
                notional_usd: self.state.position_usd.abs(),
                limit_price: None,
                reason: ActionReason::PanicSell,
            };
        }

        if is_fomo && rng.gen_bool(0.6) {
            AgentAction::Buy {
                agent_id: id,
                notional_usd: base_size,
                limit_price: None,
                reason: ActionReason::FomoEntry,
            }
        } else if noisy_rsi < 35.0 {
            AgentAction::Buy {
                agent_id: id,
                notional_usd: base_size,
                limit_price: Some(market.ask * 1.001),
                reason: ActionReason::RsiOversold { rsi },
            }
        } else if noisy_rsi > 65.0 && self.state.position_usd > 0.0 {
            AgentAction::Sell {
                agent_id: id,
                notional_usd: (base_size).min(self.state.position_usd),
                limit_price: Some(market.bid * 0.999),
                reason: ActionReason::RsiOverbought { rsi },
            }
        } else {
            AgentAction::Hold { agent_id: id }
        }
    }

    fn decide_hedge_fund(&mut self, market: &MarketState, rng: &mut impl Rng) -> AgentAction {
        let id = self.state.agent_id;
        let fv = self.state.fair_value_estimate;
        let current = market.mid_price;

        if rng.gen_bool(0.05) {
            let normal = Normal::new(0.0, 0.01).unwrap();
            self.state.fair_value_estimate *= 1.0 + normal.sample(rng);
        }

        let discount = (fv - current) / current;
        let conviction = discount.abs().min(0.20);
        let size = self.max_position_usd * conviction * 2.0;

        if discount > 0.03 {
            AgentAction::Buy {
                agent_id: id,
                notional_usd: size,
                limit_price: Some(current * 1.002),
                reason: ActionReason::MeanReversion {
                    fair_value: fv,
                    current,
                },
            }
        } else if discount < -0.03 && self.state.position_usd > 0.0 {
            AgentAction::Sell {
                agent_id: id,
                notional_usd: size.min(self.state.position_usd),
                limit_price: Some(current * 0.998),
                reason: ActionReason::MeanReversion {
                    fair_value: fv,
                    current,
                },
            }
        } else {
            AgentAction::Hold { agent_id: id }
        }
    }

    fn decide_market_maker(&mut self, market: &MarketState, _rng: &mut impl Rng) -> AgentAction {
        let id = self.state.agent_id;
        let vol_premium = if market.is_high_vol() { 2.5 } else { 1.0 };
        let half_spread = market.spread * 0.5 * vol_premium;
        let quote_size = (self.max_position_usd * 0.20) / vol_premium;

        AgentAction::ProvideLiquidity {
            agent_id: id,
            bid_size: quote_size,
            ask_size: quote_size,
            spread: half_spread * 2.0,
        }
    }

    fn decide_arb_bot(&mut self, market: &MarketState, rng: &mut impl Rng) -> AgentAction {
        let id = self.state.agent_id;
        let noise_frac: f64 = rng.gen_range(-0.002..0.002);
        let venue_b_price = market.mid_price * (1.0 + noise_frac);
        let spread_bps = ((market.ask - venue_b_price) / market.ask * 10_000.0).abs();

        if spread_bps > 0.5 && self.state.cash > 10_000.0 {
            let notional = self.max_position_usd * 0.50;
            let is_buy = venue_b_price < market.bid;

            return if is_buy {
                AgentAction::Buy {
                    agent_id: id,
                    notional_usd: notional,
                    limit_price: Some(market.ask),
                    reason: ActionReason::ArbitrageOpportunity { spread_bps },
                }
            } else {
                AgentAction::CrossSpread {
                    agent_id: id,
                    notional_usd: notional,
                    venue_a_price: market.bid,
                    venue_b_price,
                }
            };
        }

        AgentAction::Hold { agent_id: id }
    }

    fn decide_momentum(&mut self, market: &MarketState, rng: &mut impl Rng) -> AgentAction {
        let id = self.state.agent_id;
        let signal = 0.6 * market.momentum_1h.signum() + 0.4 * market.momentum_1d.signum();
        let strength = (market.momentum_1h.abs() + market.momentum_1d.abs()) / 2.0;

        let vol_discount = if market.is_high_vol() { 0.4 } else { 1.0 };
        let size = self.max_position_usd * 0.30 * strength * vol_discount * 10.0;
        let size = size.min(self.max_position_usd * 0.30);

        if strength < 0.005 || rng.gen_bool(0.3) {
            return AgentAction::Hold { agent_id: id };
        }

        if signal > 0.0 {
            AgentAction::Buy {
                agent_id: id,
                notional_usd: size,
                limit_price: None,
                reason: ActionReason::MomentumSignal { strength: signal },
            }
        } else if signal < 0.0 {
            AgentAction::Sell {
                agent_id: id,
                notional_usd: size.min(self.state.position_usd.abs().max(size)),
                limit_price: None,
                reason: ActionReason::MomentumSignal { strength: signal },
            }
        } else {
            AgentAction::Hold { agent_id: id }
        }
    }

    fn decide_news_trader(&mut self, market: &MarketState, rng: &mut impl Rng) -> AgentAction {
        let id = self.state.agent_id;
        let sentiment = self.state.current_sentiment;

        if sentiment.abs() < 0.1 {
            return AgentAction::Hold { agent_id: id };
        }

        let size = self.max_position_usd * sentiment.abs() * 0.50 * rng.gen_range(0.5..1.0);

        if sentiment > 0.0 {
            AgentAction::Buy {
                agent_id: id,
                notional_usd: size,
                limit_price: Some(market.ask * 1.005),
                reason: ActionReason::NewsShock { sentiment },
            }
        } else {
            AgentAction::Sell {
                agent_id: id,
                notional_usd: size.min(self.state.position_usd.abs().max(size)),
                limit_price: Some(market.bid * 0.995),
                reason: ActionReason::NewsShock { sentiment },
            }
        }
    }

    /// Contrarian agents deliberately oppose the majority.
    /// They buy when momentum is negative (oversold reversion play)
    /// and sell when momentum is positive (overbought reversion play).
    /// This provides adversarial tension that prevents herding.
    fn decide_contrarian(&mut self, market: &MarketState, rng: &mut impl Rng) -> AgentAction {
        let id = self.state.agent_id;

        // Use perceived momentum (inverted) as the signal
        let momentum = market.momentum_1h;
        let strength = momentum.abs();

        // Mean-reversion bias: buy dips, sell rips
        let size = self.max_position_usd * 0.25 * (strength * 10.0).min(1.0);

        // Only act on medium+ momentum (don't fight noise)
        if strength < 0.003 || rng.gen_bool(0.4) {
            return AgentAction::Hold { agent_id: id };
        }

        // RSI extremes boost contrarian conviction
        let rsi = market.rsi_14();
        let rsi_boost = if rsi > 70.0 || rsi < 30.0 { 1.5 } else { 1.0 };
        let boosted_size = (size * rsi_boost).min(self.max_position_usd * 0.30);

        if momentum > 0.0 {
            // Market going up → contrarian sells (mean reversion)
            AgentAction::Sell {
                agent_id: id,
                notional_usd: boosted_size.min(self.state.position_usd.abs().max(boosted_size)),
                limit_price: Some(market.bid * 0.999),
                reason: ActionReason::MeanReversion {
                    fair_value: self.state.fair_value_estimate,
                    current: market.mid_price,
                },
            }
        } else {
            // Market going down → contrarian buys
            AgentAction::Buy {
                agent_id: id,
                notional_usd: boosted_size,
                limit_price: Some(market.ask * 1.001),
                reason: ActionReason::MeanReversion {
                    fair_value: self.state.fair_value_estimate,
                    current: market.mid_price,
                },
            }
        }
    }
}
