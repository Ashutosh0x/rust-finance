// Hull-White One-Factor Short Rate Model
// Bloomberg uses this for CDS valuation and interest rate derivatives

#[derive(Debug, Clone)]
pub struct HullWhiteParams {
    pub mean_reversion: f64,
    pub volatility: f64,
}

pub struct YieldCurve {
    pub points: Vec<(f64, f64)>,
}

impl YieldCurve {
    pub fn zero_rate(&self, t: f64) -> f64 {
        if t <= 0.0 { return self.points.first().map(|(_, r)| *r).unwrap_or(0.05); }
        let n = self.points.len();
        for i in 0..n.saturating_sub(1) {
            let (t1, r1) = self.points[i];
            let (t2, r2) = self.points[i + 1];
            if t <= t2 {
                let frac = (t - t1) / (t2 - t1);
                return r1 + frac * (r2 - r1);
            }
        }
        self.points.last().map(|(_, r)| *r).unwrap_or(0.05)
    }

    pub fn discount_factor(&self, t: f64) -> f64 {
        (-self.zero_rate(t) * t).exp()
    }

    pub fn forward_rate(&self, t: f64) -> f64 {
        let h = 0.001;
        let r1 = self.zero_rate(t);
        let r2 = self.zero_rate(t + h);
        r1 + t * (r2 - r1) / h  
    }
}

fn hw_b(a: f64, t: f64, cap_t: f64) -> f64 {
    if a.abs() < 1e-8 { cap_t - t }
    else { (1.0 - (-(a * (cap_t - t))).exp()) / a }
}

fn hw_ln_a(a: f64, sigma: f64, t: f64, cap_t: f64, curve: &YieldCurve) -> f64 {
    let b = hw_b(a, t, cap_t);
    let p0t  = curve.discount_factor(t);
    let p0T  = curve.discount_factor(cap_t);
    let f0t  = curve.forward_rate(t);
    let sigma2_term = if a.abs() < 1e-8 {
        sigma * sigma * t * b * b / 2.0
    } else {
        (sigma * sigma / (4.0 * a)) * (1.0 - (-2.0 * a * t).exp()) * b * b
    };
    (p0T / p0t).ln() + b * f0t - sigma2_term
}

pub fn hw_bond_price(p: &HullWhiteParams, curve: &YieldCurve, r_t: f64, t: f64, cap_t: f64) -> f64 {
    let b = hw_b(p.mean_reversion, t, cap_t);
    let ln_a = hw_ln_a(p.mean_reversion, p.volatility, t, cap_t, curve);
    (ln_a - b * r_t).exp()
}

#[derive(Debug, Clone)]
pub struct HwBondOptionInput {
    pub option_maturity: f64,    
    pub bond_maturity: f64,      
    pub strike: f64,             
    pub face_value: f64,         
    pub is_call: bool,
    pub current_rate: f64,       
}

pub fn hw_bond_option(p: &HullWhiteParams, curve: &YieldCurve, input: &HwBondOptionInput) -> f64 {
    let a = p.mean_reversion;
    let sigma = p.volatility;
    let t_opt = input.option_maturity;
    let t_bond = input.bond_maturity;

    let b = hw_b(a, t_opt, t_bond);
    let sigma_p = sigma * b * ((1.0 - (-2.0 * a * t_opt).exp()) / (2.0 * a)).sqrt();

    if sigma_p <= 0.0 { return 0.0; }

    let p0s = curve.discount_factor(t_opt);   
    let p0t = curve.discount_factor(t_bond);  
    let k   = input.strike;
    let fv  = input.face_value;

    let h = (p0t / (p0s * k)).ln() / sigma_p + sigma_p / 2.0;

    let call_price = fv * (p0t * super::bsm::norm_cdf(h) - k * p0s * super::bsm::norm_cdf(h - sigma_p));
    let put_price  = fv * (k * p0s * super::bsm::norm_cdf(-(h - sigma_p)) - p0t * super::bsm::norm_cdf(-h));

    if input.is_call { call_price } else { put_price }
}

#[derive(Debug, Clone)]
pub struct TreeParams {
    pub spot: f64,
    pub strike: f64,
    pub rate: f64,
    pub dividend_yield: f64,
    pub volatility: f64,
    pub time_to_expiry: f64,
    pub steps: usize,
    pub is_call: bool,
    pub is_american: bool,
}

