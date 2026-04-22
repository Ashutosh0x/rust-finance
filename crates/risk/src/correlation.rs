// ═══════════════════════════════════════════════════════════════════════════════
// Cross-Asset Correlation & Portfolio Concentration
//
// Your 15 assets are analyzed independently — this module adds
// portfolio-level awareness to detect correlated signals.
// ═══════════════════════════════════════════════════════════════════════════════

/// Compute rolling correlation between two return series.
/// Uses a trailing window of `window` observations.
pub fn rolling_correlation(returns_a: &[f64], returns_b: &[f64], window: usize) -> f64 {
    assert_eq!(
        returns_a.len(),
        returns_b.len(),
        "Return series must be same length"
    );
    let n = returns_a.len().min(window);
    if n < 3 {
        return 0.0;
    }

    let a = &returns_a[returns_a.len() - n..];
    let b = &returns_b[returns_b.len() - n..];

    let mean_a: f64 = a.iter().sum::<f64>() / n as f64;
    let mean_b: f64 = b.iter().sum::<f64>() / n as f64;

    let cov: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| (x - mean_a) * (y - mean_b))
        .sum::<f64>()
        / n as f64;
    let std_a: f64 = (a.iter().map(|x| (x - mean_a).powi(2)).sum::<f64>() / n as f64).sqrt();
    let std_b: f64 = (b.iter().map(|y| (y - mean_b).powi(2)).sum::<f64>() / n as f64).sqrt();

    if std_a < 1e-10 || std_b < 1e-10 {
        return 0.0;
    }
    (cov / (std_a * std_b)).clamp(-1.0, 1.0)
}

/// Portfolio-level concentration check.
/// Returns a penalty multiplier (0.7 = 30% penalty) when too many signals
/// point in the same direction, which usually indicates a systematic error
/// rather than genuine diversified alpha.
///
/// Input: vec of (symbol, signal_direction) where direction > 0 = long, < 0 = short.
pub fn concentration_penalty(signals: &[(String, f64)]) -> f64 {
    if signals.is_empty() {
        return 1.0;
    }

    let long_count = signals.iter().filter(|(_, conf)| *conf > 0.0).count();
    let ratio = long_count as f64 / signals.len() as f64;

    if ratio > 0.85 || ratio < 0.15 {
        0.7 // 30% penalty — extreme directional concentration
    } else if ratio > 0.75 || ratio < 0.25 {
        0.85 // 15% penalty — high concentration
    } else {
        1.0 // healthy diversification
    }
}

/// Compute pairwise average correlation for a set of return series.
/// Returns the average absolute correlation across all pairs.
pub fn avg_pairwise_correlation(return_series: &[Vec<f64>], window: usize) -> f64 {
    let n = return_series.len();
    if n < 2 {
        return 0.0;
    }

    let min_len = return_series.iter().map(|s| s.len()).min().unwrap_or(0);
    if min_len < window.max(3) {
        return 0.0;
    }

    let mut sum_corr = 0.0;
    let mut count = 0;

    for i in 0..n {
        for j in (i + 1)..n {
            let corr = rolling_correlation(&return_series[i], &return_series[j], window);
            sum_corr += corr.abs();
            count += 1;
        }
    }

    if count == 0 {
        0.0
    } else {
        sum_corr / count as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perfect_positive_correlation() {
        let a = vec![
            0.01, 0.02, -0.01, 0.03, -0.02, 0.01, -0.01, 0.02, -0.03, 0.01,
        ];
        let b = a.clone(); // identical series
        let corr = rolling_correlation(&a, &b, 10);
        assert!(
            (corr - 1.0).abs() < 0.001,
            "Perfect correlation should be 1.0, got {}",
            corr
        );
    }

    #[test]
    fn test_perfect_negative_correlation() {
        let a = vec![
            0.01, 0.02, -0.01, 0.03, -0.02, 0.01, -0.01, 0.02, -0.03, 0.01,
        ];
        let b: Vec<f64> = a.iter().map(|x| -x).collect();
        let corr = rolling_correlation(&a, &b, 10);
        assert!(
            (corr + 1.0).abs() < 0.001,
            "Perfect negative correlation should be -1.0, got {}",
            corr
        );
    }

    #[test]
    fn test_concentration_penalty_extreme_long() {
        let signals: Vec<(String, f64)> = (0..10).map(|i| (format!("SYM{}", i), 0.8)).collect();
        assert!(
            concentration_penalty(&signals) < 1.0,
            "All-long should be penalized"
        );
    }

    #[test]
    fn test_concentration_penalty_balanced() {
        let mut signals: Vec<(String, f64)> = Vec::new();
        for i in 0..5 {
            signals.push((format!("L{}", i), 0.8));
        }
        for i in 0..5 {
            signals.push((format!("S{}", i), -0.6));
        }
        assert_eq!(
            concentration_penalty(&signals),
            1.0,
            "50/50 should not be penalized"
        );
    }
}
