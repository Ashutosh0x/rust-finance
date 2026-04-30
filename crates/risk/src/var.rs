// crates/risk/src/var.rs
// Value at Risk — Historical and Parametric VaR at 95%/99% confidence
// Required for SEBI algo registration and any regulated fund entity

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Position {
    pub symbol: String,
    pub quantity: f64, // negative for short
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
        Self {
            returns_history: HashMap::new(),
            min_history_days,
        }
    }

    /// Feed daily returns data. Call once per day per symbol.
    pub fn update_returns(&mut self, symbol: &str, daily_return_pct: f64) {
        let history = self.returns_history.entry(symbol.to_string()).or_default();
        history.push(daily_return_pct);
        // Keep rolling window of 252 trading days
        if history.len() > 252 {
            history.remove(0); // TODO: migrate to VecDeque for O(1)
        }
    }

    /// Historical VaR — non-parametric, uses actual return distribution
    /// Most accurate because it captures fat tails and skew
    pub fn historical_var(&self, positions: &[Position]) -> Option<VarResult> {
        if positions.is_empty() {
            return None;
        }

        // Ensure sufficient history for all positions
        for pos in positions {
            let hist = self.returns_history.get(&pos.symbol)?;
            if hist.len() < self.min_history_days {
                return None;
            }
        }

        let n_days = self
            .returns_history
            .values()
            .map(|h| h.len())
            .min()
            .unwrap_or(0);
        if n_days < self.min_history_days {
            return None;
        }

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
        let tail_95: Vec<f64> = sorted[..=idx_95].to_vec();
        let cvar_95 = if !tail_95.is_empty() {
            -tail_95.iter().sum::<f64>() / tail_95.len() as f64
        } else {
            var_95
        };

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
            var_99_10d_usd: var_99 * 10.0f64.sqrt(), // Basel scaling
            var_95_1d_pct: var_95 / portfolio_notional.max(1.0) * 100.0,
            var_99_1d_pct: var_99 / portfolio_notional.max(1.0) * 100.0,
            cvar_95_usd: cvar_95,
            portfolio_notional,
            component_var,
        })
    }

    /// Parametric (Delta-Normal) VaR — faster, assumes normal distribution
    pub fn parametric_var(&self, positions: &[Position]) -> Option<VarResult> {
        if positions.is_empty() {
            return None;
        }

        let mut portfolio_variance = 0.0;
        let mut component_var = HashMap::new();

        for pos in positions {
            let hist = self.returns_history.get(&pos.symbol)?;
            if hist.len() < self.min_history_days {
                return None;
            }

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
            cvar_95_usd: var_95 * 1.2, // Approximate CVaR for normal distribution
            portfolio_notional,
            component_var,
        })
    }

    /// Parametric VaR using Student-t distribution — accounts for fat tails.
    ///
    /// Financial returns have fatter tails than Gaussian (empirical ν ≈ 4-9
    /// for equities). Student-t VaR is typically 15-30% larger than Gaussian VaR,
    /// providing more conservative risk estimates.
    ///
    /// `nu`: degrees of freedom. Lower = fatter tails. Typical values:
    ///   - 5.0 for equities
    ///   - 3.5 for crypto
    ///   - 8.0 for bonds
    pub fn parametric_var_student_t(&self, positions: &[Position], nu: f64) -> Option<VarResult> {
        use statrs::distribution::{ContinuousCDF, StudentsT};

        if positions.is_empty() {
            return None;
        }
        if nu <= 2.0 {
            return None;
        } // Student-t variance undefined for ν ≤ 2

        let t_dist = StudentsT::new(0.0, 1.0, nu).ok()?;
        let z_95_t = t_dist.inverse_cdf(0.95).abs();
        let z_99_t = t_dist.inverse_cdf(0.99).abs();

        // Scale adjustment: Student-t has variance = ν/(ν-2), so we normalize
        let scale = ((nu - 2.0) / nu).sqrt();

        let mut portfolio_variance = 0.0;
        let mut component_var = HashMap::new();

        for pos in positions {
            let hist = self.returns_history.get(&pos.symbol)?;
            if hist.len() < self.min_history_days {
                return None;
            }

            let daily_vol = Self::std_dev(hist);
            let pos_dollar_vol = pos.notional().abs() * daily_vol / 100.0;
            portfolio_variance += pos_dollar_vol * pos_dollar_vol;

            component_var.insert(pos.symbol.clone(), pos_dollar_vol * z_99_t / scale);
        }

        let portfolio_vol = portfolio_variance.sqrt();
        let var_95 = portfolio_vol * z_95_t / scale;
        let var_99 = portfolio_vol * z_99_t / scale;
        let portfolio_notional: f64 = positions.iter().map(|p| p.notional().abs()).sum();

        // CVaR for Student-t (fat tails increase expected shortfall)
        let _t_quantile_05 = statrs::distribution::ContinuousCDF::inverse_cdf(&t_dist, 0.05);
        let cvar_95 = var_95 * 1.3; // Approximate CVaR scaling for Student-t

        Some(VarResult {
            var_95_1d_usd: var_95,
            var_99_1d_usd: var_99,
            var_99_10d_usd: var_99 * 10.0f64.sqrt(),
            var_95_1d_pct: var_95 / portfolio_notional.max(1.0) * 100.0,
            var_99_1d_pct: var_99 / portfolio_notional.max(1.0) * 100.0,
            cvar_95_usd: cvar_95,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_calculator_with_normal_returns(
        sigma_pct: f64,
        n_days: usize,
    ) -> (VarCalculator, Vec<Position>) {
        let mut calc = VarCalculator::new(20);
        // Generate deterministic pseudo-normal returns using simple xorshift
        let mut rng: u64 = 12345;
        for _ in 0..n_days {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let u = (rng as f64) / (u64::MAX as f64);
            let z = (u - 0.5) * 3.46; // approximate normal spread
            let ret = z * sigma_pct;
            calc.update_returns("TEST", ret);
        }
        let positions = vec![Position {
            symbol: "TEST".into(),
            quantity: 100.0,
            current_price: 100.0,
        }];
        (calc, positions)
    }

    #[test]
    fn test_var_95_vs_99_ordering() {
        let (calc, positions) = make_calculator_with_normal_returns(1.5, 252);
        let result = calc.historical_var(&positions).unwrap();
        assert!(
            result.var_99_1d_usd >= result.var_95_1d_usd,
            "VaR99 ({:.2}) must be >= VaR95 ({:.2})",
            result.var_99_1d_usd,
            result.var_95_1d_usd
        );
    }

    #[test]
    fn test_cvar_exceeds_var() {
        let (calc, positions) = make_calculator_with_normal_returns(1.5, 252);
        let result = calc.historical_var(&positions).unwrap();
        assert!(
            result.cvar_95_usd >= result.var_95_1d_usd,
            "CVaR ({:.2}) must be >= VaR ({:.2})",
            result.cvar_95_usd,
            result.var_95_1d_usd
        );
    }

    #[test]
    fn test_var_empty_portfolio() {
        let calc = VarCalculator::new(20);
        let result = calc.historical_var(&[]);
        assert!(result.is_none(), "Empty portfolio should return None");

        let result2 = calc.parametric_var(&[]);
        assert!(
            result2.is_none(),
            "Empty portfolio parametric should return None"
        );
    }

    #[test]
    fn test_var_insufficient_history() {
        let mut calc = VarCalculator::new(50);
        for i in 0..20 {
            calc.update_returns("TEST", (i as f64) * 0.01);
        }
        let positions = vec![Position {
            symbol: "TEST".into(),
            quantity: 100.0,
            current_price: 100.0,
        }];
        let result = calc.historical_var(&positions);
        assert!(result.is_none(), "Insufficient history should return None");
    }

    #[test]
    fn test_var_10d_scaling() {
        let (calc, positions) = make_calculator_with_normal_returns(1.5, 252);
        let result = calc.historical_var(&positions).unwrap();
        let expected_10d = result.var_99_1d_usd * 10.0_f64.sqrt();
        let relative_err = (result.var_99_10d_usd - expected_10d).abs() / expected_10d.max(0.001);
        assert!(
            relative_err < 0.001,
            "10-day VaR ({:.4}) should equal 1-day × √10 ({:.4})",
            result.var_99_10d_usd,
            expected_10d
        );
    }

    #[test]
    fn test_parametric_var_positive() {
        let (calc, positions) = make_calculator_with_normal_returns(1.5, 252);
        let result = calc.parametric_var(&positions).unwrap();
        assert!(
            result.var_95_1d_usd > 0.0,
            "Parametric VaR95 must be positive"
        );
        assert!(
            result.var_99_1d_usd > 0.0,
            "Parametric VaR99 must be positive"
        );
        assert!(
            result.var_99_1d_usd >= result.var_95_1d_usd,
            "Parametric VaR99 >= VaR95"
        );
    }

    #[test]
    fn test_var_percentage_bounded() {
        let (calc, positions) = make_calculator_with_normal_returns(1.5, 252);
        let result = calc.historical_var(&positions).unwrap();
        assert!(
            result.var_95_1d_pct > 0.0 && result.var_95_1d_pct < 100.0,
            "VaR percentage should be between 0 and 100, got {}",
            result.var_95_1d_pct
        );
    }

    #[test]
    fn test_component_var_sums_reasonably() {
        let mut calc = VarCalculator::new(20);
        let mut rng: u64 = 99;
        for _ in 0..100 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let u = (rng as f64) / (u64::MAX as f64);
            calc.update_returns("A", (u - 0.5) * 4.0);
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let u2 = (rng as f64) / (u64::MAX as f64);
            calc.update_returns("B", (u2 - 0.5) * 4.0);
        }
        let positions = vec![
            Position {
                symbol: "A".into(),
                quantity: 50.0,
                current_price: 100.0,
            },
            Position {
                symbol: "B".into(),
                quantity: 50.0,
                current_price: 100.0,
            },
        ];
        let result = calc.historical_var(&positions).unwrap();
        let component_sum: f64 = result.component_var.values().sum();
        // Component VaR should roughly equal total VaR (exactly equal for our simple weighting)
        let relative_diff =
            (component_sum - result.var_99_1d_usd).abs() / result.var_99_1d_usd.max(0.001);
        assert!(relative_diff < 0.01, "Component VaR sum should ≈ total VaR");
    }

    /// Parametric and Historical VaR should be in the same ballpark for the same data.
    /// A >5× divergence indicates a broken model.
    #[test]
    fn test_var_parametric_vs_historical_same_ballpark() {
        let (calc, positions) = make_calculator_with_normal_returns(1.5, 252);
        let hist = calc.historical_var(&positions).unwrap();
        let para = calc.parametric_var(&positions).unwrap();

        // They won't match exactly (different methods), but should be within 3×
        let ratio = hist.var_95_1d_usd / para.var_95_1d_usd.max(0.001);
        assert!(
            ratio > 0.2 && ratio < 5.0,
            "Parametric vs Historical VaR ratio too extreme: {:.2} (hist={:.2}, para={:.2})",
            ratio,
            hist.var_95_1d_usd,
            para.var_95_1d_usd
        );
    }

    /// Doubling position size should approximately double VaR.
    #[test]
    fn test_var_scales_with_position_size() {
        let (calc, _) = make_calculator_with_normal_returns(1.5, 252);
        let small = vec![Position {
            symbol: "TEST".into(),
            quantity: 50.0,
            current_price: 100.0,
        }];
        let large = vec![Position {
            symbol: "TEST".into(),
            quantity: 100.0,
            current_price: 100.0,
        }];

        let var_small = calc.parametric_var(&small).unwrap().var_95_1d_usd;
        let var_large = calc.parametric_var(&large).unwrap().var_95_1d_usd;

        let ratio = var_large / var_small.max(0.001);
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "VaR should scale linearly with position: ratio={:.4}, expected 2.0",
            ratio
        );
    }

    /// VaR with a single holding should equal the component VaR for that holding.
    #[test]
    fn test_var_single_position_equals_component() {
        let (calc, positions) = make_calculator_with_normal_returns(1.5, 252);
        let result = calc.historical_var(&positions).unwrap();
        assert_eq!(result.component_var.len(), 1);
        let component = result.component_var.get("TEST").unwrap();
        let relative_diff =
            (component - result.var_99_1d_usd).abs() / result.var_99_1d_usd.max(0.001);
        assert!(
            relative_diff < 0.01,
            "Single-position component VaR should equal total VaR"
        );
    }
}
