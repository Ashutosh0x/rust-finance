use std::f64::consts::PI;

#[derive(Debug, Clone)]
pub struct BsmParams {
    pub spot: f64,
    pub strike: f64,
    pub rate: f64,
    pub dividend_yield: f64,
    pub volatility: f64,
    pub time_to_expiry: f64,
}

#[derive(Debug, Clone)]
pub struct BsmResult {
    pub call_price: f64,
    pub put_price: f64,
    pub d1: f64,
    pub d2: f64,
    pub call_delta: f64,
    pub put_delta: f64,
    pub gamma: f64,
    pub vega: f64,
    pub call_theta: f64,
    pub put_theta: f64,
    pub call_rho: f64,
    pub put_rho: f64,
    pub charm: f64, // ∂Delta/∂T
    pub vanna: f64, // ∂Delta/∂σ
}

pub fn norm_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * PI).sqrt()
}

pub fn norm_cdf(x: f64) -> f64 {
    let l = x.abs();
    let k = 1.0 / (1.0 + 0.2316419 * l);
    let w = 1.0
        - 1.0 / (2.0 * PI).sqrt()
            * (-l * l / 2.0).exp()
            * (0.319381530 * k - 0.356563782 * k * k + 1.781477937 * k * k * k
                - 1.821255978 * k * k * k * k
                + 1.330274429 * k * k * k * k * k);
    if x < 0.0 { 1.0 - w } else { w }
}

pub fn price(p: &BsmParams) -> Option<BsmResult> {
    if p.time_to_expiry <= 0.0 || p.volatility <= 0.0 {
        return None;
    }
    let sqrt_t = p.time_to_expiry.sqrt();
    let d1 = ((p.spot / p.strike).ln()
        + (p.rate - p.dividend_yield + 0.5 * p.volatility * p.volatility) * p.time_to_expiry)
        / (p.volatility * sqrt_t);
    let d2 = d1 - p.volatility * sqrt_t;

    let nd1 = norm_cdf(d1);
    let nd2 = norm_cdf(d2);
    let n_d1 = norm_cdf(-d1);
    let n_d2 = norm_cdf(-d2);
    let pdf_d1 = norm_pdf(d1);

    let exp_qt = (-p.dividend_yield * p.time_to_expiry).exp();
    let exp_rt = (-p.rate * p.time_to_expiry).exp();

    let call_price = p.spot * exp_qt * nd1 - p.strike * exp_rt * nd2;
    let put_price = p.strike * exp_rt * n_d2 - p.spot * exp_qt * n_d1;

    let call_delta = exp_qt * nd1;
    let put_delta = exp_qt * (nd1 - 1.0);

    let gamma = exp_qt * pdf_d1 / (p.spot * p.volatility * sqrt_t);
    let vega = p.spot * exp_qt * pdf_d1 * sqrt_t;

    let theta_term1 = -(p.spot * exp_qt * pdf_d1 * p.volatility) / (2.0 * sqrt_t);
    let call_theta =
        theta_term1 - p.rate * p.strike * exp_rt * nd2 + p.dividend_yield * p.spot * exp_qt * nd1;
    let put_theta =
        theta_term1 + p.rate * p.strike * exp_rt * n_d2 - p.dividend_yield * p.spot * exp_qt * n_d1;

    let call_rho = p.strike * p.time_to_expiry * exp_rt * nd2;
    let put_rho = -p.strike * p.time_to_expiry * exp_rt * n_d2;

    let vanna = -exp_qt * pdf_d1 * d2 / p.volatility;
    let charm = exp_qt
        * (p.dividend_yield * nd1
            - pdf_d1
                * (2.0 * (p.rate - p.dividend_yield) * p.time_to_expiry
                    - d2 * p.volatility * sqrt_t)
                / (2.0 * p.time_to_expiry * p.volatility * sqrt_t));

    Some(BsmResult {
        call_price,
        put_price,
        d1,
        d2,
        call_delta,
        put_delta,
        gamma,
        vega,
        call_theta,
        put_theta,
        call_rho,
        put_rho,
        charm,
        vanna,
    })
}

