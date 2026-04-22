// crates/ml/src/alpha_monitor.rs
//
// Alpha Decay Monitor — tracks strategy health via rolling Information
// Coefficient (IC), Sharpe ratio, and hit rate.
//
// When a strategy's signal-to-return correlation degrades below threshold,
// the monitor flags AlphaHealth::Decayed, signaling the daemon to pause
// the strategy until recalibration.
//
// Key metrics:
//   IC (Information Coefficient) = Spearman rank correlation(signal, return)
//   Rolling Sharpe = mean(pnl) / std(pnl) × √252
//   Hit Rate = fraction of trades with positive P&L
//
// Reference: Grinold & Kahn (2000) "Active Portfolio Management"

use std::collections::VecDeque;

/// Health status of a strategy's alpha signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaHealth {
    /// IC > ic_threshold, Sharpe > sharpe_threshold.
    /// Strategy is performing as expected.
    Healthy,
    /// IC or Sharpe below threshold but not critically.
    /// Warning state — monitor closely.
    Degraded,
    /// IC and Sharpe both below critical thresholds.
    /// Strategy should be paused.
    Decayed,
    /// Not enough data to evaluate (< min_observations).
    Insufficient,
}

impl AlphaHealth {
    /// Whether the strategy should continue trading.
    pub fn should_trade(&self) -> bool {
        matches!(self, AlphaHealth::Healthy | AlphaHealth::Insufficient)
    }

    pub fn label(&self) -> &'static str {
        match self {
            AlphaHealth::Healthy => "HEALTHY",
            AlphaHealth::Degraded => "DEGRADED",
            AlphaHealth::Decayed => "DECAYED",
            AlphaHealth::Insufficient => "INSUFFICIENT",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            AlphaHealth::Healthy => "🟢",
            AlphaHealth::Degraded => "🟡",
            AlphaHealth::Decayed => "🔴",
            AlphaHealth::Insufficient => "⚪",
        }
    }
}

/// Configuration for the alpha decay monitor.
#[derive(Debug, Clone)]
pub struct AlphaMonitorConfig {
    /// Rolling window size (number of trades).
    pub window: usize,
    /// Minimum observations before evaluation.
    pub min_observations: usize,
    /// IC threshold below which alpha is "degraded".
    pub ic_degraded_threshold: f64,
    /// IC threshold below which alpha is "decayed".
    pub ic_decayed_threshold: f64,
    /// Sharpe threshold below which alpha is "degraded".
    pub sharpe_degraded_threshold: f64,
    /// Sharpe threshold below which alpha is "decayed".
    pub sharpe_decayed_threshold: f64,
}

impl Default for AlphaMonitorConfig {
    fn default() -> Self {
        Self {
            window: 500,
            min_observations: 50,
            ic_degraded_threshold: 0.03,
            ic_decayed_threshold: 0.01,
            sharpe_degraded_threshold: 0.5,
            sharpe_decayed_threshold: 0.0,
        }
    }
}

/// Alpha Decay Monitor.
///
/// Feed it (signal_direction, actual_return) pairs after each trade.
/// Query `health()` to determine whether the strategy should continue.
pub struct AlphaMonitor {
    config: AlphaMonitorConfig,
    /// Rolling window of (signal_value, actual_return) observations.
    history: VecDeque<(f64, f64)>,
    /// Strategy name for logging.
    strategy_name: String,
}

impl AlphaMonitor {
    pub fn new(strategy_name: impl Into<String>, config: AlphaMonitorConfig) -> Self {
        let window = config.window;
        Self {
            config,
            history: VecDeque::with_capacity(window + 1),
            strategy_name: strategy_name.into(),
        }
    }

    /// Record a new observation: what the strategy predicted vs actual outcome.
    ///
    /// `signal`: the strategy's directional signal (positive = bullish, negative = bearish)
    /// `actual_return`: the realized return for that trade
    pub fn record(&mut self, signal: f64, actual_return: f64) {
        self.history.push_back((signal, actual_return));
        if self.history.len() > self.config.window {
            self.history.pop_front();
        }
    }

