pub fn calculate_wacc(
    equity: f64,
    debt: f64,
    cost_of_equity: f64,
    cost_of_debt: f64,
    tax_rate: f64,
) -> f64 {
    let v = equity + debt;
    if v == 0.0 {
        return 0.0;
    }
    (equity / v) * cost_of_equity + (debt / v) * cost_of_debt * (1.0 - tax_rate)
}

pub fn mipd_probability_of_default(cds_spread_bps: f64, recovery_rate: f64, t_years: f64) -> f64 {
    let spread_decimal = cds_spread_bps / 10000.0;
    let lambda = spread_decimal / (1.0 - recovery_rate);
    1.0 - (-lambda * t_years).exp()
}

pub fn bquant_multifactor_score(
    pb: f64,
    roe: f64,
    ocf_to_assets: f64,
    earnings_yield: f64,
    momentum: f64,
) -> f64 {
    -0.10 * pb + 0.25 * roe + 0.25 * ocf_to_assets + 0.20 * earnings_yield + 0.20 * momentum
}

pub fn universe_ranker_quartiles(scores: &[(String, f64)]) -> Vec<(String, &'static str)> {
    let mut sorted = scores.to_vec();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let n = sorted.len();
    if n == 0 {
        return vec![];
    }
    let qsize = (n / 4).max(1);

    sorted
        .into_iter()
        .enumerate()
        .map(|(i, (symbol, _))| {
            let label = if i < qsize {
                "Long"
            } else if i >= n - qsize {
                "Short"
            } else {
                "Neutral"
            };
            (symbol, label)
        })
        .collect()
}
