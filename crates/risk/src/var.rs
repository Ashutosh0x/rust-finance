// crates/risk/src/var.rs
// Value at Risk — Historical and Parametric VaR at 95%/99% confidence
// Required for SEBI algo registration and any regulated fund entity

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Position {
    pub symbol: String,
    pub quantity: f64,       // negative for short
    pub current_price: f64,
}

impl Position {
    pub fn notional(&self) -> f64 {
        self.quantity * self.current_price
    }
}

#[derive(Debug, Clone)]
pub struct VarResult {
    /// 1-day VaR at 95% confidence in USD
    pub var_95_1d_usd: f64,
    /// 1-day VaR at 99% confidence in USD
    pub var_99_1d_usd: f64,
    /// 10-day VaR at 99% (Basel requirement: √10 × 1-day)
    pub var_99_10d_usd: f64,
    /// VaR as % of total portfolio value
    pub var_95_1d_pct: f64,
    pub var_99_1d_pct: f64,
    /// Expected Shortfall (CVaR) — average loss beyond VaR threshold
    pub cvar_95_usd: f64,
    /// Total portfolio notional value
    pub portfolio_notional: f64,
    /// Per-symbol contribution to VaR
    pub component_var: HashMap<String, f64>,
}

pub struct VarCalculator {
    /// Historical daily returns per symbol: symbol → vec of daily % returns
    returns_history: HashMap<String, Vec<f64>>,
    /// Minimum days of history needed
    min_history_days: usize,
}

impl VarCalculator {
    pub fn new(min_history_days: usize) -> Self {
        Self { returns_history: HashMap::new(), min_history_days }
    }

    /// Feed daily returns data. Call once per day per symbol.
    pub fn update_returns(&mut self, symbol: &str, daily_return_pct: f64) {
        let history = self.returns_history.entry(symbol.to_string()).or_default();
        history.push(daily_return_pct);
        // Keep rolling window of 252 trading days
        if history.len() > 252 {
            history.remove(0);
        }
    }

    /// Historical VaR — non-parametric, uses actual return distribution
    /// Most accurate because it captures fat tails and skew
    pub fn historical_var(&self, positions: &[Position]) -> Option<VarResult> {
        if positions.is_empty() { return None; }

        // Ensure sufficient history for all positions
        for pos in positions {
            let hist = self.returns_history.get(&pos.symbol)?;
            if hist.len() < self.min_history_days { return None; }
        }

        let n_days = self.returns_history.values().map(|h| h.len()).min().unwrap_or(0);
        if n_days < self.min_history_days { return None; }

        // Compute portfolio P&L for each historical day
        let mut pnl_series: Vec<f64> = Vec::with_capacity(n_days);
        for day_idx in 0..n_days {
            let mut day_pnl = 0.0;
            for pos in positions {
                if let Some(hist) = self.returns_history.get(&pos.symbol) {
                    if let Some(ret) = hist.get(day_idx) {
                        day_pnl += pos.notional() * ret / 100.0;
                    }
                }
            }
            pnl_series.push(day_pnl);
        }

        // Sort losses (negative P&L = loss)
        let mut sorted = pnl_series.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let n = sorted.len() as f64;
        let idx_95 = ((1.0 - 0.95) * n) as usize;
        let idx_99 = ((1.0 - 0.99) * n) as usize;

        let var_95 = -sorted.get(idx_95).copied().unwrap_or(0.0).min(0.0);
        let var_99 = -sorted.get(idx_99).copied().unwrap_or(0.0).min(0.0);

        // CVaR: average of losses beyond VaR threshold
        let tail_95: Vec<f64> = sorted[..=idx_95].iter().copied().collect();
        let cvar_95 = if !tail_95.is_empty() {
            -tail_95.iter().sum::<f64>() / tail_95.len() as f64
        } else { var_95 };

        let portfolio_notional: f64 = positions.iter().map(|p| p.notional().abs()).sum();

        // Component VaR: each position's marginal contribution
        let mut component_var = HashMap::new();
        for pos in positions {
            let pos_weight = pos.notional().abs() / portfolio_notional.max(1.0);
            component_var.insert(pos.symbol.clone(), var_99 * pos_weight);
        }

        Some(VarResult {
            var_95_1d_usd: var_95,
            var_99_1d_usd: var_99,
            var_99_10d_usd: var_99 * 10.0f64.sqrt(),  // Basel scaling
            var_95_1d_pct: var_95 / portfolio_notional.max(1.0) * 100.0,
            var_99_1d_pct: var_99 / portfolio_notional.max(1.0) * 100.0,
            cvar_95_usd: cvar_95,
            portfolio_notional,
            component_var,
        })
    }

    /// Parametric (Delta-Normal) VaR — faster, assumes normal distribution
    pub fn parametric_var(&self, positions: &[Position]) -> Option<VarResult> {
        if positions.is_empty() { return None; }

        let mut portfolio_variance = 0.0;
        let mut component_var = HashMap::new();

        for pos in positions {
            let hist = self.returns_history.get(&pos.symbol)?;
            if hist.len() < self.min_history_days { return None; }

            // Position standard deviation
            let daily_vol = Self::std_dev(hist);
            let pos_dollar_vol = pos.notional().abs() * daily_vol / 100.0;

            // Simplified: assume zero correlation between positions (conservative)
            portfolio_variance += pos_dollar_vol * pos_dollar_vol;

            // Z-score 1.645 for 95%, 2.326 for 99%
            component_var.insert(pos.symbol.clone(), pos_dollar_vol * 2.326);
        }

        let portfolio_vol = portfolio_variance.sqrt();
        let var_95 = portfolio_vol * 1.645;
        let var_99 = portfolio_vol * 2.326;
        let portfolio_notional: f64 = positions.iter().map(|p| p.notional().abs()).sum();

        Some(VarResult {
            var_95_1d_usd: var_95,
            var_99_1d_usd: var_99,
            var_99_10d_usd: var_99 * 10.0f64.sqrt(),
            var_95_1d_pct: var_95 / portfolio_notional.max(1.0) * 100.0,
            var_99_1d_pct: var_99 / portfolio_notional.max(1.0) * 100.0,
            cvar_95_usd: var_95 * 1.2,  // Approximate CVaR for normal distribution
            portfolio_notional,
            component_var,
        })
    }

    fn std_dev(returns: &[f64]) -> f64 {
        let n = returns.len() as f64;
        let mean = returns.iter().sum::<f64>() / n;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
        variance.sqrt()
    }
}
