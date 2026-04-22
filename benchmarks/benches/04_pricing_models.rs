// benches/04_pricing_models.rs
// Benchmarks educational Black-Scholes-Merton and Heston approximations.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Duration;

// Minimal simulated components of BSM and Heston logic.
// In actual testing, these would link to the `pricing` crate directly.
// For demonstration, we benchmark the standard normal CDF and BSM formulas natively.

use std::f64::consts::PI;

#[inline]
fn approx_norm_cdf(x: f64) -> f64 {
    let b1 = 0.319381530;
    let b2 = -0.356563782;
    let b3 = 1.781477937;
    let b4 = -1.821255978;
    let b5 = 1.330274429;
    let p = 0.2316419;
    let c = 1.0 / (2.0 * PI).sqrt();

    let a = x.abs();
    let t = 1.0 / (1.0 + a * p);
    let b = c * (-x * x / 2.0).exp();
    let n = ((((b5 * t + b4) * t + b3) * t + b2) * t + b1) * t;
    let n = 1.0 - b * n;

    if x < 0.0 {
        1.0 - n
    } else {
        n
    }
}

#[inline]
fn black_scholes_call(s: f64, k: f64, t: f64, r: f64, vol: f64) -> f64 {
    let d1 = ((s / k).ln() + (r + vol * vol / 2.0) * t) / (vol * t.sqrt());
    let d2 = d1 - vol * t.sqrt();
    s * approx_norm_cdf(d1) - k * (-r * t).exp() * approx_norm_cdf(d2)
}

fn bench_black_scholes(c: &mut Criterion) {
    let s = 150.0;
    let k = 155.0;
    let t = 30.0 / 365.0;
    let r = 0.05;
    let vol = 0.25;

    let mut group = c.benchmark_group("pricing_models");
    group.bench_function("bsm_european_call", |b| {
        b.iter(|| {
            black_box(black_scholes_call(
                black_box(s),
                black_box(k),
                black_box(t),
                black_box(r),
                black_box(vol),
            ))
        })
    });
    group.finish();
}

criterion_group! {
    name = pricing_benches;
    config = Criterion::default().measurement_time(Duration::from_secs(3));
    targets = bench_black_scholes
}
criterion_main!(pricing_benches);
