// GARCH(1,1) — Generalized Autoregressive Conditional Heteroskedasticity
// Industry standard volatility forecasting model

#[derive(Debug, Clone)]
pub struct GarchParams {
    pub omega: f64,
    pub alpha: f64,
    pub beta: f64,
}

impl GarchParams {
    pub fn is_stationary(&self) -> bool {
        self.alpha + self.beta < 1.0
    }

    pub fn long_run_variance(&self) -> f64 {
        if self.alpha + self.beta >= 1.0 {
            return f64::INFINITY;
        }
        self.omega / (1.0 - self.alpha - self.beta)
    }

    pub fn long_run_vol_annualized(&self) -> f64 {
        (self.long_run_variance() * 252.0).sqrt()
    }

    pub fn persistence(&self) -> f64 {
        self.alpha + self.beta
    }

    pub fn shock_half_life(&self) -> f64 {
        let p = self.persistence();
        if p <= 0.0 || p >= 1.0 {
            return f64::NAN;
        }
        -std::f64::consts::LN_2 / p.ln()
    }
}

#[derive(Debug, Clone)]
pub struct GarchState {
    pub params: GarchParams,
    pub conditional_variance: f64,
    pub last_return: f64,
    pub variance_history: Vec<f64>,
}

impl GarchState {
    pub fn new(params: GarchParams, initial_variance: f64) -> Self {
        Self {
            conditional_variance: initial_variance,
            last_return: 0.0,
            variance_history: vec![initial_variance],
            params,
        }
    }

    pub fn from_returns(params: GarchParams, returns: &[f64]) -> Self {
        let n = returns.len() as f64;
        let mean = returns.iter().sum::<f64>() / n;
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
        Self::new(params, var)
    }

    pub fn update(&mut self, return_t: f64) {
        let new_var = self.params.omega
            + self.params.alpha * return_t.powi(2)
            + self.params.beta * self.conditional_variance;
        self.conditional_variance = new_var.max(1e-10);
        self.last_return = return_t;
        self.variance_history.push(self.conditional_variance);
    }

    pub fn current_vol_annualized(&self) -> f64 {
        (self.conditional_variance * 252.0).sqrt()
    }

    pub fn forecast(&self, h: usize) -> f64 {
        let lr_var = self.params.long_run_variance();
        if lr_var.is_infinite() {
            return self.conditional_variance;
        }
        let persistence_h = self.params.persistence().powi(h as i32);
        lr_var + persistence_h * (self.conditional_variance - lr_var)
    }

    pub fn forecast_vol_annualized(&self, h: usize) -> f64 {
        (self.forecast(h) * 252.0).sqrt()
    }

    pub fn var_1day(&self, z_score: f64, position_value: f64) -> f64 {
        z_score * self.conditional_variance.sqrt() * position_value
    }
}

pub struct GarchEstimator;

impl GarchEstimator {
    pub fn fit(returns: &[f64]) -> Option<(GarchParams, f64)> {
        if returns.len() < 50 {
            return None;
        }

        let n = returns.len() as f64;
        let sample_var: f64 = {
            let mean = returns.iter().sum::<f64>() / n;
            returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0)
        };

        let mut best_ll = f64::NEG_INFINITY;
        let mut best_params = GarchParams {
            omega: sample_var * 0.05,
            alpha: 0.05,
            beta: 0.90,
        };

        let alpha_grid = [0.03, 0.05, 0.08, 0.10, 0.12, 0.15];
        let beta_grid = [0.80, 0.85, 0.87, 0.90, 0.92, 0.94];

        for &alpha in &alpha_grid {
            for &beta in &beta_grid {
                if alpha + beta >= 0.999 {
                    continue;
                }
                let omega = sample_var * (1.0 - alpha - beta);
                let params = GarchParams { omega, alpha, beta };
                let ll = Self::log_likelihood(returns, &params);
                if ll > best_ll {
                    best_ll = ll;
                    best_params = params;
                }
            }
        }

