use serde::{Deserialize, Serialize};
use tracing::info;

use crate::engine::SwarmEngine;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketScenario {
    FedRateHike {
        bps: u32,
    },
    FedRateCut {
        bps: u32,
    },
    CpiSurprise {
        actual: f64,
        expected: f64,
    },
    EarningsBeat {
        symbol: String,
        surprise_pct: f64,
    },
    EarningsMiss {
        symbol: String,
        surprise_pct: f64,
    },
    DividendCut {
        symbol: String,
    },
    MergerAnnouncement {
        acquirer: String,
        target: String,
        premium_pct: f64,
    },
    OilSupplyShock {
        change_pct: f64,
    },
    GeopoliticalCrisis {
        severity: f64,
    },
    ExchangeCollapse {
        exchange: String,
    },
    RegulatoryCrackdown {
        severity: f64,
    },
    LiquidityVacuum,
    FlashCrash {
        drop_pct: f64,
    },
    ShortSqueeze {
        symbol: String,
        move_pct: f64,
    },
    Custom {
        name: String,
        price_shock_pct: f64,
        sentiment: f64,
        volatility_multiplier: f64,
        duration_rounds: u32,
    },
}

#[derive(Debug, Clone)]
pub struct ScenarioEffect {
    pub name: String,
    pub price_shock_pct: f64,
    pub sentiment: f64,
    pub volatility_multiplier: f64,
    pub duration_rounds: u32,
}

pub struct ScenarioEngine {
    active_effects: Vec<(ScenarioEffect, u32)>,
}

impl ScenarioEngine {
    pub fn new() -> Self {
        Self {
            active_effects: Vec::new(),
        }
    }

    pub fn apply(&mut self, scenario: MarketScenario, engine: &mut SwarmEngine) {
        let effect = Self::resolve(scenario);
        info!(
            "Applying scenario '{}': shock={:.1}% sentiment={:.2} vol_mult={:.1}x for {} rounds",
            effect.name,
            effect.price_shock_pct * 100.0,
            effect.sentiment,
            effect.volatility_multiplier,
            effect.duration_rounds
        );

        if effect.price_shock_pct != 0.0 {
            let new_price = engine.market.mid_price * (1.0 + effect.price_shock_pct);
            let shock = new_price - engine.market.mid_price;
            let synthetic_flow = shock / engine.config.price_impact_lambda;
            let mut rng = rand::thread_rng();
            engine.market.advance(
                synthetic_flow,
                engine.config.price_impact_lambda,
                0.0,
                0.0,
                &mut rng,
                engine.config.max_drift_pct,
                engine.config.spread_bps_for(&engine.market.symbol),
            );
        }

        engine.inject_sentiment(effect.sentiment);
        self.active_effects.push((effect, 0));
    }

    pub fn tick(&mut self, engine: &mut SwarmEngine) {
        self.active_effects.retain_mut(|(effect, elapsed)| {
            *elapsed += 1;
            let progress = *elapsed as f64 / effect.duration_rounds as f64;
            let decayed_sentiment = effect.sentiment * (1.0 - progress);
            engine.inject_sentiment(decayed_sentiment);
            *elapsed < effect.duration_rounds
        });
    }

