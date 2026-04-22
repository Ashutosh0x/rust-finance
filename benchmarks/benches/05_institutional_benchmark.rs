// benches/05_institutional_benchmark.rs
//
// Citadel / Jane Street / JP Morgan 2026 benchmark suite
// Tests RustForge against institutional performance standards
//
// References:
//   - JP Morgan LTCMA 2026: 6.4% annual return baseline
//   - arXiv 2509.16707: AI trading Sharpe > 2.5
//   - arXiv 2602.00080: GT-Score generalization ratio > 0.183
//   - Citadel GQS: sub-μs order routing
//   - HFTPerformance: nanosecond tick pipeline

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// ═══════════════════════════════════════════════════════════════════════════════
// LAYER 4: LATENCY BENCHMARKS (Citadel/HFTPerformance Standard)
// ═══════════════════════════════════════════════════════════════════════════════

/// Simplified GARCH(1,1) variance update — cache-line aligned
#[repr(C, align(64))]
struct GarchState {
    omega: f64,
    alpha: f64,
    beta: f64,
    current_variance: f64,
    last_return: f64,
    annualised_vol: f64,
    _pad: [u8; 16],
}

impl GarchState {
    fn new() -> Self {
        Self {
            omega: 0.000001,
            alpha: 0.10,
            beta: 0.85,
            current_variance: 0.0001,
            last_return: 0.0,
            annualised_vol: 0.0,
            _pad: [0; 16],
        }
    }

    #[inline(always)]
    fn update(&mut self, new_return: f64) -> f64 {
        let eps_sq = self.last_return * self.last_return;
        self.current_variance =
            self.omega + self.alpha * eps_sq + self.beta * self.current_variance;
        self.last_return = new_return;
        self.annualised_vol = (self.current_variance * 252.0).sqrt();
        self.annualised_vol
    }
}

/// Branchless safety gate — deterministic risk check
#[inline(always)]
fn branchless_safety_gate(
    agreement: f64,
    drawdown: f64,
    vol_ratio: f64,
    concentration: f64,
) -> bool {
    let agree_ok = (agreement <= 0.85) as u8;
    let dd_ok = (drawdown <= 0.05) as u8;
    let vol_ok = (vol_ratio <= 2.5) as u8;
    let conc_ok = (concentration <= 0.20) as u8;
    (agree_ok & dd_ok & vol_ok & conc_ok) != 0
}

/// Swarm agent decision (simplified single-agent tick)
#[repr(C, align(64))]
struct SwarmAgent {
    position_usd: f64,
    cash: f64,
    bias: f64,
    threshold: f64,
}

impl SwarmAgent {
    fn new() -> Self {
        Self {
            position_usd: 0.0,
            cash: 5000.0,
            bias: 0.0,
            threshold: 0.5,
        }
    }

    #[inline(always)]
    fn decide(&mut self, signal: f64, price: f64) -> i8 {
        let adjusted = signal + self.bias;
        if adjusted > self.threshold {
            let buy_amt = (self.cash * 0.1).min(1000.0);
            self.cash -= buy_amt;
            self.position_usd += buy_amt / price;
            1 // buy
        } else if adjusted < -self.threshold {
            let sell_amt = self.position_usd * 0.1;
            self.position_usd -= sell_amt;
            self.cash += sell_amt * price;
            -1 // sell
        } else {
            0 // hold
        }
    }
}

/// Full tick → GARCH → safetygate pipeline
#[inline(always)]
fn full_institutional_pipeline(
    garch: &mut GarchState,
    new_return: f64,
    agreement: f64,
    drawdown: f64,
    concentration: f64,
) -> (f64, bool) {
    let vol = garch.update(new_return);
    let vol_ratio = vol / 0.15; // vs average vol
    let safe = branchless_safety_gate(agreement, drawdown, vol_ratio, concentration);
    (vol, safe)
}

// ═══════════════════════════════════════════════════════════════════════════════
// CRITERION BENCHMARKS
// ═══════════════════════════════════════════════════════════════════════════════

fn bench_garch_update_institutional(c: &mut Criterion) {
    let mut group = c.benchmark_group("institutional_latency");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(10_000);

    let mut garch = GarchState::new();
    group.bench_function("garch_update_ns", |b| {
        b.iter(|| black_box(garch.update(black_box(0.001))));
    });

    group.finish();
}

fn bench_safety_gate_branchless(c: &mut Criterion) {
    let mut group = c.benchmark_group("institutional_latency");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(10_000);

    // Normal conditions → PASS
    group.bench_function("safety_gate_pass", |b| {
        b.iter(|| {
            black_box(branchless_safety_gate(
                black_box(0.60),
                black_box(0.02),
                black_box(1.2),
                black_box(0.10),
            ))
        });
    });

    // Bias conditions → BLOCK
    group.bench_function("safety_gate_block", |b| {
        b.iter(|| {
            black_box(branchless_safety_gate(
                black_box(0.90),
                black_box(0.02),
                black_box(1.2),
                black_box(0.10),
            ))
        });
    });

    group.finish();
}

fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("institutional_latency");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(5_000);

    let mut garch = GarchState::new();
    group.bench_function("full_pipeline_tick_to_gate", |b| {
        b.iter(|| {
            black_box(full_institutional_pipeline(
                &mut garch,
                black_box(0.001),
                black_box(0.60),
                black_box(0.02),
                black_box(0.10),
            ))
        });
    });

    group.finish();
}

fn bench_swarm_agent_decision(c: &mut Criterion) {
    let mut group = c.benchmark_group("institutional_latency");
    group.measurement_time(Duration::from_secs(5));

    // Single agent decision
    let mut agent = SwarmAgent::new();
    group.bench_function("single_agent_decision", |b| {
        b.iter(|| black_box(agent.decide(black_box(0.6), black_box(175.50))));
    });

    // Batch: 1K agents (simulating the hot path)
    group.bench_function("batch_1k_agents", |b| {
        let mut agents: Vec<SwarmAgent> = (0..1_000).map(|_| SwarmAgent::new()).collect();
        b.iter(|| {
            let mut buys = 0i32;
            for agent in agents.iter_mut() {
                buys += black_box(agent.decide(black_box(0.6), black_box(175.50))) as i32;
            }
            black_box(buys)
        });
    });

    group.finish();
}

fn bench_kill_switch_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("institutional_latency");
    group.measurement_time(Duration::from_secs(3));

    let kill = AtomicBool::new(false);
    group.bench_function("kill_switch_check_ns", |b| {
        b.iter(|| black_box(kill.load(Ordering::Relaxed)));
    });

    group.finish();
}

fn bench_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("swarm_scaling");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(100);

    for agent_count in [100, 1_000, 10_000, 100_000].iter() {
        group.bench_with_input(
            BenchmarkId::new("agent_batch", agent_count),
            agent_count,
            |b, &n| {
                let mut agents: Vec<SwarmAgent> = (0..n).map(|_| SwarmAgent::new()).collect();
                b.iter(|| {
                    let mut net = 0i32;
                    for agent in agents.iter_mut() {
                        net += agent.decide(black_box(0.55), black_box(175.50)) as i32;
                    }
                    black_box(net)
                });
            },
        );
    }
    group.finish();
}

criterion_group! {
    name = institutional_benches;
    config = Criterion::default();
    targets = bench_garch_update_institutional,
              bench_safety_gate_branchless,
              bench_full_pipeline,
              bench_swarm_agent_decision,
              bench_kill_switch_overhead,
              bench_scaling
}
criterion_main!(institutional_benches);