pub fn trinomial_tree_price(p: &TreeParams) -> f64 {
    let n     = p.steps;
    let dt    = p.time_to_expiry / n as f64;
    let sigma = p.volatility;
    let r     = p.rate;
    let q     = p.dividend_yield;
    let s     = p.spot;
    let k     = p.strike;

    let u = (sigma * (2.0 * dt).sqrt()).exp();
    let d = 1.0 / u;
    
    let e_r  = ((r - q) * dt / 2.0).exp();

    let pu = ((e_r - d.sqrt()) / (u.sqrt() - d.sqrt())).powi(2);
    let pd = ((u.sqrt() - e_r) / (u.sqrt() - d.sqrt())).powi(2);
    let pm = 1.0 - pu - pd;

    let discount = (-r * dt).exp();
    let n_nodes = 2 * n + 1;

    let mut prices: Vec<f64> = (0..n_nodes)
        .map(|i| {
            let j = i as i64 - n as i64;
            if j >= 0 { s * u.powi(j as i32) }
            else       { s * d.powi((-j) as i32) }
        })
        .collect();

    let mut values: Vec<f64> = prices.iter().map(|&sp| {
        let intrinsic = if p.is_call { (sp - k).max(0.0) } else { (k - sp).max(0.0) };
        intrinsic
    }).collect();

    for step in (0..n).rev() {
        let n_curr = 2 * step + 1;
        let mut new_values = vec![0.0_f64; n_curr];

        for i in 0..n_curr {
            let v_up  = values[i + 2];
            let v_mid = values[i + 1];
            let v_dn  = values[i];

            let continuation = discount * (pu * v_up + pm * v_mid + pd * v_dn);

            if p.is_american {
                let j = i as i64 - step as i64;
                let sp = if j >= 0 { s * u.powi(j as i32) } else { s * d.powi((-j) as i32) };
                let intrinsic = if p.is_call { (sp - k).max(0.0) } else { (k - sp).max(0.0) };
                new_values[i] = continuation.max(intrinsic);
            } else {
                new_values[i] = continuation;
            }
        }
        values = new_values;
    }

    values[0]
}

pub fn binomial_crr_price(p: &TreeParams) -> f64 {
    let n     = p.steps;
    let dt    = p.time_to_expiry / n as f64;
    let u     = (p.volatility * dt.sqrt()).exp();
    let d     = 1.0 / u;
    let pu    = (((p.rate - p.dividend_yield) * dt).exp() - d) / (u - d);
    let pd    = 1.0 - pu;
    let disc  = (-p.rate * dt).exp();

    let mut values: Vec<f64> = (0..=n).map(|j| {
        let sp = p.spot * u.powi(j as i32) * d.powi((n - j) as i32);
        if p.is_call { (sp - p.strike).max(0.0) } else { (p.strike - sp).max(0.0) }
    }).collect();

    for step in (0..n).rev() {
        for j in 0..=step {
            let continuation = disc * (pu * values[j + 1] + pd * values[j]);
            if p.is_american {
                let sp = p.spot * u.powi(j as i32) * d.powi((step - j) as i32);
                let intrinsic = if p.is_call { (sp - p.strike).max(0.0) } else { (p.strike - sp).max(0.0) };
                values[j] = continuation.max(intrinsic);
            } else {
                values[j] = continuation;
            }
        }
    }
    values[0]
}

pub fn early_exercise_premium(p: &TreeParams) -> f64 {
    let american = if p.steps > 50 {
        trinomial_tree_price(p)
    } else {
        binomial_crr_price(p)
    };

    let european = {
        let mut ep = p.clone();
        ep.is_american = false;
        if p.steps > 50 { trinomial_tree_price(&ep) } else { binomial_crr_price(&ep) }
    };

    (american - european).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_european_put_vs_bsm() {
        let p = TreeParams {
            spot: 100.0, strike: 105.0, rate: 0.05, dividend_yield: 0.0,
            volatility: 0.20, time_to_expiry: 1.0,
            steps: 200, is_call: false, is_american: false,
        };
        let tree_put = trinomial_tree_price(&p);

        let bsm = super::super::bsm::price(&super::super::bsm::BsmParams {
            spot: 100.0, strike: 105.0, rate: 0.05, dividend_yield: 0.0,
            volatility: 0.20, time_to_expiry: 1.0,
        }).unwrap();

        assert!((tree_put - bsm.put_price).abs() < 0.10);
    }

    #[test]
    fn test_american_put_geq_european() {
        let base = TreeParams {
            spot: 100.0, strike: 105.0, rate: 0.05, dividend_yield: 0.0,
            volatility: 0.20, time_to_expiry: 1.0, steps: 100,
            is_call: false, is_american: false,
        };
        let euro = trinomial_tree_price(&base);
        let amer = trinomial_tree_price(&TreeParams { is_american: true, ..base });
        assert!(amer >= euro - 1e-6);
    }

    #[test]
    fn test_american_call_no_dividend_no_early_exercise() {
        let p = TreeParams {
            spot: 100.0, strike: 100.0, rate: 0.05, dividend_yield: 0.0,
            volatility: 0.25, time_to_expiry: 1.0, steps: 100,
            is_call: true, is_american: true,
        };
        let premium = early_exercise_premium(&p);
        assert!(premium < 0.05);
    }

    #[test]
    fn test_hw_bond_price_at_zero_time() {
        let p = HullWhiteParams { mean_reversion: 0.10, volatility: 0.01 };
        let curve = YieldCurve {
            points: vec![(0.0, 0.05), (1.0, 0.05), (5.0, 0.05), (10.0, 0.05)],
        };
        let price_5y = hw_bond_price(&p, &curve, 0.05, 0.0, 5.0);
        let expected = (-0.05 * 5.0_f64).exp();
        assert!((price_5y - expected).abs() < 0.01);
    }
}
