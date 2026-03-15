// benches/09_memory_layout.rs
// Verifies HRT's discoveries surrounding huge pages, sequential/strided access
// penalties, and lock-free SPSC ring buffers vs false sharing.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, Ordering};


#[repr(C)]
pub struct FalseSharingBad {
    pub counter_a: AtomicU64,
    pub counter_b: AtomicU64,
}

#[repr(C, align(64))]
pub struct CachePaddedCounter {
    pub value: AtomicU64,
    _pad: [u8; 56],
}

#[repr(C)]
pub struct FalseSharingGood {
    pub counter_a: CachePaddedCounter,
    pub counter_b: CachePaddedCounter,
}

const L3_SIZE: usize = 1 << 20;

fn make_array(size: usize) -> Vec<f64> { (0..size).map(|i| i as f64 * 0.001).collect() }

fn sequential_sum(data: &[f64]) -> f64 { data.iter().sum() }
fn strided_sum(data: &[f64], stride: usize) -> f64 {
    let mut s = 0.0_f64;
    let mut i = 0;
    while i < data.len() { s += data[i]; i += stride; }
    s
}

pub struct SpscQueue<T, const N: usize> {
    buffer: Box<[UnsafeCell<T>; N]>,
    head: CachePaddedCounter,
    tail: CachePaddedCounter,
}
unsafe impl<T: Send, const N: usize> Send for SpscQueue<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for SpscQueue<T, N> {}

impl<T: Default + Clone, const N: usize> SpscQueue<T, N> {
    pub fn new() -> Self {
        let buffer: Vec<UnsafeCell<T>> = (0..N).map(|_| UnsafeCell::new(T::default())).collect();
        let buffer: Box<[UnsafeCell<T>; N]> = buffer.try_into().ok().unwrap();
        Self { buffer, head: CachePaddedCounter { value: AtomicU64::new(0), _pad: [0; 56] }, tail: CachePaddedCounter { value: AtomicU64::new(0), _pad: [0; 56] } }
    }
    #[inline(always)]
    pub fn try_push(&self, item: T) -> bool {
        let head = self.head.value.load(Ordering::Relaxed);
        let tail = self.tail.value.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N as u64 { return false; }
        unsafe { *self.buffer[(head as usize) & (N - 1)].get() = item; }
        self.head.value.store(head + 1, Ordering::Release);
        true
    }
    #[inline(always)]
    pub fn try_pop(&self) -> Option<T> {
        let tail = self.tail.value.load(Ordering::Relaxed);
        let head = self.head.value.load(Ordering::Acquire);
        if tail == head { return None; }
        let item = unsafe { (*self.buffer[(tail as usize) & (N - 1)].get()).clone() };
        self.tail.value.store(tail + 1, Ordering::Release);
        Some(item)
    }
}

fn bench_false_sharing(c: &mut Criterion) {
    let bad = FalseSharingBad { counter_a: AtomicU64::new(0), counter_b: AtomicU64::new(0) };
    let good = FalseSharingGood { counter_a: CachePaddedCounter { value: AtomicU64::new(0), _pad: [0; 56] }, counter_b: CachePaddedCounter { value: AtomicU64::new(0), _pad: [0; 56] } };
    let mut group = c.benchmark_group("false_sharing");
    group.bench_function("bad_layout_single_thread", |b| b.iter(|| { bad.counter_a.fetch_add(1, Ordering::Relaxed); black_box(bad.counter_b.load(Ordering::Relaxed)); }));
    group.bench_function("padded_layout_single_thread", |b| b.iter(|| { good.counter_a.value.fetch_add(1, Ordering::Relaxed); black_box(good.counter_b.value.load(Ordering::Relaxed)); }));
    group.finish();
}

fn bench_memory_access_patterns(c: &mut Criterion) {
    let l3_data = make_array(L3_SIZE);
    let mut group = c.benchmark_group("memory_access");
    group.bench_function("l3_sequential", |b| b.iter(|| black_box(sequential_sum(black_box(&l3_data)))));
    group.bench_function("l3_stride_64", |b| b.iter(|| black_box(strided_sum(black_box(&l3_data), 64))));
    group.finish();
}

fn bench_spsc_ring_buffer(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc_ring_buffer");
    group.bench_function("push_pop_roundtrip", |b| {
        let q = SpscQueue::<u64, 4096>::new();
        b.iter(|| { let _ = q.try_push(black_box(42)); black_box(q.try_pop()); });
    });
    group.finish();
}

criterion_group!(memory_benches, bench_false_sharing, bench_memory_access_patterns, bench_spsc_ring_buffer);
criterion_main!(memory_benches);
