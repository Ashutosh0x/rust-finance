// crates/ai/src/mirofish.rs
//
// MiroFish Swarm Simulator
// Runs N agent iterations (default 5,000) over a market snapshot,
// each agent sampling a random parameter set and producing a directional vote.
// Final signal is the probability-weighted consensus across all agents.

use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, info};

/// A market snapshot fed to each agent.
#[derive(Debug, Clone)]
pub struct MarketSnapshot {
    pub symbol: String,
    pub price: f64,
    pub bid: f64,
    pub ask: f64,
    /// Rolling prices, newest last.
    pub price_history: Vec<f64>,
    /// Rolling volumes, newest last.
    pub volume_history: Vec<f64>,
}

/// Vote cast by a single simulated agent.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentVote {
    Buy(f64),   // confidence 0.0–1.0
    Sell(f64),
    Hold,
}

/// Aggregated swarm result.
#[derive(Debug, Clone)]
pub struct SwarmSignal {
    pub symbol: String,
    pub buy_probability: f64,
    pub sell_probability: f64,
    pub hold_probability: f64,
    /// Weighted net directional score: positive = bullish, negative = bearish.
    pub net_score: f64,
    pub agents_run: usize,
    pub dominant_action: &'static str,
}

/// Configuration for a swarm run.
#[derive(Debug, Clone)]
pub struct SwarmConfig {
    /// Total number of agent iterations.
    pub n_agents: usize,
    /// Max concurrent Tokio tasks.
    pub concurrency: usize,
    /// Momentum lookback window (bars).
    pub momentum_window: usize,
    /// Mean-reversion z-score threshold.
    pub reversion_z_threshold: f64,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            n_agents: 5_000,
            concurrency: 200,
            momentum_window: 20,
            reversion_z_threshold: 2.0,
        }
    }
}

/// Run the full MiroFish swarm simulation.
pub async fn run_swarm(snapshot: MarketSnapshot, cfg: SwarmConfig) -> SwarmSignal {
    info!(
        symbol = %snapshot.symbol,
        n_agents = cfg.n_agents,
        "MiroFish swarm starting"
    );

    let snapshot = Arc::new(snapshot);
    let semaphore = Arc::new(Semaphore::new(cfg.concurrency));
    let mut join_set = JoinSet::new();

    for agent_id in 0..cfg.n_agents {
        let snap = snapshot.clone();
        let sem = semaphore.clone();
        let cfg_clone = cfg.clone();

        join_set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            simulate_agent(agent_id, &snap, &cfg_clone)
        });
    }

    let mut buy_weight = 0.0_f64;
    let mut sell_weight = 0.0_f64;
    let mut hold_count = 0usize;
    let mut total = 0usize;

    while let Some(result) = join_set.join_next().await {
        if let Ok(vote) = result {
            total += 1;
            match vote {
                AgentVote::Buy(conf) => buy_weight += conf,
                AgentVote::Sell(conf) => sell_weight += conf,
                AgentVote::Hold => hold_count += 1,
            }
        }
    }

    let total_f = total as f64;
    let hold_weight = hold_count as f64;
    let grand_total = buy_weight + sell_weight + hold_weight;

    let buy_prob = buy_weight / grand_total;
    let sell_prob = sell_weight / grand_total;
    let hold_prob = hold_weight / grand_total;
    let net_score = buy_prob - sell_prob;

    let dominant_action = if buy_prob > sell_prob && buy_prob > hold_prob {
        "BUY"
    } else if sell_prob > buy_prob && sell_prob > hold_prob {
        "SELL"
    } else {
        "HOLD"
    };

    let signal = SwarmSignal {
        symbol: snapshot.symbol.clone(),
        buy_probability: buy_prob,
        sell_probability: sell_prob,
        hold_probability: hold_prob,
        net_score,
        agents_run: total,
        dominant_action,
    };

    info!(
        symbol = %signal.symbol,
        action = signal.dominant_action,
        buy_prob = format!("{:.3}", signal.buy_probability),
        sell_prob = format!("{:.3}", signal.sell_probability),
        net_score = format!("{:.4}", signal.net_score),
        agents = signal.agents_run,
        "MiroFish swarm complete"
    );

    signal
}

