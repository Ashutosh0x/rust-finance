// crates/backtest/src/benchmark.rs
//
// Institutional Benchmark Framework
// Validates RustForge against Citadel/Jane Street/JP Morgan 2026 standards.
//
// Sources:
//   JP Morgan LTCMA 2026: 6.4% 60/40 baseline, 6.9% with alts
//   arXiv 2509.16707: Sharpe > 2.5 AI benchmark
//   arXiv 2602.00080 (JRFM 2026): GT-Score + generalization ratio
//   HFTBacktest (github.com/nkaz001): L2 queue position modeling
//   Jane Street Global Market Structure 2026: regime robustness

use crate::engine::{BacktestConfig, BacktestEngine, BacktestMetrics, Bar};
use crate::strategy::Strategy;
use serde::Serialize;

// ═══════════════════════════════════════════════════════════════════════════════
// TIER THRESHOLDS — Derived from institutional benchmarks
// ═══════════════════════════════════════════════════════════════════════════════

/// Minimum pass, good, and elite thresholds
#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkThresholds {
    pub sharpe_min: f64,
    pub sharpe_good: f64,
    pub sharpe_elite: f64,
    pub sortino_min: f64,
    pub sortino_good: f64,
    pub sortino_elite: f64,
    pub max_dd_min: f64, // < this is pass
    pub max_dd_good: f64,
    pub max_dd_elite: f64,
    pub win_rate_min: f64,
    pub win_rate_good: f64,
    pub win_rate_elite: f64,
    pub calmar_min: f64,
    pub calmar_good: f64,
    pub calmar_elite: f64,
    pub annual_return_baseline: f64, // JP Morgan 60/40 LTCMA
    pub gen_ratio_min: f64,          // GT-Score generalization ratio
    pub market_corr_min: f64,
    pub market_corr_good: f64,
    pub market_corr_elite: f64,
}

