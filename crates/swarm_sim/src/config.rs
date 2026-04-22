use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmConfig {
    pub agent_count: usize,
    pub retail_fraction: f64,
    pub hedge_fund_fraction: f64,
    pub market_maker_fraction: f64,
    pub arbitrage_fraction: f64,
    pub momentum_fraction: f64,
    pub news_trader_fraction: f64,
    pub contrarian_fraction: f64,
    pub rounds_per_day: u32,
    pub round_delay_ms: u64,
    pub price_impact_lambda: f64,
    pub base_spread_frac: f64,
    pub annualized_vol: f64,
    pub activation_prob: f64,
    pub peak_hour_multiplier: f64,
    pub max_position_usd: f64,
    pub max_parallel_ai: usize,
    pub db_path: String,
    pub db_batch_size: usize,
    pub signal_emit_interval: u32,

    // ── v2.1 — Production calibration fields ──
    /// Cash per agent in USD. Total pool = cash_per_agent * agent_count.
    pub cash_per_agent_usd: f64,
    /// Mean reversion speed (percentage-based). Higher = stronger anchor to initial price.
    pub mean_reversion_speed: f64,
    /// Hard cap on cumulative drift (%). Simulation clamps price beyond this.
    pub max_drift_pct: f64,
    /// Spread in basis points for liquid assets (SPY, QQQ, AAPL, MSFT).
    pub spread_bps_liquid: f64,
    /// Spread in basis points for illiquid assets (FXI, EEM, XLE).
    pub spread_bps_illiquid: f64,
    /// Max flow any single agent can deploy per round (USD).
    pub max_deploy_per_round: f64,
    /// Inventory decay rate per round. Position *= (1 - decay) each round.
    pub inventory_decay_rate: f64,
    /// Deterministic seed for agent initialization. Different seeds produce
    /// different agent behavior while remaining reproducible.
    /// 0 = legacy behavior (backward compatible with existing tests).
    pub seed: u64,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            agent_count: 5_000,
            retail_fraction: 0.48,
            hedge_fund_fraction: 0.10,
            market_maker_fraction: 0.15,
            arbitrage_fraction: 0.08,
            momentum_fraction: 0.05,
            news_trader_fraction: 0.02,
            contrarian_fraction: 0.12,
            rounds_per_day: 390,
            round_delay_ms: 100,
            price_impact_lambda: 0.00001, // multiplicative — % impact per $1M flow
            base_spread_frac: 0.0005,
            annualized_vol: 0.20,
            activation_prob: 0.30,
            peak_hour_multiplier: 2.5,
            max_position_usd: 50_000.0,
            max_parallel_ai: 30,
            db_path: "swarm_simulation.db".to_string(),
            db_batch_size: 500,
            signal_emit_interval: 5,

            // v2.1 defaults — calibrated for ±2-10% drift over 200 rounds
            cash_per_agent_usd: 10_000.0, // $50M total pool (vs old $500M)
            mean_reversion_speed: 0.05,   // 5% reversion (vs old 2%)
            max_drift_pct: 20.0,          // hard cap ±20%
            spread_bps_liquid: 2.0,       // 0.02% for SPY, QQQ
            spread_bps_illiquid: 8.0,     // 0.08% for FXI, EEM
            max_deploy_per_round: 500.0,  // 5% of $10K cash/round
            inventory_decay_rate: 0.005,  // 0.5%/round → full shed ~140 rounds
            seed: 0,                      // 0 = legacy compatible
        }
    }
}

impl SwarmConfig {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str::<Self>(&content)?)
    }

    pub fn daily_vol(&self) -> f64 {
        self.annualized_vol / (252_f64).sqrt()
    }

    pub fn round_vol(&self) -> f64 {
        self.daily_vol() / (self.rounds_per_day as f64).sqrt()
    }

    /// Get spread in basis points for a given symbol based on asset class.
    pub fn spread_bps_for(&self, symbol: &str) -> f64 {
        match symbol {
            "SPY" | "QQQ" | "AAPL" | "MSFT" | "AMZN" | "NVDA" => self.spread_bps_liquid,
            _ => self.spread_bps_illiquid,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let sum = self.retail_fraction
            + self.hedge_fund_fraction
            + self.market_maker_fraction
            + self.arbitrage_fraction
            + self.momentum_fraction
            + self.news_trader_fraction
            + self.contrarian_fraction;

        if (sum - 1.0).abs() > 0.001 {
            return Err(format!(
                "Trader type fractions sum to {:.4}, must equal 1.0",
                sum
            ));
        }
        if self.cash_per_agent_usd <= 0.0 {
            return Err("cash_per_agent_usd must be positive".to_string());
        }
        if self.max_drift_pct <= 0.0 {
            return Err("max_drift_pct must be positive".to_string());
        }
        Ok(())
    }
}