    /// Compute the rolling Information Coefficient (Spearman rank correlation).
    ///
    /// IC = 1 - (6 × Σd² / (n × (n² - 1)))
    /// where d = difference between signal rank and return rank.
    pub fn rolling_ic(&self) -> f64 {
        let n = self.history.len();
        if n < 3 {
            return 0.0;
        }

        // Rank signals
        let mut signal_indexed: Vec<(usize, f64)> =
            self.history.iter().enumerate().map(|(i, (s, _))| (i, *s)).collect();
        signal_indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut signal_ranks = vec![0.0; n];
        for (rank, (idx, _)) in signal_indexed.iter().enumerate() {
            signal_ranks[*idx] = rank as f64 + 1.0;
        }

        // Rank returns
        let mut return_indexed: Vec<(usize, f64)> =
            self.history.iter().enumerate().map(|(i, (_, r))| (i, *r)).collect();
        return_indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut return_ranks = vec![0.0; n];
        for (rank, (idx, _)) in return_indexed.iter().enumerate() {
            return_ranks[*idx] = rank as f64 + 1.0;
        }

        // Spearman: 1 - 6Σd²/(n(n²-1))
        let sum_d_sq: f64 = signal_ranks
            .iter()
            .zip(return_ranks.iter())
            .map(|(sr, rr)| (sr - rr).powi(2))
            .sum();

        let nf = n as f64;
        1.0 - (6.0 * sum_d_sq) / (nf * (nf * nf - 1.0))
    }

    /// Compute the rolling Sharpe ratio (annualized).
    ///
    /// Sharpe = mean(returns) / std(returns) × √252
    pub fn rolling_sharpe(&self) -> f64 {
        let n = self.history.len();
        if n < 3 {
            return 0.0;
        }

        let returns: Vec<f64> = self.history.iter().map(|(_, r)| *r).collect();
        let mean = returns.iter().sum::<f64>() / n as f64;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
        let std_dev = variance.sqrt();

        if std_dev < 1e-12 {
            return 0.0;
        }

        (mean / std_dev) * 252.0_f64.sqrt()
    }

    /// Compute the hit rate (fraction of profitable trades).
    pub fn hit_rate(&self) -> f64 {
        if self.history.is_empty() {
            return 0.0;
        }

        let wins = self.history.iter().filter(|(_, r)| *r > 0.0).count();
        wins as f64 / self.history.len() as f64
    }

    /// Evaluate the current health of the strategy's alpha.
    pub fn health(&self) -> AlphaHealth {
        if self.history.len() < self.config.min_observations {
            return AlphaHealth::Insufficient;
        }

        let ic = self.rolling_ic();
        let sharpe = self.rolling_sharpe();

        // Both below decayed threshold → Decayed
        if ic < self.config.ic_decayed_threshold && sharpe < self.config.sharpe_decayed_threshold {
            return AlphaHealth::Decayed;
        }

        // Either below degraded threshold → Degraded
        if ic < self.config.ic_degraded_threshold
            || sharpe < self.config.sharpe_degraded_threshold
        {
            return AlphaHealth::Degraded;
        }

        AlphaHealth::Healthy
    }

    /// Get the strategy name.
    pub fn strategy_name(&self) -> &str {
        &self.strategy_name
    }