pub fn implied_vol(
    market_price: f64,
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    is_call: bool,
    max_iter: usize,
    tol: f64,
) -> Option<f64> {
    let mut sigma = ((2.0 * PI).sqrt() / spot) * (market_price / tte.sqrt());
    if sigma.is_nan() || sigma <= 0.0 {
        sigma = 0.2;
    }

    for _ in 0..max_iter {
        let params = BsmParams {
            spot,
            strike,
            rate,
            dividend_yield: div_yield,
            volatility: sigma,
            time_to_expiry: tte,
        };
        if let Some(res) = price(&params) {
            let model_price = if is_call {
                res.call_price
            } else {
                res.put_price
            };
            let diff = model_price - market_price;
            if diff.abs() < tol {
                return Some(sigma);
            }
            if res.vega < 1e-8 {
                break;
            }
            sigma -= diff / res.vega;
        } else {
            break;
        }
    }
    Some(sigma.max(0.0001))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hull_params() -> BsmParams {
        // Hull's Options Tables: S=42, K=40, r=0.10, q=0, σ=0.20, T=0.50
        BsmParams {
            spot: 42.0,
            strike: 40.0,
            rate: 0.10,
            dividend_yield: 0.0,
            volatility: 0.20,
            time_to_expiry: 0.50,
        }
    }

    /// Cross-check against Hull's Table 15.6 reference values.
    #[test]
    fn test_bsm_ref_values_hull() {
        let res = price(&hull_params()).unwrap();
        // Call price ≈ 4.76 (Hull value)
        assert!(
            (res.call_price - 4.76).abs() < 0.02,
            "Call price: expected ~4.76, got {:.4}",
            res.call_price
        );
        // Call delta ≈ 0.7791
        assert!(
            (res.call_delta - 0.7791).abs() < 0.005,
            "Call delta: expected ~0.7791, got {:.4}",
            res.call_delta
        );
        // Gamma = n(d1) / (S·σ·√T) ≈ 0.0497 for Hull params
        assert!(
            (res.gamma - 0.0497).abs() < 0.002,
            "Gamma: expected ~0.0497, got {:.4}",
            res.gamma
        );
    }

    /// Put-call parity: C - P = Se^{-qT} - Ke^{-rT}
    #[test]
    fn test_bsm_put_call_parity() {
        let p = hull_params();
        let res = price(&p).unwrap();
        let lhs = res.call_price - res.put_price;
        let rhs = p.spot * (-p.dividend_yield * p.time_to_expiry).exp()
            - p.strike * (-p.rate * p.time_to_expiry).exp();
        assert!(
            (lhs - rhs).abs() < 1e-8,
            "Put-call parity violated: C-P={:.8}, S*exp(-qT)-K*exp(-rT)={:.8}",
            lhs,
            rhs
        );
    }

    /// Call delta ∈ [0, 1], put delta ∈ [-1, 0].
    #[test]
    fn test_bsm_delta_bounds() {
        for &spot in &[30.0, 40.0, 50.0, 60.0] {
            let p = BsmParams {
                spot,
                ..hull_params()
            };
            let res = price(&p).unwrap();
            assert!(
                res.call_delta >= 0.0 && res.call_delta <= 1.0,
                "Call delta out of [0,1]: {} for spot={}",
                res.call_delta,
                spot
            );
            assert!(
                res.put_delta >= -1.0 && res.put_delta <= 0.0,
                "Put delta out of [-1,0]: {} for spot={}",
                res.put_delta,
                spot
            );
        }
    }

    /// Gamma must always be positive for a live option.
    #[test]
    fn test_bsm_gamma_always_positive() {
        for &spot in &[30.0, 40.0, 50.0, 60.0] {
            let p = BsmParams {
                spot,
                ..hull_params()
            };
            let res = price(&p).unwrap();
            assert!(
                res.gamma > 0.0,
                "Gamma must be > 0: got {} for spot={}",
                res.gamma,
                spot
            );
        }
    }

    /// Vega must always be positive for a live option.
    #[test]
    fn test_bsm_vega_always_positive() {
        for &spot in &[30.0, 40.0, 50.0, 60.0] {
            let p = BsmParams {
                spot,
                ..hull_params()
            };
            let res = price(&p).unwrap();
            assert!(
                res.vega > 0.0,
                "Vega must be > 0: got {} for spot={}",
                res.vega,
                spot
            );
        }
    }

    /// ATM (S≈K) call delta should be approximately 0.5.
    #[test]
    fn test_bsm_atm_delta_near_half() {
        // True ATM requires r=0, q=0 so d1 = σ√T/2 → small → N(d1) ≈ 0.5
        let p = BsmParams {
            spot: 100.0,
            strike: 100.0,
            rate: 0.0,
            dividend_yield: 0.0,
            volatility: 0.20,
            time_to_expiry: 1.0,
        };
        let res = price(&p).unwrap();
        // d1 = σ√T/2 = 0.10, N(0.10) ≈ 0.5398
        assert!(
            (res.call_delta - 0.5).abs() < 0.06,
            "ATM call delta should be ~0.5, got {:.4}",
            res.call_delta
        );

        // With r>0, delta drifts above 0.5 due to forward price effect
        let p_fwd = BsmParams { rate: 0.05, ..p };
        let res_fwd = price(&p_fwd).unwrap();
        assert!(
            res_fwd.call_delta > res.call_delta,
            "Positive rate should push call delta above the r=0 case"
        );
    }

    /// price() returns None when T ≤ 0 or vol ≤ 0.
    #[test]
    fn test_bsm_expired_option() {
        let p = BsmParams {
            time_to_expiry: 0.0,
            ..hull_params()
        };
        assert!(price(&p).is_none(), "Expired option should return None");
        let p2 = BsmParams {
            volatility: 0.0,
            ..hull_params()
        };
        assert!(price(&p2).is_none(), "Zero vol should return None");
        let p3 = BsmParams {
            time_to_expiry: -1.0,
            ..hull_params()
        };
        assert!(price(&p3).is_none(), "Negative T should return None");
    }

    /// Compute BSM price from known vol, recover via implied_vol, assert roundtrip.
    #[test]
    fn test_implied_vol_roundtrip() {
        let known_vol = 0.25;
        let p = BsmParams {
            spot: 100.0,
            strike: 105.0,
            rate: 0.05,
            dividend_yield: 0.0,
            volatility: known_vol,
            time_to_expiry: 0.50,
        };
        let res = price(&p).unwrap();
        let recovered = implied_vol(
            res.call_price,
            p.spot,
            p.strike,
            p.rate,
            p.dividend_yield,
            p.time_to_expiry,
            true,
            100,
            1e-10,
        )
        .unwrap();
        assert!(
            (recovered - known_vol).abs() < 0.001,
            "Implied vol roundtrip: expected {}, got {}",
            known_vol,
            recovered
        );
    }

    /// Ensure vanna and charm are finite and reasonable.
    #[test]
    fn test_bsm_second_order_greeks() {
        let res = price(&hull_params()).unwrap();
        assert!(res.vanna.is_finite(), "Vanna should be finite");
        assert!(res.charm.is_finite(), "Charm should be finite");
    }
}
