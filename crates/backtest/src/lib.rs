#![forbid(unsafe_code)]
// crates/backtest/src/lib.rs
//
// Root module for backtesting logic.

pub mod benchmark;
pub mod engine;
pub mod robustness;
pub mod strategy;

pub use benchmark::{
    compute_institutional_metrics, print_benchmark_report, validate_against_institutions,
    validate_swarm_stylized_facts, walk_forward_backtest, BenchmarkGrade, BenchmarkReport,
    BenchmarkThresholds, InstitutionalMetrics, SwarmValidationResult,
};
pub use engine::{BacktestConfig, BacktestEngine, BacktestMetrics, Bar};
pub use robustness::{
    capacity_degradation, cost_sensitivity_matrix, engine_consistency_check, print_capacity_report,
    print_cost_sensitivity, print_overfit_report, print_reproducibility_proof,
    probability_of_backtest_overfitting, run_full_audit, verify_reproducibility, CapacityReport,
    CostSensitivityReport, EngineConsistencyResult, FullAuditReport, OverfitReport,
    ReproducibilityProof,
};
pub use strategy::{SimpleMovingAverageCrossover, Strategy, StrategySignal, ZScoreMeanReversion};
