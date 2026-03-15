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
    let w = 1.0 - 1.0 / (2.0 * PI).sqrt() * (-l * l / 2.0).exp() *
        (0.319381530 * k - 0.356563782 * k * k + 1.781477937 * k * k * k -
         1.821255978 * k * k * k * k + 1.330274429 * k * k * k * k * k);
    if x < 0.0 { 1.0 - w } else { w }
}

pub fn price(p: &BsmParams) -> Option<BsmResult> {
    if p.time_to_expiry <= 0.0 || p.volatility <= 0.0 { return None; }
    let sqrt_t = p.time_to_expiry.sqrt();
    let d1 = ((p.spot / p.strike).ln() + (p.rate - p.dividend_yield + 0.5 * p.volatility * p.volatility) * p.time_to_expiry) / (p.volatility * sqrt_t);
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
    let call_theta = theta_term1 - p.rate * p.strike * exp_rt * nd2 + p.dividend_yield * p.spot * exp_qt * nd1;
    let put_theta = theta_term1 + p.rate * p.strike * exp_rt * n_d2 - p.dividend_yield * p.spot * exp_qt * n_d1;

    let call_rho = p.strike * p.time_to_expiry * exp_rt * nd2;
    let put_rho = -p.strike * p.time_to_expiry * exp_rt * n_d2;

    let vanna = -exp_qt * pdf_d1 * d2 / p.volatility;
    let charm = exp_qt * (p.dividend_yield * nd1 - pdf_d1 * (2.0 * (p.rate - p.dividend_yield) * p.time_to_expiry - d2 * p.volatility * sqrt_t) / (2.0 * p.time_to_expiry * p.volatility * sqrt_t));

    Some(BsmResult {
        call_price, put_price, d1, d2, call_delta, put_delta, gamma, vega, call_theta, put_theta, call_rho, put_rho, charm, vanna
    })
}

pub fn implied_vol(market_price: f64, spot: f64, strike: f64, rate: f64, div_yield: f64, tte: f64, is_call: bool, max_iter: usize, tol: f64) -> Option<f64> {
    let mut sigma = ((2.0 * PI).sqrt() / spot) * (market_price / tte.sqrt());
    if sigma.is_nan() || sigma <= 0.0 { sigma = 0.2; }

    for _ in 0..max_iter {
        let params = BsmParams { spot, strike, rate, dividend_yield: div_yield, volatility: sigma, time_to_expiry: tte };
        if let Some(res) = price(&params) {
            let model_price = if is_call { res.call_price } else { res.put_price };
            let diff = model_price - market_price;
            if diff.abs() < tol { return Some(sigma); }
            if res.vega < 1e-8 { break; }
            sigma -= diff / res.vega;
        } else {
            break;
        }
    }
    Some(sigma.max(0.0001))
}