    fn resolve(scenario: MarketScenario) -> ScenarioEffect {
        match scenario {
            MarketScenario::FedRateHike { bps } => ScenarioEffect {
                name: format!("Fed rate hike {}bps", bps),
                price_shock_pct: -(bps as f64 * 0.001),
                sentiment: -0.6,
                volatility_multiplier: 2.0,
                duration_rounds: 60,
            },
            MarketScenario::FedRateCut { bps } => ScenarioEffect {
                name: format!("Fed rate cut {}bps", bps),
                price_shock_pct: bps as f64 * 0.0008,
                sentiment: 0.7,
                volatility_multiplier: 1.5,
                duration_rounds: 40,
            },
            MarketScenario::CpiSurprise { actual, expected } => {
                let surprise = (actual - expected) / expected;
                ScenarioEffect {
                    name: format!("CPI surprise {:.1} vs {:.1}", actual, expected),
                    price_shock_pct: -surprise * 2.0,
                    sentiment: -surprise.signum() * 0.5,
                    volatility_multiplier: 2.5,
                    duration_rounds: 30,
                }
            }
            MarketScenario::EarningsBeat {
                symbol,
                surprise_pct,
            } => ScenarioEffect {
                name: format!("{} earnings beat +{:.1}%", symbol, surprise_pct),
                price_shock_pct: surprise_pct * 0.01 * 0.5,
                sentiment: 0.8,
                volatility_multiplier: 2.0,
                duration_rounds: 45,
            },
            MarketScenario::EarningsMiss {
                symbol,
                surprise_pct,
            } => ScenarioEffect {
                name: format!("{} earnings miss -{:.1}%", symbol, surprise_pct),
                price_shock_pct: -surprise_pct * 0.01 * 0.6,
                sentiment: -0.8,
                volatility_multiplier: 2.5,
                duration_rounds: 45,
            },
            MarketScenario::OilSupplyShock { change_pct } => ScenarioEffect {
                name: format!("Oil supply shock: oil {:+.1}%", change_pct),
                price_shock_pct: -change_pct * 0.001,
                sentiment: if change_pct > 0.0 { -0.4 } else { 0.2 },
                volatility_multiplier: 1.8,
                duration_rounds: 120,
            },
            MarketScenario::ExchangeCollapse { exchange } => ScenarioEffect {
                name: format!("{} exchange collapse", exchange),
                price_shock_pct: -0.15,
                sentiment: -1.0,
                volatility_multiplier: 5.0,
                duration_rounds: 200,
            },
            MarketScenario::FlashCrash { drop_pct } => ScenarioEffect {
                name: format!("Flash crash -{:.1}%", drop_pct),
                price_shock_pct: -drop_pct / 100.0,
                sentiment: -1.0,
                volatility_multiplier: 8.0,
                duration_rounds: 15,
            },
            MarketScenario::ShortSqueeze { symbol, move_pct } => ScenarioEffect {
                name: format!("{} short squeeze +{:.1}%", symbol, move_pct),
                price_shock_pct: move_pct / 100.0,
                sentiment: 0.9,
                volatility_multiplier: 4.0,
                duration_rounds: 30,
            },
            MarketScenario::LiquidityVacuum => ScenarioEffect {
                name: "Liquidity vacuum".to_string(),
                price_shock_pct: -0.05,
                sentiment: -0.7,
                volatility_multiplier: 10.0,
                duration_rounds: 10,
            },
            MarketScenario::GeopoliticalCrisis { severity } => ScenarioEffect {
                name: format!("Geopolitical crisis severity={:.1}", severity),
                price_shock_pct: -severity * 0.08,
                sentiment: -severity * 0.9,
                volatility_multiplier: 1.0 + severity * 3.0,
                duration_rounds: (severity * 200.0) as u32,
            },
            MarketScenario::RegulatoryCrackdown { severity } => ScenarioEffect {
                name: format!("Regulatory crackdown severity={:.1}", severity),
                price_shock_pct: -severity * 0.12,
                sentiment: -severity * 0.7,
                volatility_multiplier: 2.0,
                duration_rounds: 100,
            },
            MarketScenario::MergerAnnouncement {
                acquirer,
                target,
                premium_pct,
            } => ScenarioEffect {
                name: format!(
                    "{} acquires {} at {:.1}% premium",
                    acquirer, target, premium_pct
                ),
                price_shock_pct: premium_pct / 100.0 * 0.9,
                sentiment: 0.5,
                volatility_multiplier: 1.5,
                duration_rounds: 30,
            },
            MarketScenario::DividendCut { symbol } => ScenarioEffect {
                name: format!("{} dividend cut", symbol),
                price_shock_pct: -0.08,
                sentiment: -0.6,
                volatility_multiplier: 1.8,
                duration_rounds: 60,
            },
            MarketScenario::Custom {
                name,
                price_shock_pct,
                sentiment,
                volatility_multiplier,
                duration_rounds,
            } => ScenarioEffect {
                name,
                price_shock_pct,
                sentiment,
                volatility_multiplier,
                duration_rounds,
            },
        }
    }
}

impl Default for ScenarioEngine {
    fn default() -> Self {
        Self::new()
    }
}
