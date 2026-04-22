#[derive(Debug, Clone)]
pub struct BondPricingInputs {
    pub step1_direct_obs_price: f64,
    pub step1_obs_quality: f64,

    pub step2_historical_corr_price: f64,
    pub step2_corr_confidence: f64,

    pub step3_comparable_rv_price: f64,
}

#[derive(Debug, Clone)]
pub struct BvalOutput {
    pub final_price: f64,
    pub bval_score: f64,
}

pub fn bval_3step_pricer(inputs: &BondPricingInputs) -> BvalOutput {
    let mut w1 = inputs.step1_obs_quality * 2.0;
    let mut w2 = inputs.step2_corr_confidence;
    let mut w3 = 0.5;

    let total_w = w1 + w2 + w3;
    w1 /= total_w;
    w2 /= total_w;
    w3 /= total_w;

    let final_price = w1 * inputs.step1_direct_obs_price
        + w2 * inputs.step2_historical_corr_price
        + w3 * inputs.step3_comparable_rv_price;

    let bval_score =
        (inputs.step1_obs_quality * 0.5 + inputs.step2_corr_confidence * 0.3 + 0.2) * 10.0;

    BvalOutput {
        final_price,
        bval_score: bval_score.clamp(0.0, 10.0),
    }
}

pub fn bond_duration_metrics(cashflows: &[(f64, f64)], ytm: f64, freq: f64) -> (f64, f64, f64) {
    let r = ytm / freq;
    let mut price = 0.0;
    let mut mac_dur = 0.0;
    let mut conv = 0.0;

    for &(t_years, cf) in cashflows {
        let t_periods = t_years * freq;
        let df = 1.0 / (1.0 + r).powf(t_periods);
        let pv = cf * df;

        price += pv;
        mac_dur += t_years * pv;
        conv += t_years * (t_years + 1.0 / freq) * pv;
    }

    if price == 0.0 {
        return (0.0, 0.0, 0.0);
    }
    mac_dur /= price;
    conv /= price * (1.0 + r).powi(2);

    let mod_dur = mac_dur / (1.0 + r);
    let dv01 = price * mod_dur * 0.0001;

    (mod_dur, conv, dv01)
}

pub fn bond_price_change_approximation(
    mod_dur: f64,
    conv: f64,
    initial_price: f64,
    yield_change: f64,
) -> f64 {
    let pct_change = -mod_dur * yield_change + 0.5 * conv * yield_change.powi(2);
    initial_price * pct_change
}