    /// Number of observations in the window.
    pub fn observation_count(&self) -> usize {
        self.history.len()
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insufficient_data() {
        let monitor = AlphaMonitor::new("test", AlphaMonitorConfig::default());
        assert_eq!(monitor.health(), AlphaHealth::Insufficient);
        assert!(monitor.health().should_trade()); // trade with insufficient data
    }

    #[test]
    fn test_perfect_positive_ic() {
        let mut monitor = AlphaMonitor::new(
            "perfect",
            AlphaMonitorConfig {
                window: 100,
                min_observations: 5,
                ..Default::default()
            },
        );

        // Perfect correlation: signal predicts return exactly
        for i in 0..50 {
            let signal = i as f64;
            let ret = i as f64 * 0.01;
            monitor.record(signal, ret);
        }

        let ic = monitor.rolling_ic();
        assert!(
            ic > 0.99,
            "Perfectly correlated signals should have IC ≈ 1.0, got {}",
            ic
        );
        assert_eq!(monitor.health(), AlphaHealth::Healthy);
    }

    #[test]
    fn test_random_signals_low_ic() {
        let mut monitor = AlphaMonitor::new(
            "random",
            AlphaMonitorConfig {
                window: 200,
                min_observations: 50,
                ..Default::default()
            },
        );

        // Anti-correlated: signal increases, return decreases
        // This produces negative IC, which has low absolute value area
        // Use a shuffled pattern to ensure near-zero correlation
        let signals = [3.0, 7.0, 1.0, 9.0, 5.0, 2.0, 8.0, 4.0, 10.0, 6.0];
        let returns = [0.01, -0.01, 0.005, -0.005, 0.01, -0.01, 0.005, -0.005, 0.01, -0.01];
        for _ in 0..10 {
            for (s, r) in signals.iter().zip(returns.iter()) {
                monitor.record(*s, *r);
            }
        }

        let ic = monitor.rolling_ic();
        assert!(
            ic.abs() < 0.3,
            "Shuffled signals should have low IC, got {}",
            ic
        );
    }

    #[test]
    fn test_decayed_alpha() {
        let mut monitor = AlphaMonitor::new(
            "decay_test",
            AlphaMonitorConfig {
                window: 100,
                min_observations: 10,
                ic_decayed_threshold: 0.05,
                sharpe_decayed_threshold: 0.5,
                ..Default::default()
            },
        );

        // Feed negative returns with uncorrelated signals
        for i in 0..60 {
            let signal = i as f64;
            let ret = -0.002; // consistently losing
            monitor.record(signal, ret);
        }

        let health = monitor.health();
        assert!(
            matches!(health, AlphaHealth::Decayed | AlphaHealth::Degraded),
            "Losing strategy should be Decayed or Degraded, got {:?}",
            health
        );
        assert!(!health.should_trade() || matches!(health, AlphaHealth::Degraded));
    }

    #[test]
    fn test_hit_rate() {
        let mut monitor = AlphaMonitor::new(
            "hr_test",
            AlphaMonitorConfig {
                window: 100,
                min_observations: 5,
                ..Default::default()
            },
        );

        // 7 wins, 3 losses
        for i in 0..10 {
            let ret = if i < 7 { 0.01 } else { -0.01 };
            monitor.record(1.0, ret);
        }

        let hr = monitor.hit_rate();
        assert!(
            (hr - 0.7).abs() < 0.01,
            "Hit rate should be 0.70, got {}",
            hr
        );
    }

    #[test]
    fn test_window_evicts_old_data() {
        let mut monitor = AlphaMonitor::new(
            "window",
            AlphaMonitorConfig {
                window: 10,
                min_observations: 5,
                ..Default::default()
            },
        );

        // Fill beyond window
        for i in 0..20 {
            monitor.record(i as f64, 0.01);
        }

        assert_eq!(
            monitor.observation_count(),
            10,
            "Should evict old data beyond window"
        );
    }

    #[test]
    fn test_rolling_sharpe_positive() {
        let mut monitor = AlphaMonitor::new(
            "sharpe",
            AlphaMonitorConfig {
                window: 100,
                min_observations: 5,
                ..Default::default()
            },
        );

        // Consistently positive returns
        for i in 0..50 {
            monitor.record(1.0, 0.005 + (i as f64 * 0.0001)); // small positive trend
        }

        let sharpe = monitor.rolling_sharpe();
        assert!(
            sharpe > 0.0,
            "Positive returns should yield positive Sharpe, got {}",
            sharpe
        );
    }
}
