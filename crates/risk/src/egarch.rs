// ═══════════════════════════════════════════════════════════════════════════════
// EGARCH(1,1) — Exponential GARCH
//
// log(σ²_t) = ω + α·[|z_{t-1}| - E|z|] + γ·z_{t-1} + β·log(σ²_{t-1})
//
// where z_t = ε_t / σ_t (standardized residual)
//
// Advantages over symmetric GARCH and GJR-GARCH:
// 1. Operates in log-variance space → variance can NEVER go negative
// 2. Captures leverage effect through γ (sign effect)
// 3. Better fit for equity indices in empirical studies
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct EGarchParams {
    pub omega: f64, // constant term
    pub alpha: f64, // magnitude effect (|z| - E|z|)
    pub gamma: f64, // sign/leverage effect
    pub beta: f64,  // persistence (log-variance autoregressive term)
}

impl EGarchParams {
    /// Default parameters calibrated from S&P 500 research.
    pub fn equity_default() -> Self {
        Self {
            omega: -0.10,
            alpha: 0.15,
            gamma: -0.08, // negative = leverage effect (vol rises after negative returns)
            beta: 0.98,
        }
    }

    /// E[|z|] for standard normal distribution = sqrt(2/π)
    pub fn expected_abs_z() -> f64 {
        (2.0 / std::f64::consts::PI).sqrt()
    }
}

#[derive(Debug, Clone)]
pub struct EGarchState {
    pub params: EGarchParams,
    pub log_sigma2: f64, // log(σ²) — operates in log space
    pub last_return: f64,
    pub last_sigma: f64,
    pub variance_history: Vec<f64>,
}

impl EGarchState {
    pub fn new(params: EGarchParams, initial_variance: f64) -> Self {
        let log_sigma2 = initial_variance.ln();
        Self {
            log_sigma2,
            last_return: 0.0,
            last_sigma: initial_variance.sqrt(),
            variance_history: vec![initial_variance],
            params,
        }
    }

    pub fn update(&mut self, return_t: f64) {
        let z = if self.last_sigma > 1e-10 {
            return_t / self.last_sigma
        } else {
            0.0
        };
        let e_abs_z = EGarchParams::expected_abs_z();

        self.log_sigma2 = self.params.omega
            + self.params.alpha * (z.abs() - e_abs_z)
            + self.params.gamma * z  // leverage: negative z amplifies
            + self.params.beta * self.log_sigma2;

        let sigma2 = self.sigma2();
        self.last_return = return_t;
        self.last_sigma = sigma2.sqrt();
        self.variance_history.push(sigma2);
    }

    /// Current conditional variance: exp(log_σ²)
    /// Note: can never be negative (key advantage of EGARCH).
    pub fn sigma2(&self) -> f64 {
        self.log_sigma2.exp().max(1e-15)
    }

    pub fn current_vol_annualized(&self) -> f64 {
        (self.sigma2() * 252.0).sqrt()
    }

    pub fn forecast_vol_annualized(&self, h: usize) -> f64 {
        // Simple h-step forecast: assume persistence^h decay in log space
        let lr_log_var = self.params.omega / (1.0 - self.params.beta);
        let persistence_h = self.params.beta.powi(h as i32);
        let forecast_log = lr_log_var + persistence_h * (self.log_sigma2 - lr_log_var);
        (forecast_log.exp() * 252.0).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_egarch_variance_always_positive() {
        let params = EGarchParams::equity_default();
        let mut state = EGarchState::new(params, 0.0001);

        // Feed extreme returns — variance should never go negative
        for &r in &[0.05, -0.08, 0.10, -0.15, 0.01, -0.01, 0.0] {
            state.update(r);
            assert!(
                state.sigma2() > 0.0,
                "EGARCH variance must always be positive, got {}",
                state.sigma2()
            );
        }
    }

    #[test]
    fn test_egarch_leverage_effect() {
        let params = EGarchParams::equity_default();

        let mut state_pos = EGarchState::new(params.clone(), 0.0001);
        state_pos.update(0.02); // positive return

        let mut state_neg = EGarchState::new(params, 0.0001);
        state_neg.update(-0.02); // negative return (same magnitude)

        // With γ < 0, negative returns should increase vol more
        assert!(
            state_neg.sigma2() > state_pos.sigma2(),
            "EGARCH leverage: neg_var={:.8} should > pos_var={:.8}",
            state_neg.sigma2(),
            state_pos.sigma2()
        );
    }

    #[test]
    fn test_egarch_annualized_vol_sensible() {
        let params = EGarchParams::equity_default();
        let state = EGarchState::new(params, 0.0001);
        let vol = state.current_vol_annualized();
        assert!(
            vol > 0.05 && vol < 1.0,
            "Annualized vol should be 5-100%, got {:.2}%",
            vol * 100.0
        );
    }
}