        let mut p = best_params;
        let eps = 1e-6;
        let lr = 1e-5;

        for _ in 0..200 {
            let ll = Self::log_likelihood(returns, &p);

            let mut pa = p.clone();
            pa.alpha += eps;
            let mut pb = p.clone();
            pb.beta += eps;
            let mut po = p.clone();
            po.omega += eps;

            let grad_a = (Self::log_likelihood(returns, &pa) - ll) / eps;
            let grad_b = (Self::log_likelihood(returns, &pb) - ll) / eps;
            let grad_o = (Self::log_likelihood(returns, &po) - ll) / eps;

            p.alpha += lr * grad_a;
            p.beta += lr * grad_b;
            p.omega += lr * grad_o;

            p.alpha = p.alpha.max(1e-6).min(0.5);
            p.beta = p.beta.max(1e-6);
            p.omega = p.omega.max(1e-12);
            if p.alpha + p.beta >= 0.999 {
                let s = 0.998 / (p.alpha + p.beta);
                p.alpha *= s;
                p.beta *= s;
            }
        }

        let final_ll = Self::log_likelihood(returns, &p);
        Some((p, final_ll))
    }

    pub fn log_likelihood(returns: &[f64], params: &GarchParams) -> f64 {
        let n = returns.len();
        let sample_var: f64 = returns.iter().map(|r| r * r).sum::<f64>() / n as f64;
        let mut sigma2 = sample_var;
        let mut ll = 0.0;

        for &r in returns {
            sigma2 = params.omega + params.alpha * r * r + params.beta * sigma2;
            sigma2 = sigma2.max(1e-12);
            ll += -0.5 * (sigma2.ln() + r * r / sigma2);
        }
        ll / n as f64
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ─── GJR-GARCH(1,1) — Asymmetric Volatility with Leverage Effect ────────────
// ═══════════════════════════════════════════════════════════════════════════════
//
// σ²_t = ω + (α + γ·I_{t-1}) · ε²_{t-1} + β · σ²_{t-1}
//
// where I_{t-1} = 1 if ε_{t-1} < 0 (leverage indicator)
//
// γ > 0 means negative returns amplify volatility more than positive returns.
// Empirically, γ ranges from 0.05 to 0.12 for equity indices.

#[derive(Debug, Clone)]
pub struct GjrGarchParams {
    pub omega: f64, // long-run variance weight
    pub alpha: f64, // ARCH coefficient (magnitude effect)
    pub beta: f64,  // GARCH coefficient (persistence)
    pub gamma: f64, // leverage coefficient (asymmetry)
}

impl GjrGarchParams {
    /// Default parameters calibrated from equity index research.
    /// γ = 0.08 provides moderate leverage effect.
    pub fn equity_default() -> Self {
        Self {
            omega: 0.000005,
            alpha: 0.05,
            beta: 0.90,
            gamma: 0.08,
        }
    }

    pub fn is_stationary(&self) -> bool {
        // For GJR-GARCH: α + β + γ/2 < 1 (under symmetric innovation distribution)
        self.alpha + self.beta + self.gamma / 2.0 < 1.0
    }

    pub fn long_run_variance(&self) -> f64 {
        let denom = 1.0 - self.alpha - self.beta - self.gamma / 2.0;
        if denom <= 0.0 {
            return f64::INFINITY;
        }
        self.omega / denom
    }

    pub fn persistence(&self) -> f64 {
        self.alpha + self.beta + self.gamma / 2.0
    }
}

#[derive(Debug, Clone)]
pub struct GjrGarchState {
    pub params: GjrGarchParams,
    pub conditional_variance: f64,
    pub last_return: f64,
    pub variance_history: Vec<f64>,
}

impl GjrGarchState {
    pub fn new(params: GjrGarchParams, initial_variance: f64) -> Self {
        Self {
            conditional_variance: initial_variance,
            last_return: 0.0,
            variance_history: vec![initial_variance],
            params,
        }
    }

    pub fn update(&mut self, return_t: f64) {
        let leverage = if return_t < 0.0 { 1.0 } else { 0.0 };
        let new_var = self.params.omega
            + (self.params.alpha + self.params.gamma * leverage) * return_t.powi(2)
            + self.params.beta * self.conditional_variance;
        self.conditional_variance = new_var.max(1e-10);
        self.last_return = return_t;
        self.variance_history.push(self.conditional_variance);
    }

    pub fn current_vol_annualized(&self) -> f64 {
        (self.conditional_variance * 252.0).sqrt()
    }

    pub fn forecast(&self, h: usize) -> f64 {
        let lr_var = self.params.long_run_variance();
        if lr_var.is_infinite() {
            return self.conditional_variance;
        }
        let persistence_h = self.params.persistence().powi(h as i32);
        lr_var + persistence_h * (self.conditional_variance - lr_var)
    }

    pub fn forecast_vol_annualized(&self, h: usize) -> f64 {
        (self.forecast(h) * 252.0).sqrt()
    }

    pub fn var_1day(&self, z_score: f64, position_value: f64) -> f64 {
        z_score * self.conditional_variance.sqrt() * position_value
    }
}

pub struct RollingGarch {
    states: std::collections::HashMap<String, GarchState>,
    return_history: std::collections::HashMap<String, Vec<f64>>,
    min_history: usize,
    reestimate_every: usize,
    tick_counts: std::collections::HashMap<String, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify GARCH(1,1) variance converges to ω/(1-α-β) after many updates.
    /// Feed returns whose squared magnitude equals long_run_var so the
    /// fixed-point equation ω + α·lr + β·var* = var* is satisfied at var* = lr.
    /// (Zero returns converge to ω/(1-β), not the full long-run variance.)
    #[test]
    fn test_garch_sigma_convergence() {
        let params = GarchParams {
            omega: 0.000001,
            alpha: 0.09,
            beta: 0.90,
        };
        let long_run_var = params.long_run_variance(); // 0.000001 / 0.01 = 0.0001
        assert!(
            (long_run_var - 0.0001).abs() < 1e-8,
            "Long-run variance formula wrong"
        );

        // Feed returns at sqrt(lr) so each step's alpha term = α·lr,
        // making the fixed point exactly long_run_var.
        let r = long_run_var.sqrt(); // 0.01
        let mut state = GarchState::new(params.clone(), 0.001); // start far from LR
        for _ in 0..500 {
            state.update(r);
        }
        let relative_error = (state.conditional_variance - long_run_var).abs() / long_run_var;
        assert!(
            relative_error < 0.01,
            "Variance didn't converge: got {}, expected {}, err={:.4}%",
            state.conditional_variance,
            long_run_var,
            relative_error * 100.0
        );
    }

    /// Inject a 5-sigma shock and verify variance spikes then exponentially decays.
    #[test]
    fn test_garch_shock_response() {
        let params = GarchParams {
            omega: 0.000001,
            alpha: 0.09,
            beta: 0.90,
        };
        let long_run_var = params.long_run_variance();
        let mut state = GarchState::new(params.clone(), long_run_var);

        // Inject a 5-sigma shock (return = 5 * sqrt(long_run_var))
        let shock = 5.0 * long_run_var.sqrt();
        state.update(shock);
        let post_shock_var = state.conditional_variance;
        assert!(
            post_shock_var > long_run_var * 2.0,
            "Variance should spike after shock: got {}, baseline {}",
            post_shock_var,
            long_run_var
        );

        // Decay back — feed returns at sqrt(lr) so the fixed point IS long_run_var.
        // (Zero returns pull variance toward ω/(1-β), not the full LR.)
        let r = long_run_var.sqrt();
        for _ in 0..200 {
            state.update(r);
        }
        let after_decay = state.conditional_variance;
        let relative_to_lr = (after_decay - long_run_var).abs() / long_run_var;
        assert!(
            relative_to_lr < 0.05,
            "Variance should decay back to LR after 200 steps: got {}, LR={}",
            after_decay,
            long_run_var
        );

        // Verify half-life formula
        let half_life = params.shock_half_life();
        assert!(
            half_life > 0.0 && half_life.is_finite(),
            "Half-life should be positive finite: got {}",
            half_life
        );
    }

    /// When α+β ≥ 1, the process is non-stationary. long_run_variance() must return INFINITY.
    #[test]
    fn test_garch_nonstationarity_guard() {
        let params = GarchParams {
            omega: 0.00001,
            alpha: 0.15,
            beta: 0.90,
        };
        assert!(!params.is_stationary());
        assert!(params.long_run_variance().is_infinite());

        // Exactly 1.0
        let params2 = GarchParams {
            omega: 0.00001,
            alpha: 0.10,
            beta: 0.90,
        };
        assert!(!params2.is_stationary());
        assert!(params2.long_run_variance().is_infinite());
    }

    /// Verify estimator recovers known params from synthetic GARCH(1,1) data.
    #[test]
    fn test_garch_estimator_fit_recovers_known_params() {
        let true_omega = 0.000002;
        let true_alpha = 0.08;
        let true_beta = 0.90;

        // Generate 1000 synthetic returns with known GARCH(1,1) process
        let mut rng_state: u64 = 42;
        let mut sigma2: f64 = true_omega / (1.0 - true_alpha - true_beta);
        let mut returns = Vec::with_capacity(1000);
        for _ in 0..1000 {
            // Simple xorshift pseudo-normal
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            let u = (rng_state as f64) / (u64::MAX as f64);
            // Box-Muller approximation (crude but deterministic)
            let z = (u * 2.0 - 1.0) * 2.5; // uniform-ish spread

            let r = z * sigma2.sqrt();
            returns.push(r);
            sigma2 = true_omega + true_alpha * r * r + true_beta * sigma2;
            sigma2 = sigma2.max(1e-10);
        }

        let result = GarchEstimator::fit(&returns);
        assert!(
            result.is_some(),
            "Estimator should converge on 1000 samples"
        );
        let (fitted, _ll) = result.unwrap();

        assert!(fitted.is_stationary(), "Fitted params should be stationary");
        assert!(
            (fitted.alpha - true_alpha).abs() < 0.05,
            "Alpha: expected ~{}, got {}",
            true_alpha,
            fitted.alpha
        );
        assert!(
            (fitted.beta - true_beta).abs() < 0.10,
            "Beta: expected ~{}, got {}",
            true_beta,
            fitted.beta
        );
        assert!(fitted.persistence() < 1.0, "Persistence must be < 1");
    }

    /// Verify forecast formula matches manual calculation.
    /// Use persistence=0.95 so 0.95^100 ≈ 0.006 — close enough to LR.
    /// (With persistence=0.99: 0.99^100 = 0.366 → forecast still 2.5× LR.)
    #[test]
    fn test_garch_forecast_horizon() {
        let params = GarchParams {
            omega: 0.000010,
            alpha: 0.10,
            beta: 0.85,
        };
        let lr = params.long_run_variance(); // 0.0002
        let current = 0.0005; // above long-run
        let state = GarchState::new(params.clone(), current);

        // h=1 forecast should be between current and long-run
        let f1 = state.forecast(1);
        assert!(
            f1 < current && f1 > lr,
            "1-step forecast {} should be between current {} and LR {}",
            f1,
            current,
            lr
        );

        // h=100 forecast should be very close to long-run
        let f100 = state.forecast(100);
        assert!(
            (f100 - lr).abs() / lr < 0.01,
            "100-step forecast {} should ≈ LR {}",
            f100,
            lr
        );
    }

    /// Estimator returns None with < 50 samples.
    #[test]
    fn test_garch_estimator_insufficient_data() {
        let returns: Vec<f64> = (0..30).map(|i| (i as f64 * 0.001).sin() * 0.01).collect();
        assert!(GarchEstimator::fit(&returns).is_none());
    }

    /// GARCH(1,1) conditional variance should feed correctly into parametric VaR:
    /// VaR_95 = 1.645 × σ_daily × position_value
    #[test]
    fn test_garch_var_integration() {
        let params = GarchParams {
            omega: 0.000001,
            alpha: 0.09,
            beta: 0.90,
        };
        let state = GarchState::new(params, 0.0001); // daily variance = 0.0001, σ = 0.01

        let position_value = 100_000.0;
        let z_95 = 1.645;
        let var_1d = state.var_1day(z_95, position_value);

        // VaR = 1.645 * sqrt(0.0001) * 100000 = 1.645 * 0.01 * 100000 = 1645.0
        assert!(
            (var_1d - 1645.0).abs() < 1.0,
            "GARCH VaR integration: expected ~1645, got {:.2}",
            var_1d
        );

        let z_99 = 2.326;
        let var_99 = state.var_1day(z_99, position_value);
        assert!(
            var_99 > var_1d,
            "VaR99 ({:.2}) must exceed VaR95 ({:.2})",
            var_99,
            var_1d
        );
    }

    /// Higher persistence (α+β closer to 1) → slower variance decay after shock.
    /// Both models share the same long-run variance so the comparison is fair.
    #[test]
    fn test_garch_persistence_affects_decay_speed() {
        let long_run_target = 0.0001;

        // Both models: omega = lr * (1 - alpha - beta) → same long-run variance.
        // Low persistence = 0.90
        let params_low = GarchParams {
            alpha: 0.10,
            beta: 0.80,
            omega: long_run_target * (1.0 - 0.10 - 0.80), // persistence=0.90
        };
        // High persistence = 0.95
        let params_high = GarchParams {
            alpha: 0.05,
            beta: 0.90,
            omega: long_run_target * (1.0 - 0.05 - 0.90), // persistence=0.95
        };

        let start = long_run_target * 10.0; // both start at 10× LR
        let mut state_low = GarchState::new(params_low, start);
        let mut state_high = GarchState::new(params_high, start);

        // Feed returns at sqrt(lr) so both converge toward long_run_target.
        let r = long_run_target.sqrt();
        for _ in 0..50 {
            state_low.update(r);
            state_high.update(r);
        }

        let excess_low = state_low.conditional_variance - long_run_target;
        let excess_high = state_high.conditional_variance - long_run_target;

        // High-persistence should retain MORE excess above LR after 50 steps.
        assert!(
            excess_high > excess_low,
            "High-persistence GARCH should decay slower: excess_low={:.8}, excess_high={:.8}",
            excess_low,
            excess_high
        );
    }

    // ── GJR-GARCH Tests ──────────────────────────────────────────────────────

    /// Core leverage effect: negative returns should produce HIGHER variance
    /// than positive returns of the same magnitude.
    #[test]
    fn test_gjr_garch_leverage_effect() {
        let params = GjrGarchParams::equity_default();
        let initial_var = 0.0001;

        // Positive return path
        let mut state_pos = GjrGarchState::new(params.clone(), initial_var);
        state_pos.update(0.02); // +2% return

        // Negative return path (same magnitude)
        let mut state_neg = GjrGarchState::new(params, initial_var);
        state_neg.update(-0.02); // -2% return

        // Negative return should produce higher variance due to γ leverage term
        assert!(
            state_neg.conditional_variance > state_pos.conditional_variance,
            "GJR leverage effect: neg_var={:.8} should > pos_var={:.8}",
            state_neg.conditional_variance,
            state_pos.conditional_variance
        );

        // The difference should be exactly γ * ε² = 0.08 * 0.0004 = 0.000032
        let expected_diff = 0.08 * 0.02_f64.powi(2);
        let actual_diff = state_neg.conditional_variance - state_pos.conditional_variance;
        assert!(
            (actual_diff - expected_diff).abs() < 1e-10,
            "Leverage diff: expected {:.8}, got {:.8}",
            expected_diff,
            actual_diff
        );
    }

    /// GJR-GARCH with default equity params should be stationary.
    #[test]
    fn test_gjr_garch_stationarity() {
        let params = GjrGarchParams::equity_default();
        assert!(
            params.is_stationary(),
            "Default equity GJR-GARCH should be stationary: persistence={:.4}",
            params.persistence()
        );
        assert!(params.long_run_variance() > 0.0 && params.long_run_variance().is_finite());
    }

    /// GJR-GARCH variance should converge to long-run under alternating returns.
    /// We use alternating +/- so the leverage term fires ~50% of the time,
    /// matching the γ/2 assumption in the long-run variance formula.
    #[test]
    fn test_gjr_garch_convergence() {
        let params = GjrGarchParams::equity_default();
        let lr = params.long_run_variance();
        let mut state = GjrGarchState::new(params, lr * 5.0); // start high

        // Alternate +/- returns at sqrt(lr) magnitude
        let r = lr.sqrt();
        for i in 0..1000 {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            state.update(r * sign);
        }

        let relative_error = (state.conditional_variance - lr).abs() / lr;
        assert!(
            relative_error < 0.15,
            "GJR-GARCH didn't converge: got {:.8}, expected {:.8}, err={:.2}%",
            state.conditional_variance,
            lr,
            relative_error * 100.0
        );
    }

    /// GJR-GARCH annualized vol should be in a sensible range.
    #[test]
    fn test_gjr_garch_annualized_vol() {
        let params = GjrGarchParams {
            omega: 0.000005,
            alpha: 0.05,
            beta: 0.90,
            gamma: 0.08,
        };
        let state = GjrGarchState::new(params, 0.0001); // daily var = 0.01%
        let vol = state.current_vol_annualized();
        // sqrt(0.0001 * 252) = sqrt(0.0252) ≈ 0.159 = 15.9%
        assert!(
            vol > 0.10 && vol < 0.25,
            "Annualized vol should be 10-25%, got {:.2}%",
            vol * 100.0
        );
    }
}

impl RollingGarch {
    pub fn new(min_history: usize, reestimate_every: usize) -> Self {
        Self {
            states: std::collections::HashMap::new(),
            return_history: std::collections::HashMap::new(),
            min_history,
            reestimate_every,
            tick_counts: std::collections::HashMap::new(),
        }
    }

    pub fn update(&mut self, symbol: &str, log_return: f64) -> Option<f64> {
        let history = self.return_history.entry(symbol.to_string()).or_default();
        history.push(log_return);
        if history.len() > 1000 {
            history.remove(0);
        }

        let tick = self.tick_counts.entry(symbol.to_string()).or_insert(0);
        *tick += 1;

        if (!self.states.contains_key(symbol) || (*tick).is_multiple_of(self.reestimate_every))
            && history.len() >= self.min_history
        {
            if let Some((params, _)) = GarchEstimator::fit(history) {
                let initial_var = history.last().map(|r| r * r).unwrap_or(0.0001);
                let state = GarchState::new(params, initial_var);
                self.states.insert(symbol.to_string(), state);
            }
        }

        if let Some(state) = self.states.get_mut(symbol) {
            state.update(log_return);
            Some(state.current_vol_annualized())
        } else {
            None
        }
    }

    pub fn current_vol(&self, symbol: &str) -> Option<f64> {
        self.states.get(symbol).map(|s| s.current_vol_annualized())
    }

    pub fn forecast_vol(&self, symbol: &str, days_ahead: usize) -> Option<f64> {
        self.states
            .get(symbol)
            .map(|s| s.forecast_vol_annualized(days_ahead))
    }
}
