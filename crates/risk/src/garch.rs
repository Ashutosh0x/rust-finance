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
        if self.alpha + self.beta >= 1.0 { return f64::INFINITY; }
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
        if p <= 0.0 || p >= 1.0 { return f64::NAN; }
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
            + self.params.alpha * self.last_return.powi(2)
            + self.params.beta  * self.conditional_variance;
        self.conditional_variance = new_var.max(1e-10);
        self.last_return = return_t;
        self.variance_history.push(self.conditional_variance);
    }

    pub fn current_vol_annualized(&self) -> f64 {
        (self.conditional_variance * 252.0).sqrt()
    }

    pub fn forecast(&self, h: usize) -> f64 {
        let lr_var = self.params.long_run_variance();
        if lr_var.is_infinite() { return self.conditional_variance; }
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
        if returns.len() < 50 { return None; }

        let n = returns.len() as f64;
        let sample_var: f64 = {
            let mean = returns.iter().sum::<f64>() / n;
            returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0)
        };

        let mut best_ll = f64::NEG_INFINITY;
        let mut best_params = GarchParams { omega: sample_var * 0.05, alpha: 0.05, beta: 0.90 };

        let alpha_grid = [0.03, 0.05, 0.08, 0.10, 0.12, 0.15];
        let beta_grid  = [0.80, 0.85, 0.87, 0.90, 0.92, 0.94];

        for &alpha in &alpha_grid {
            for &beta in &beta_grid {
                if alpha + beta >= 0.999 { continue; }
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
        let lr  = 1e-5;

        for _ in 0..200 {
            let ll = Self::log_likelihood(returns, &p);

            let mut pa = p.clone(); pa.alpha += eps;
            let mut pb = p.clone(); pb.beta  += eps;
            let mut po = p.clone(); po.omega += eps;

            let grad_a = (Self::log_likelihood(returns, &pa) - ll) / eps;
            let grad_b = (Self::log_likelihood(returns, &pb) - ll) / eps;
            let grad_o = (Self::log_likelihood(returns, &po) - ll) / eps;

            p.alpha += lr * grad_a;
            p.beta  += lr * grad_b;
            p.omega += lr * grad_o;

            p.alpha = p.alpha.max(1e-6).min(0.5);
            p.beta  = p.beta.max(1e-6);
            p.omega = p.omega.max(1e-12);
            if p.alpha + p.beta >= 0.999 {
                let s = 0.998 / (p.alpha + p.beta);
                p.alpha *= s; p.beta *= s;
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

pub struct RollingGarch {
    states: std::collections::HashMap<String, GarchState>,
    return_history: std::collections::HashMap<String, Vec<f64>>,
    min_history: usize,
    reestimate_every: usize,
    tick_counts: std::collections::HashMap<String, usize>,
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
        if history.len() > 1000 { history.remove(0); }

        let tick = self.tick_counts.entry(symbol.to_string()).or_insert(0);
        *tick += 1;

        if !self.states.contains_key(symbol) || *tick % self.reestimate_every == 0 {
            if history.len() >= self.min_history {
                if let Some((params, _)) = GarchEstimator::fit(history) {
                    let initial_var = history.last().map(|r| r * r).unwrap_or(0.0001);
                    let state = GarchState::new(params, initial_var);
                    self.states.insert(symbol.to_string(), state);
                }
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
        self.states.get(symbol).map(|s| s.forecast_vol_annualized(days_ahead))
    }
}
