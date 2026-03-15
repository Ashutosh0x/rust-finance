// benches/03_risk_checks.rs
// Measures atomic kill switch, branchless limits, and GARCH volatility overhead

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct KillSwitch { active: AtomicBool }

impl KillSwitch {
    pub fn new() -> Self { Self { active: AtomicBool::new(false) } }
    #[inline(always)] pub fn is_active(&self) -> bool { self.active.load(Ordering::Relaxed) }
}

#[derive(Clone)]
pub struct RiskLimits {
    pub max_order_qty: f64,
    pub max_notional: f64,
    pub max_position: f64,
    pub daily_turnover_remaining: f64,
}

#[inline(always)]
pub fn branchless_risk_check(qty: f64, price: f64, limits: &RiskLimits) -> bool {
    let notional = qty * price;
    let qty_ok = (qty <= limits.max_order_qty) as u8;
    let notional_ok = (notional <= limits.max_notional) as u8;
    let turnover_ok = (notional <= limits.daily_turnover_remaining) as u8;
    (qty_ok & notional_ok & turnover_ok) != 0
}

#[repr(C, align(64))]
pub struct GarchState {
    pub omega: f64, pub alpha: f64, pub beta: f64,
    pub current_variance: f64, pub last_return: f64, pub annualised_vol: f64,
    _pad: [u8; 16],
}
impl GarchState {
    pub fn new(omega: f64, alpha: f64, beta: f64) -> Self {
        Self { omega, alpha, beta, current_variance: 0.0001, last_return: 0.0, annualised_vol: 0.0, _pad: [0; 16] }
    }
    #[inline(always)]
    pub fn update(&mut self, new_return: f64) -> f64 {
        let epsilon_sq = self.last_return * self.last_return;
        self.current_variance = self.omega + self.alpha * epsilon_sq + self.beta * self.current_variance;
        self.last_return = new_return;
        self.annualised_vol = (self.current_variance * 252.0).sqrt();
        self.annualised_vol
    }
}

fn bench_kill_switch_atomic(c: &mut Criterion) {
    let ks = KillSwitch::new();
    c.bench_function("kill_switch_atomic_read", |b| {
        b.iter(|| black_box(ks.is_active()));
    });
}

fn bench_branchless_pre_trade(c: &mut Criterion) {
    let limits = RiskLimits { max_order_qty: 10_000.0, max_notional: 5_000_000.0, max_position: 50_000.0, daily_turnover_remaining: 100_000_000.0 };
    c.bench_function("branchless_risk_check", |b| {
        b.iter(|| black_box(branchless_risk_check(black_box(100.0), black_box(175.5), black_box(&limits))));
    });
}

fn bench_garch_update(c: &mut Criterion) {
    let mut garch = GarchState::new(0.000001, 0.10, 0.85);
    c.bench_function("garch_variance_update", |b| {
        b.iter(|| black_box(garch.update(black_box(0.001))));
    });
}

criterion_group! {
    name = risk_benches;
    config = Criterion::default().measurement_time(Duration::from_secs(5));
    targets = bench_kill_switch_atomic, bench_branchless_pre_trade, bench_garch_update
}
criterion_main!(risk_benches);
