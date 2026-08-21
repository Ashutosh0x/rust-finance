//! Cost of the latency instrumentation itself.
//!
//! # Why this benchmark exists
//!
//! Every span recorded on the tick-to-trade path is work the feed handler does
//! *instead of* decoding the next message. Instrumentation that costs more than
//! the stage it measures does not report latency, it creates it — and it does so
//! invisibly, because the number it prints excludes its own overhead.
//!
//! So the bar these numbers have to clear is concrete: recording a span must be
//! small next to an ITCH decode, which `06_direct_exchange_feeds` measures in
//! tens of nanoseconds. If `record_span` ever approaches that, the histogram
//! belongs behind a feature flag rather than on by default.
//!
//! What is measured here:
//!
//!   - `record` / `record_span` — the hot path, once per stage per message
//!   - `now_monotonic_ns` — the clock read, which happens *more* often than a
//!     record, since a span needs two readings
//!   - `record_path` — all six stages of a tick-to-trade in one call
//!   - `quantile` / `summary` — the query path, which is off the hot path and
//!     is expected to be far slower; it walks 640 buckets
//!
//! Run with:
//! ```text
//! cargo bench -p benchmarks --bench 07_latency_recording
//! ```

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use exchange_core::latency::{now_monotonic_ns, LatencyHistogram, TickToTrade};

/// A spread of samples across several orders of magnitude.
///
/// A single repeated value would land in one bucket and measure a
/// best-case branch pattern that no real feed produces.
fn spread() -> Vec<u64> {
    let mut out = Vec::with_capacity(1024);
    let mut x: u64 = 12_345;
    for _ in 0..1024 {
        // xorshift: deterministic, and cheap enough not to dominate the loop.
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        // 50ns … ~5us, the range a feed-handler stage actually occupies.
        out.push(50 + (x % 5_000));
    }
    out
}

fn bench_record(c: &mut Criterion) {
    let mut group = c.benchmark_group("latency/record");
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements(1));

    let samples = spread();

    group.bench_function("record", |b| {
        let mut h = LatencyHistogram::new("bench");
        let mut i = 0usize;
        b.iter(|| {
            // Wrapping index rather than a fresh histogram per iteration: the
            // reset would be measured as part of the record.
            h.record(black_box(samples[i % samples.len()]));
            i += 1;
        });
        black_box(h.count());
    });

    group.bench_function("record_span", |b| {
        let mut h = LatencyHistogram::new("bench");
        let mut i = 0usize;
        b.iter(|| {
            let start = black_box(1_000_000u64);
            let end = start + samples[i % samples.len()];
            h.record_span(black_box(start), black_box(end));
            i += 1;
        });
        black_box(h.count());
    });

    group.bench_function("now_monotonic_ns", |b| {
        // Read twice per span, so this is the more frequent of the two costs.
        b.iter(|| black_box(now_monotonic_ns()));
    });

    group.finish();
}

fn bench_tick_to_trade(c: &mut Criterion) {
    let mut group = c.benchmark_group("latency/tick_to_trade");
    group.measurement_time(Duration::from_secs(5));
    // One call records six stages plus the end-to-end span.
    group.throughput(Throughput::Elements(1));

    group.bench_function("record_path", |b| {
        let mut t = TickToTrade::new();
        let mut base = 0u64;
        b.iter(|| {
            // Strictly increasing, so the path is accepted rather than dropped
            // by the monotonicity check — measuring the rejection path would
            // report a number the real system never pays.
            base = base.wrapping_add(10_000);
            t.record_path(
                black_box(base),
                base + 80,
                base + 130,
                base + 250,
                base + 300,
                base + 400,
            );
        });
        black_box(t.total.count());
    });

    group.finish();
}

fn bench_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("latency/query");
    group.measurement_time(Duration::from_secs(5));

    // Filled once: the query cost depends on how many buckets are occupied,
    // not on how the samples got there.
    let mut h = LatencyHistogram::new("bench");
    for s in spread() {
        h.record(s);
    }

    group.bench_function("p99", |b| b.iter(|| black_box(h.p99())));
    group.bench_function("quantile_0999", |b| b.iter(|| black_box(h.quantile(0.999))));
    // Six quantiles plus min/max/mean — what a dashboard refresh costs.
    group.bench_function("summary", |b| b.iter(|| black_box(h.summary())));

    group.finish();
}

criterion_group!(benches, bench_record, bench_tick_to_trade, bench_query);
criterion_main!(benches);