/// Single agent simulation — deterministic given its agent_id seed.
///
/// Each agent runs a random mix of:
/// - Momentum (trend-following)
/// - Mean-reversion
/// - Spread-based liquidity signals
fn simulate_agent(agent_id: usize, snap: &MarketSnapshot, cfg: &SwarmConfig) -> AgentVote {
    // Lightweight deterministic RNG seeded from agent_id
    let mut rng = XorShift64(agent_id as u64 ^ 0xcafe_babe_dead_beef);

    // Each agent randomly weights its three strategy components
    let w_momentum: f64 = rng.next_f64();
    let w_reversion: f64 = rng.next_f64();
    let w_spread: f64 = rng.next_f64();
    let w_sum = w_momentum + w_reversion + w_spread + f64::EPSILON;

    let mut score = 0.0_f64;

    // ── Momentum signal ─────────────────────────────────────────────────────
    if snap.price_history.len() >= cfg.momentum_window {
        let window = &snap.price_history[snap.price_history.len() - cfg.momentum_window..];
        let oldest = window[0];
        let newest = *window.last().unwrap();
        let momentum = (newest - oldest) / (oldest + f64::EPSILON);
        score += (w_momentum / w_sum) * momentum.signum() * momentum.abs().min(1.0);
    }

    // ── Mean-reversion signal ────────────────────────────────────────────────
    if snap.price_history.len() >= 2 {
        let mean: f64 = snap.price_history.iter().sum::<f64>() / snap.price_history.len() as f64;
        let variance: f64 = snap
            .price_history
            .iter()
            .map(|p| (p - mean).powi(2))
            .sum::<f64>()
            / snap.price_history.len() as f64;
        let std_dev = variance.sqrt() + f64::EPSILON;
        let z = (snap.price - mean) / std_dev;

        if z.abs() > cfg.reversion_z_threshold {
            // Revert: if price is above mean by z, expect it to fall → sell signal
            score -= (w_reversion / w_sum) * z.signum() * (z.abs() - cfg.reversion_z_threshold);
        }
    }

    // ── Spread signal ────────────────────────────────────────────────────────
    let spread_pct = (snap.ask - snap.bid) / (snap.price + f64::EPSILON);
    // Tight spread → more confident in direction, wide spread → hold
    let spread_signal = if spread_pct < 0.001 { 1.0 } else { -spread_pct * 10.0 };
    score += (w_spread / w_sum) * spread_signal;

    debug!(agent_id, score = format!("{:.4}", score), "Agent vote");

    // Threshold-based vote
    if score > 0.05 {
        AgentVote::Buy(score.abs().min(1.0))
    } else if score < -0.05 {
        AgentVote::Sell(score.abs().min(1.0))
    } else {
        AgentVote::Hold
    }
}

// ── Minimal XorShift RNG (no external deps) ──────────────────────────────────
struct XorShift64(u64);

impl XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(trend: f64) -> MarketSnapshot {
        let history: Vec<f64> = (0..50)
            .map(|i| 100.0 + i as f64 * trend)
            .collect();
        let price = *history.last().unwrap();
        MarketSnapshot {
            symbol: "TEST".to_string(),
            price,
            bid: price - 0.05,
            ask: price + 0.05,
            price_history: history.clone(),
            volume_history: vec![1_000.0; 50],
        }
    }

    #[tokio::test]
    async fn test_bullish_trend_produces_buy_signal() {
        let snap = make_snapshot(0.5); // strong uptrend
        let signal = run_swarm(snap, SwarmConfig { n_agents: 100, ..Default::default() }).await;
        assert_eq!(signal.dominant_action, "BUY");
        assert!(signal.net_score > 0.0);
    }

    #[tokio::test]
    async fn test_bearish_trend_produces_sell_signal() {
        let snap = make_snapshot(-0.5); // strong downtrend
        let signal = run_swarm(snap, SwarmConfig { n_agents: 100, ..Default::default() }).await;
        assert_eq!(signal.dominant_action, "SELL");
        assert!(signal.net_score < 0.0);
    }

    #[tokio::test]
    async fn test_agents_run_count_matches_config() {
        let snap = make_snapshot(0.0);
        let cfg = SwarmConfig { n_agents: 500, ..Default::default() };
        let signal = run_swarm(snap, cfg).await;
        assert_eq!(signal.agents_run, 500);
    }
}
