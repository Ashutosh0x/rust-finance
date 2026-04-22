#[derive(Debug, Clone)]
pub struct SabrParams {
    pub alpha: f64,
    pub beta: f64,
    pub rho: f64,
    pub nu: f64,
}

pub fn implied_vol(p: &SabrParams, forward: f64, strike: f64, tte: f64) -> f64 {
    if forward <= 0.0 || strike <= 0.0 {
        return p.alpha;
    }

    let f_k = forward * strike;
    let log_fk = (forward / strike).ln();

    if (forward - strike).abs() < 1e-6 || log_fk.abs() < 1e-6 {
        let f_beta_1 = forward.powf(p.beta - 1.0);
        let term1 = p.alpha * f_beta_1;
        let term2 =
            ((1.0 - p.beta).powi(2) / 24.0) * p.alpha.powi(2) * forward.powf(2.0 * p.beta - 2.0);
        let term3 = (p.rho * p.beta * p.nu * p.alpha) / (4.0 * forward.powf(1.0 - p.beta));
        let term4 = (2.0 - 3.0 * p.rho.powi(2)) / 24.0 * p.nu.powi(2);
        return term1 * (1.0 + (term2 + term3 + term4) * tte);
    }

    let z = (p.nu / p.alpha) * f_k.powf((1.0 - p.beta) / 2.0) * log_fk;
    let x_safe = ((1.0 - 2.0 * p.rho * z + z.powi(2)).sqrt() + z - p.rho).ln() - (1.0 - p.rho).ln();

    let term1_denom = f_k.powf((1.0 - p.beta) / 2.0)
        * (1.0
            + (1.0 - p.beta).powi(2) / 24.0 * log_fk.powi(2)
            + (1.0 - p.beta).powi(4) / 1920.0 * log_fk.powi(4));
    let term1 = p.alpha / term1_denom;

    let term2_val = if z.abs() < 1e-6 { 1.0 } else { z / x_safe };

    let term3 = 1.0
        + (((1.0 - p.beta).powi(2) / 24.0) * p.alpha.powi(2) / f_k.powf(1.0 - p.beta)
            + (p.rho * p.beta * p.nu * p.alpha) / (4.0 * f_k.powf((1.0 - p.beta) / 2.0))
            + (2.0 - 3.0 * p.rho.powi(2)) / 24.0 * p.nu.powi(2))
            * tte;

    term1 * term2_val * term3
}

pub struct VolSurface {
    pub params: SabrParams,
    pub forward: f64,
}

impl VolSurface {
    pub fn interpolate(&self, strike: f64, tte: f64) -> f64 {
        implied_vol(&self.params, self.forward, strike, tte)
    }
}

pub struct SabrCalibrator;

impl SabrCalibrator {
    pub fn calibrate(
        forward: f64,
        _tte: f64,
        beta: f64,
        market_vols: &[(f64, f64)],
        _max_iter: usize,
    ) -> SabrParams {
        let atm_vol = market_vols
            .iter()
            .min_by(|a, b| {
                (a.0 - forward)
                    .abs()
                    .partial_cmp(&(b.0 - forward).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, v)| *v)
            .unwrap_or(0.20);

        SabrParams {
            alpha: atm_vol * forward.powf(1.0 - beta),
            beta,
            rho: -0.5,
            nu: 0.4,
        }
    }
}
