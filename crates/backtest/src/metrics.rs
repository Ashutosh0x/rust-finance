pub fn calculate_max_drawdown(equity_curve: &[f64]) -> f64 {
    let mut max_dd = 0.0;
    let mut peak = f64::MIN;

    for &equity in equity_curve {
        if equity > peak {
            peak = equity;
        }
        let dd = (peak - equity) / peak;
        if dd > max_dd {
            max_dd = dd;
        }
    }
    max_dd
}

pub fn calculate_sharpe_ratio(returns: &[f64], risk_free_rate: f64) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }
    
    let sum: f64 = returns.iter().sum();
    let mean = sum / returns.len() as f64;
    
    let variance: f64 = returns.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / returns.len() as f64;
    let std_dev = variance.sqrt();
    
    if std_dev == 0.0 {
        return 0.0;
    }
    
    // Annualized multiplier assuming 252 trading days. If returns are daily.
    // For tick/M1 data, we adjust accordingly. Assuming these are per-trade returns for the stub.
    (mean - risk_free_rate) / std_dev
}

pub fn calculate_sortino_ratio(returns: &[f64], risk_free_rate: f64, target_return: f64) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }

    let sum: f64 = returns.iter().sum();
    let mean = sum / returns.len() as f64;

    let downside_variance: f64 = returns.iter()
        .filter(|&&x| x < target_return)
        .map(|&x| (x - target_return).powi(2))
        .sum::<f64>() / returns.len() as f64;

    let downside_deviation = downside_variance.sqrt();

    if downside_deviation == 0.0 {
        return 0.0;
    }

    (mean - risk_free_rate) / downside_deviation
}

pub fn calculate_calmar_ratio(annualized_return: f64, max_drawdown: f64) -> f64 {
    if max_drawdown == 0.0 {
        return 0.0;
    }
    annualized_return / max_drawdown
}
