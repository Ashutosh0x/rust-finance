// benches/01_tick_pipeline.rs
//
// VERIFIED SOURCE: rustquant.dev/blog/nanosecond-precision-benchmarking-rust-hft
// VERIFIED SOURCE: HRT blog — huge pages, TSC timer, mfence fencing patterns
// VERIFIED SOURCE: deepengineering.substack.com/p/from-nic-to-p99 (Jan 2026)
//
// Measures: full tick → order book update → strategy signal path

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use hdrhistogram::Histogram;
use std::arch::x86_64::{_mm_lfence, _mm_mfence, _rdtsc};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static TSC_FREQ_MHZ: AtomicU64 = AtomicU64::new(0);

#[inline(always)]
unsafe fn tsc_start() -> u64 {
    _mm_mfence();
    let t = _rdtsc();
    _mm_lfence();
    t
}

#[inline(always)]
unsafe fn tsc_stop() -> u64 {
    _mm_lfence();
    let t = _rdtsc();
    _mm_mfence();
    t
}

fn tsc_to_ns(cycles: u64) -> u64 {
    let freq = TSC_FREQ_MHZ.load(Ordering::Relaxed);
    if freq == 0 { return cycles; }
    (cycles * 1_000) / freq
}

pub fn calibrate_tsc() {
    let start_wall = std::time::Instant::now();
    let start_tsc = unsafe { _rdtsc() };
    std::thread::sleep(Duration::from_millis(500));
    let end_wall = std::time::Instant::now();
    let end_tsc = unsafe { _rdtsc() };

    let elapsed_ns = end_wall.duration_since(start_wall).as_nanos() as u64;
    let cycles = end_tsc - start_tsc;
    let freq_mhz = (cycles * 1_000) / elapsed_ns;
    TSC_FREQ_MHZ.store(freq_mhz, Ordering::Relaxed);
}

#[derive(Clone)]
pub struct MarketTick {
    pub symbol_id: u32,
    pub price: f64,
    pub size: f64,
    pub bid: f64,
    pub ask: f64,
    pub timestamp_ns: u64,
}

pub struct OrderBook {
    bids: [f64; 128],
    asks: [f64; 128],
    bid_sizes: [f64; 128],
    ask_sizes: [f64; 128],
    best_bid_idx: usize,
    best_ask_idx: usize,
}

impl OrderBook {
    pub fn new() -> Self {
        Self {
            bids: [0.0; 128], asks: [0.0; 128],
            bid_sizes: [0.0; 128], ask_sizes: [0.0; 128],
            best_bid_idx: 0, best_ask_idx: 0,
        }
    }

    #[inline(always)]
    pub fn update(&mut self, tick: &MarketTick) {
        let bid_idx = self.best_bid_idx;
        self.bids[bid_idx] = tick.bid;
        self.bid_sizes[bid_idx] = tick.size;

        let ask_idx = self.best_ask_idx;
        self.asks[ask_idx] = tick.ask;
        self.ask_sizes[ask_idx] = tick.size;

        self.best_bid_idx = (bid_idx + 1) & 127;
        self.best_ask_idx = (ask_idx + 1) & 127;
    }
}

#[inline(always)]
pub fn compute_imbalance_signal(book: &OrderBook) -> f64 {
    let bid_vol: f64 = book.bid_sizes.iter().take(5).sum();
    let ask_vol: f64 = book.ask_sizes.iter().take(5).sum();
    let total = bid_vol + ask_vol;
    if total > 0.0 { bid_vol / total } else { 0.5 }
}

#[inline(always)]
pub fn full_pipeline(book: &mut OrderBook, tick: &MarketTick) -> f64 {
    book.update(tick);
    compute_imbalance_signal(book)
}

fn bench_order_book_update(c: &mut Criterion) {
    let tick = MarketTick {
        symbol_id: 1, price: 175.50, size: 100.0,
        bid: 175.49, ask: 175.51, timestamp_ns: 1_700_000_000_000,
    };
    let mut book = OrderBook::new();

    c.bench_function("order_book_update", |b| {
        b.iter(|| { book.update(black_box(&tick)); });
    });
}

fn bench_full_pipeline(c: &mut Criterion) {
    let tick = MarketTick {
        symbol_id: 1, price: 175.50, size: 100.0,
        bid: 175.49, ask: 175.51, timestamp_ns: 1_700_000_000_000,
    };
    let mut book = OrderBook::new();

    c.bench_function("full_tick_pipeline", |b| {
        b.iter(|| { black_box(full_pipeline(&mut book, black_box(&tick))); });
    });
}

fn bench_tsc_hdr_histogram(c: &mut Criterion) {
    calibrate_tsc();
    let tick = MarketTick {
        symbol_id: 1, price: 175.50, size: 100.0,
        bid: 175.49, ask: 175.51, timestamp_ns: 0,
    };
    let mut book = OrderBook::new();

    c.bench_function("tick_pipeline_tsc_p999", |b| {
        let mut hist = Histogram::<u64>::new_with_bounds(1, 1_000_000, 3).unwrap();
        b.iter(|| {
            let start = unsafe { tsc_start() };
            black_box(full_pipeline(&mut book, black_box(&tick)));
            let stop = unsafe { tsc_stop() };
            let _ = hist.record(tsc_to_ns(stop - start));
        });
        println!(
            "\n[tick_pipeline] P50={} ns | P99={} ns | P999={} ns | max={} ns",
            hist.value_at_quantile(0.50), hist.value_at_quantile(0.99), hist.value_at_quantile(0.999), hist.max(),
        );
    });
}

criterion_group! {
    name = tick_benches;
    config = Criterion::default().measurement_time(Duration::from_secs(5)).sample_size(1000);
    targets = bench_order_book_update, bench_full_pipeline, bench_tsc_hdr_histogram
}
criterion_main!(tick_benches);