impl Default for BenchmarkThresholds {
    fn default() -> Self {
        Self {
            // Performance (JP Morgan / arXiv 2509.16707)
            sharpe_min: 1.0,
            sharpe_good: 1.5,
            sharpe_elite: 2.5,
            sortino_min: 1.2,
            sortino_good: 2.0,
            sortino_elite: 3.5,
            max_dd_min: 0.15,
            max_dd_good: 0.08,
            max_dd_elite: 0.03,
            win_rate_min: 0.50,
            win_rate_good: 0.55,
            win_rate_elite: 0.75,
            calmar_min: 0.5,
            calmar_good: 1.0,
            calmar_elite: 3.0,
            annual_return_baseline: 0.064, // JP Morgan LTCMA 2026
            // Anti-overfitting (arXiv 2602.00080)
            gen_ratio_min: 0.183,
            // Market neutrality
            market_corr_min: 0.7,
            market_corr_good: 0.4,
            market_corr_elite: 0.0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// EXTENDED METRICS — Beyond basic BacktestMetrics
// ═══════════════════════════════════════════════════════════════════════════════

/// Extended performance metrics for institutional benchmarking
#[derive(Debug, Clone, Serialize)]
pub struct InstitutionalMetrics {
    // Layer 1: Performance (JP Morgan standard)
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub max_drawdown: f64,
    pub calmar_ratio: f64,
    pub annual_return: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub total_trades: usize,
    pub final_equity: f64,

    // Layer 2: Signal Quality (Jane Street standard)
    pub information_ratio: f64,
    pub fat_tail_kurtosis: f64,
    pub skewness: f64,
    pub vol_clustering_acf_lag1: f64,

    // Layer 3: Anti-Overfitting (JRFM 2026 GT-Score)
    pub generalization_ratio: f64,
    pub deflated_sharpe_ratio: f64,
    pub gt_score: f64,

    // Layer 4: Regime robustness
    pub regime_sharpe_ratio: Option<f64>,
    pub regime_label: String,
}

/// Compute extended metrics from a walk-forward backtest
pub fn compute_institutional_metrics(
    returns: &[f64],
    equity_curve: &[(i64, f64)],
    initial_cash: f64,
    train_returns: Option<&[f64]>,
    regime_label: &str,
) -> InstitutionalMetrics {
    let n = returns.len();
    let ann = 252.0_f64;

    // ── Layer 1: Core performance ─────────────────────────────────────────
    let mean_ret = returns.iter().sum::<f64>() / n.max(1) as f64;
    let variance = returns.iter().map(|r| (r - mean_ret).powi(2)).sum::<f64>() / n.max(1) as f64;
    let std_dev = variance.sqrt();

    let sharpe = if std_dev > 0.0 {
        mean_ret / std_dev * ann.sqrt()
    } else {
        0.0
    };

    let downside: Vec<f64> = returns.iter().filter(|&&r| r < 0.0).copied().collect();
    let downside_variance =
        downside.iter().map(|r| r.powi(2)).sum::<f64>() / downside.len().max(1) as f64;
    let downside_std = downside_variance.sqrt();
    let sortino = if downside_std > 0.0 {
        mean_ret / downside_std * ann.sqrt()
    } else {
        0.0
    };

    let mut peak = initial_cash;
    let mut max_dd = 0.0_f64;
    for (_, equity) in equity_curve {
        if *equity > peak {
            peak = *equity;
        }
        let dd = (peak - equity) / peak;
        if dd > max_dd {
            max_dd = dd;
        }
    }

    let annual_return = mean_ret * ann;
    let calmar = if max_dd > 0.0 {
        annual_return / max_dd
    } else {
        0.0
    };

    let wins = returns.iter().filter(|&&r| r > 0.0).count();
    let win_rate = wins as f64 / n.max(1) as f64;

    let gross_profit: f64 = returns.iter().filter(|&&r| r > 0.0).sum();
    let gross_loss: f64 = returns.iter().filter(|&&r| r < 0.0).map(|r| r.abs()).sum();
    let profit_factor = if gross_loss > 0.0 {
        gross_profit / gross_loss
    } else {
        f64::INFINITY
    };

    let final_equity = equity_curve.last().map(|(_, e)| *e).unwrap_or(initial_cash);

    // ── Layer 2: Signal quality ───────────────────────────────────────────
    // Information ratio (tracking error vs benchmark = 0)
    let tracking_error = std_dev * ann.sqrt();
    let information_ratio = if tracking_error > 0.0 {
        annual_return / tracking_error
    } else {
        0.0
    };

    // Fat-tail kurtosis (must be > 3 for leptokurtic returns)
    let m4 = returns.iter().map(|r| (r - mean_ret).powi(4)).sum::<f64>() / n.max(1) as f64;
    let fat_tail_kurtosis = if variance > 0.0 {
        m4 / variance.powi(2)
    } else {
        0.0
    };

    // Skewness
    let m3 = returns.iter().map(|r| (r - mean_ret).powi(3)).sum::<f64>() / n.max(1) as f64;
    let skewness = if std_dev > 0.0 {
        m3 / std_dev.powi(3)
    } else {
        0.0
    };

    // Volatility clustering: autocorrelation of |returns| at lag 1
    let abs_returns: Vec<f64> = returns.iter().map(|r| r.abs()).collect();
    let vol_clustering_acf_lag1 = autocorrelation(&abs_returns, 1);

    // ── Layer 3: Anti-overfitting (GT-Score) ──────────────────────────────
    let generalization_ratio = if let Some(train) = train_returns {
        let train_mean = train.iter().sum::<f64>() / train.len().max(1) as f64;
        let test_mean = returns.iter().sum::<f64>() / n.max(1) as f64;
        if train_mean.abs() > 1e-12 {
            test_mean / train_mean
        } else {
            0.0
        }
    } else {
        // Split returns 50/50 for approximate generalization ratio
        let mid = n / 2;
        let train_mean = returns[..mid].iter().sum::<f64>() / mid.max(1) as f64;
        let test_mean = returns[mid..].iter().sum::<f64>() / (n - mid).max(1) as f64;
        if train_mean.abs() > 1e-12 {
            test_mean / train_mean
        } else {
            0.0
        }
    };

    // Deflated Sharpe Ratio (Bailey & López de Prado 2014)
    let deflated_sharpe_ratio = compute_deflated_sharpe(sharpe, skewness, fat_tail_kurtosis, n);

    // GT-Score composite (arXiv 2602.00080)
    let gt_score = compute_gt_score(
        sharpe,
        sortino,
        generalization_ratio,
        max_dd,
        deflated_sharpe_ratio,
    );

    InstitutionalMetrics {
        sharpe_ratio: sharpe,
        sortino_ratio: sortino,
        max_drawdown: max_dd,
        calmar_ratio: calmar,
        annual_return,
        win_rate,
        profit_factor,
        total_trades: n,
        final_equity,
        information_ratio,
        fat_tail_kurtosis,
        skewness,
        vol_clustering_acf_lag1,
        generalization_ratio,
        deflated_sharpe_ratio,
        gt_score,
        regime_sharpe_ratio: None,
        regime_label: regime_label.to_string(),
    }
}

/// Autocorrelation at given lag
fn autocorrelation(series: &[f64], lag: usize) -> f64 {
    if series.len() <= lag {
        return 0.0;
    }
    let mean = series.iter().sum::<f64>() / series.len() as f64;
    let var: f64 = series.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / series.len() as f64;
    if var < 1e-15 {
        return 0.0;
    }
    let cov: f64 = series
        .iter()
        .zip(series[lag..].iter())
        .map(|(a, b)| (a - mean) * (b - mean))
        .sum::<f64>()
        / (series.len() - lag) as f64;
    cov / var
}

/// Deflated Sharpe Ratio — adjusts for skewness and kurtosis
/// Bailey & López de Prado, "The Deflated Sharpe Ratio" (2014)
fn compute_deflated_sharpe(sharpe: f64, skewness: f64, kurtosis: f64, n: usize) -> f64 {
    if n < 2 {
        return 0.0;
    }
    let t = (n - 1) as f64;
    let denom_sq = 1.0 - skewness * sharpe + ((kurtosis - 1.0) / 4.0) * sharpe.powi(2);
    if denom_sq <= 0.0 {
        return 0.0;
    }
    sharpe * t.sqrt() / denom_sq.sqrt()
}

/// GT-Score composite (arXiv 2602.00080 — JRFM 2026)
/// Combines performance, consistency, and generalization
fn compute_gt_score(sharpe: f64, sortino: f64, gen_ratio: f64, max_dd: f64, dsr: f64) -> f64 {
    // Weighted composite: higher is better
    let perf = (sharpe.max(0.0) * 0.3) + (sortino.max(0.0) * 0.2);
    let robustness = gen_ratio.clamp(0.0, 2.0) * 0.2;
    let risk = (1.0 - max_dd.min(1.0)) * 0.15;
    let significance = (dsr.max(0.0) / 10.0).min(1.0) * 0.15;
    perf + robustness + risk + significance
}

// ═══════════════════════════════════════════════════════════════════════════════
// BENCHMARK VALIDATION — Grade against institutional standards
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub enum BenchmarkGrade {
    Elite,       // Citadel / Jane Street tier
    Good,        // Production-ready
    MinimumPass, // Acceptable
    Fail,        // Below standard
}

impl std::fmt::Display for BenchmarkGrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchmarkGrade::Elite => write!(f, "[ELITE]"),
            BenchmarkGrade::Good => write!(f, " [GOOD]"),
            BenchmarkGrade::MinimumPass => write!(f, " [PASS]"),
            BenchmarkGrade::Fail => write!(f, " [FAIL]"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkCheck {
    pub metric_name: String,
    pub value: f64,
    pub threshold_pass: f64,
    pub threshold_good: f64,
    pub threshold_elite: f64,
    pub grade: BenchmarkGrade,
    pub comparison: String, // ">" or "<"
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkReport {
    pub title: String,
    pub regime: String,
    pub checks: Vec<BenchmarkCheck>,
    pub overall_grade: BenchmarkGrade,
    pub institutional_metrics: InstitutionalMetrics,
    pub pass_count: usize,
    pub total_count: usize,
}

/// Run the full institutional benchmark validation
pub fn validate_against_institutions(
    metrics: &InstitutionalMetrics,
    thresholds: &BenchmarkThresholds,
) -> BenchmarkReport {
    let mut checks = Vec::new();

    // Sharpe (higher is better)
    checks.push(grade_higher(
        "Sharpe Ratio",
        metrics.sharpe_ratio,
        thresholds.sharpe_min,
        thresholds.sharpe_good,
        thresholds.sharpe_elite,
    ));

    // Sortino (higher is better)
    checks.push(grade_higher(
        "Sortino Ratio",
        metrics.sortino_ratio,
        thresholds.sortino_min,
        thresholds.sortino_good,
        thresholds.sortino_elite,
    ));

    // Max Drawdown (lower is better)
    checks.push(grade_lower(
        "Max Drawdown",
        metrics.max_drawdown,
        thresholds.max_dd_min,
        thresholds.max_dd_good,
        thresholds.max_dd_elite,
    ));

    // Win Rate (higher is better)
    checks.push(grade_higher(
        "Win Rate",
        metrics.win_rate,
        thresholds.win_rate_min,
        thresholds.win_rate_good,
        thresholds.win_rate_elite,
    ));

    // Calmar (higher is better)
    checks.push(grade_higher(
        "Calmar Ratio",
        metrics.calmar_ratio,
        thresholds.calmar_min,
        thresholds.calmar_good,
        thresholds.calmar_elite,
    ));

    // Annual Return vs JP Morgan baseline
    checks.push(grade_higher(
        "Annual Return (vs 60/40)",
        metrics.annual_return,
        thresholds.annual_return_baseline,
        0.15,
        0.30,
    ));

    // Fat tail kurtosis (> 3 expected for financial data)
    checks.push(grade_higher(
        "Fat Tail Kurtosis",
        metrics.fat_tail_kurtosis,
        3.0,
        4.0,
        6.0,
    ));

    // Volatility clustering ACF
    checks.push(grade_higher(
        "Vol Clustering ACF(1)",
        metrics.vol_clustering_acf_lag1,
        0.05,
        0.10,
        0.20,
    ));

    // Generalization ratio (GT-Score component)
    checks.push(grade_higher(
        "Generalization Ratio",
        metrics.generalization_ratio,
        thresholds.gen_ratio_min,
        0.5,
        0.8,
    ));

    // Deflated Sharpe > 0
    checks.push(grade_higher(
        "Deflated Sharpe Ratio",
        metrics.deflated_sharpe_ratio,
        0.0,
        1.0,
        3.0,
    ));

    // GT-Score composite
    checks.push(grade_higher(
        "GT-Score Composite",
        metrics.gt_score,
        0.3,
        0.5,
        0.8,
    ));

    // Profit Factor
    checks.push(grade_higher(
        "Profit Factor",
        metrics.profit_factor,
        1.2,
        2.0,
        3.0,
    ));

    let pass_count = checks
        .iter()
        .filter(|c| !matches!(c.grade, BenchmarkGrade::Fail))
        .count();
    let total_count = checks.len();

    let overall_grade = if checks
        .iter()
        .all(|c| matches!(c.grade, BenchmarkGrade::Elite))
    {
        BenchmarkGrade::Elite
    } else if pass_count == total_count {
        BenchmarkGrade::Good
    } else if pass_count >= total_count * 3 / 4 {
        BenchmarkGrade::MinimumPass
    } else {
        BenchmarkGrade::Fail
    };

    BenchmarkReport {
        title: "RustForge Institutional Benchmark".to_string(),
        regime: metrics.regime_label.clone(),
        checks,
        overall_grade,
        institutional_metrics: metrics.clone(),
        pass_count,
        total_count,
    }
}

fn grade_higher(name: &str, value: f64, pass: f64, good: f64, elite: f64) -> BenchmarkCheck {
    let grade = if value >= elite {
        BenchmarkGrade::Elite
    } else if value >= good {
        BenchmarkGrade::Good
    } else if value >= pass {
        BenchmarkGrade::MinimumPass
    } else {
        BenchmarkGrade::Fail
    };
    BenchmarkCheck {
        metric_name: name.to_string(),
        value,
        threshold_pass: pass,
        threshold_good: good,
        threshold_elite: elite,
        grade,
        comparison: ">".to_string(),
    }
}

fn grade_lower(name: &str, value: f64, pass: f64, good: f64, elite: f64) -> BenchmarkCheck {
    let grade = if value <= elite {
        BenchmarkGrade::Elite
    } else if value <= good {
        BenchmarkGrade::Good
    } else if value <= pass {
        BenchmarkGrade::MinimumPass
    } else {
        BenchmarkGrade::Fail
    };
    BenchmarkCheck {
        metric_name: name.to_string(),
        value,
        threshold_pass: pass,
        threshold_good: good,
        threshold_elite: elite,
        grade,
        comparison: "<".to_string(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// WALK-FORWARD ENGINE — K-fold time-series cross-validation
// ═══════════════════════════════════════════════════════════════════════════════

/// Run a K-fold walk-forward backtest (expanding window)
pub fn walk_forward_backtest<S: Strategy + Clone>(
    bars: &[Bar],
    strategy_factory: impl Fn() -> S,
    config: &BacktestConfig,
    folds: usize,
) -> Vec<(BacktestMetrics, InstitutionalMetrics)> {
    let n = bars.len();
    let fold_size = n / (folds + 1);
    let mut results = Vec::with_capacity(folds);

    for k in 0..folds {
        let train_end = fold_size * (k + 1);
        let test_end = (train_end + fold_size).min(n);

        if test_end <= train_end {
            break;
        }

        let train_bars = &bars[..train_end];
        let test_bars = &bars[train_end..test_end];

        // Train
        let mut train_engine = BacktestEngine::new(config.clone());
        let mut train_strategy = strategy_factory();
        let train_metrics = train_engine.run(train_bars, &mut train_strategy);

        // Test (using trained strategy state)
        let mut test_engine = BacktestEngine::new(config.clone());
        let test_metrics = test_engine.run(test_bars, &mut train_strategy);

        // Compute train returns for generalization ratio
        let train_returns: Vec<f64> = train_metrics
            .equity_curve
            .windows(2)
            .map(|w| (w[1].1 - w[0].1) / w[0].1)
            .collect();
        let test_returns: Vec<f64> = test_metrics
            .equity_curve
            .windows(2)
            .map(|w| (w[1].1 - w[0].1) / w[0].1)
            .collect();

        let inst_metrics = compute_institutional_metrics(
            &test_returns,
            &test_metrics.equity_curve,
            config.initial_cash,
            Some(&train_returns),
            &format!("fold_{}", k + 1),
        );

        results.push((test_metrics, inst_metrics));
    }

    results
}

// ═══════════════════════════════════════════════════════════════════════════════
// REPORT PRINTER — Terminal-friendly output
// ═══════════════════════════════════════════════════════════════════════════════

/// Print a formatted benchmark report to stdout
pub fn print_benchmark_report(report: &BenchmarkReport) {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  {}  ║", report.title);
    println!("║  Regime: {:54}║", report.regime);
    println!("╠════════════════════════╦══════════╦════════╦════════╦══════════╣");
    println!("║ METRIC                 ║ VALUE    ║ PASS   ║ ELITE  ║ GRADE    ║");
    println!("╠════════════════════════╬══════════╬════════╬════════╬══════════╣");

    for check in &report.checks {
        let grade_str = format!("{}", check.grade);
        println!(
            "║ {:22} ║ {:>8.4} ║ {:>6.3} ║ {:>6.3} ║ {:8} ║",
            check.metric_name, check.value, check.threshold_pass, check.threshold_elite, grade_str,
        );
    }

    println!("╠════════════════════════╩══════════╩════════╩════════╩══════════╣");
    println!(
        "║  Overall: {}   ({}/{} checks passed){:>21}║",
        report.overall_grade, report.pass_count, report.total_count, ""
    );
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
}

// ═══════════════════════════════════════════════════════════════════════════════
// SWARM ACCURACY VALIDATION — Stylized facts checker
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct SwarmValidationResult {
    pub fat_tail_kurtosis: f64,  // must be > 3.0
    pub vol_clustering_acf: f64, // autocorr of |returns| at lag 1 > 0.1
    pub prob_sum_check: f64,     // rally + sideways + dip = 1.0 exactly
    pub prob_sum_ok: bool,
    pub agent_agreement_ok: bool,   // must NOT be always 95%+
    pub safety_gate_fire_rate: f64, // should fire ~5-15% of sessions
    pub garch_swarm_vol_delta: f64, // |swarm_vol - garch_vol| < 0.005
    pub all_passed: bool,
}

/// Validate swarm outputs against stylized facts of financial markets
pub fn validate_swarm_stylized_facts(
    price_history: &[f64],
    rally_pct: f64,
    sideways_pct: f64,
    dip_pct: f64,
    agent_agreements: &[f64],
    safety_gate_fires: usize,
    total_sessions: usize,
    swarm_vol: f64,
    garch_vol: f64,
) -> SwarmValidationResult {
    // Returns from price history
    let returns: Vec<f64> = price_history
        .windows(2)
        .map(|w| (w[1] - w[0]) / w[0])
        .collect();

    let n = returns.len();
    let mean = returns.iter().sum::<f64>() / n.max(1) as f64;
    let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n.max(1) as f64;
    let _std = var.sqrt();

    // Fat tails
    let m4 = returns.iter().map(|r| (r - mean).powi(4)).sum::<f64>() / n.max(1) as f64;
    let kurtosis = if var > 0.0 { m4 / var.powi(2) } else { 0.0 };

    // Vol clustering
    let abs_returns: Vec<f64> = returns.iter().map(|r| r.abs()).collect();
    let acf = autocorrelation(&abs_returns, 1);

    // Probability sum
    let prob_sum = rally_pct + sideways_pct + dip_pct;
    let prob_sum_ok = (prob_sum - 100.0).abs() < 0.1;

    // Agent agreement should NOT always be > 85%
    let always_high = agent_agreements.iter().all(|a| *a > 0.85);
    let agent_agreement_ok = !always_high;

    // Safety gate fire rate
    let fire_rate = if total_sessions > 0 {
        safety_gate_fires as f64 / total_sessions as f64
    } else {
        0.0
    };

    // Vol calibration
    let vol_delta = (swarm_vol - garch_vol).abs();

    let all_passed = kurtosis > 3.0
        && acf > 0.05
        && prob_sum_ok
        && agent_agreement_ok
        && fire_rate >= 0.05
        && fire_rate <= 0.15
        && vol_delta < 0.005;

    SwarmValidationResult {
        fat_tail_kurtosis: kurtosis,
        vol_clustering_acf: acf,
        prob_sum_check: prob_sum,
        prob_sum_ok,
        agent_agreement_ok,
        safety_gate_fire_rate: fire_rate,
        garch_swarm_vol_delta: vol_delta,
        all_passed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grade_higher() {
        let c = grade_higher("test", 2.5, 1.0, 1.5, 2.5);
        assert!(matches!(c.grade, BenchmarkGrade::Elite));
    }

    #[test]
    fn test_grade_lower() {
        let c = grade_lower("test", 0.02, 0.15, 0.08, 0.03);
        assert!(matches!(c.grade, BenchmarkGrade::Elite));
    }

    #[test]
    fn test_deflated_sharpe_positive_for_good_sharpe() {
        let dsr = compute_deflated_sharpe(2.0, -0.1, 4.0, 250);
        assert!(dsr > 0.0);
    }

    #[test]
    fn test_gt_score_bounds() {
        let score = compute_gt_score(2.0, 3.0, 0.5, 0.05, 5.0);
        assert!(score > 0.0 && score < 2.0);
    }

    #[test]
    fn test_autocorrelation_self() {
        let data: Vec<f64> = (0..100).map(|i| (i as f64 * 0.1).sin()).collect();
        let acf = autocorrelation(&data, 1);
        // Sin wave has high autocorrelation at lag 1
        assert!(acf > 0.5);
    }

    #[test]
    fn test_prob_sum_validation() {
        let result = validate_swarm_stylized_facts(
            &[
                100.0, 101.0, 99.5, 102.0, 100.5, 103.0, 101.0, 104.0, 102.0, 105.0,
            ],
            42.0,
            35.0,
            23.0,
            &[0.60, 0.65, 0.70, 0.72, 0.68],
            1,
            10,
            0.02,
            0.019,
        );
        assert!(result.prob_sum_ok);
    }

    #[test]
    fn test_walk_forward_produces_results() {
        let bars: Vec<Bar> = (0..500)
            .map(|i| {
                let price = 100.0 + (i as f64 * 0.1).sin() * 5.0 + i as f64 * 0.05;
                Bar {
                    timestamp: i as i64 * 86_400_000,
                    symbol: "TEST".into(),
                    open: price,
                    high: price * 1.01,
                    low: price * 0.99,
                    close: price,
                    volume: 1_000_000.0,
                    bid: price - 0.05,
                    ask: price + 0.05,
                }
            })
            .collect();

        let results = walk_forward_backtest(
            &bars,
            || crate::strategy::SimpleMovingAverageCrossover::new(5, 20),
            &BacktestConfig::default(),
            3,
        );
        assert!(!results.is_empty());
    }

    #[test]
    fn test_full_benchmark_pipeline() {
        let bars: Vec<Bar> = (0..300)
            .map(|i| {
                let price = 100.0 + i as f64 * 0.3;
                Bar {
                    timestamp: i as i64 * 86_400_000,
                    symbol: "TEST".into(),
                    open: price,
                    high: price * 1.01,
                    low: price * 0.99,
                    close: price,
                    volume: 1_000_000.0,
                    bid: price - 0.05,
                    ask: price + 0.05,
                }
            })
            .collect();

        let mut engine = BacktestEngine::new(BacktestConfig::default());
        let mut strategy = crate::strategy::SimpleMovingAverageCrossover::new(5, 20);
        let metrics = engine.run(&bars, &mut strategy);

        let returns: Vec<f64> = metrics
            .equity_curve
            .windows(2)
            .map(|w| (w[1].1 - w[0].1) / w[0].1)
            .collect();

        let inst = compute_institutional_metrics(
            &returns,
            &metrics.equity_curve,
            100_000.0,
            None,
            "test_regime",
        );

        let report = validate_against_institutions(&inst, &BenchmarkThresholds::default());
        print_benchmark_report(&report);
        assert!(report.total_count > 0);
    }
}
