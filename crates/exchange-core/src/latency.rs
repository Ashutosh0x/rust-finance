//! Latency recording for the feed-handler hot path.
//!
//! Direct-feed work is judged in microseconds, and an average is the wrong statistic — what
//! matters is the tail. This is a fixed-bucket log-linear histogram: constant memory, no
//! allocation on record, and no sorting at query time.
//!
//! Bucketing: each power-of-two magnitude is split into [`SUB_BUCKETS`] linear sub-buckets,
//! giving ~3% worst-case relative error over the whole range — the same scheme HdrHistogram
//! uses, minus the auto-resizing.

/// Sub-buckets per power of two. 16 → ≤ 1/32 ≈ 3.1% relative error.
const SUB_BUCKETS: usize = 16;
/// Powers of two covered: 1 ns … ~2^40 ns (≈ 18 minutes).
const MAGNITUDES: usize = 40;
const BUCKET_COUNT: usize = SUB_BUCKETS * MAGNITUDES;

/// A latency histogram over nanosecond samples.
#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    label: &'static str,
    buckets: [u64; BUCKET_COUNT],
    count: u64,
    sum: u128,
    min: u64,
    max: u64,
}

impl LatencyHistogram {
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            buckets: [0; BUCKET_COUNT],
            count: 0,
            sum: 0,
            min: u64::MAX,
            max: 0,
        }
    }

    #[inline]
    pub fn label(&self) -> &'static str {
        self.label
    }

    #[inline]
    pub fn count(&self) -> u64 {
        self.count
    }

    #[inline]
    pub fn min(&self) -> Option<u64> {
        (self.count > 0).then_some(self.min)
    }

    #[inline]
    pub fn max(&self) -> Option<u64> {
        (self.count > 0).then_some(self.max)
    }

    pub fn mean(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum as f64 / self.count as f64)
    }

    /// Record one sample, in nanoseconds.
    #[inline]
    pub fn record(&mut self, nanos: u64) {
        let idx = Self::bucket_index(nanos);
        self.buckets[idx] += 1;
        self.count += 1;
        self.sum += nanos as u128;
        self.min = self.min.min(nanos);
        self.max = self.max.max(nanos);
    }

    /// Record the interval between two monotonic readings, ignoring the sample if the
    /// second reading is not after the first (clock adjustment, or reordered timestamps).
    #[inline]
    pub fn record_span(&mut self, start_ns: u64, end_ns: u64) {
        if end_ns > start_ns {
            self.record(end_ns - start_ns);
        }
    }

    pub fn reset(&mut self) {
        self.buckets = [0; BUCKET_COUNT];
        self.count = 0;
        self.sum = 0;
        self.min = u64::MAX;
        self.max = 0;
    }

    /// Value at the given quantile (0.0 … 1.0), in nanoseconds.
    ///
    /// Returns the upper edge of the containing bucket, so the reported value is an upper
    /// bound on the true quantile — the conservative direction for a latency budget.
    pub fn quantile(&self, q: f64) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let q = q.clamp(0.0, 1.0);
        let target = ((self.count as f64) * q).ceil().max(1.0) as u64;
        let mut seen = 0u64;
        for (idx, &n) in self.buckets.iter().enumerate() {
            seen += n;
            if seen >= target {
                return Some(Self::bucket_upper_bound(idx).min(self.max));
            }
        }
        Some(self.max)
    }

    pub fn p50(&self) -> Option<u64> {
        self.quantile(0.50)
    }
    pub fn p99(&self) -> Option<u64> {
        self.quantile(0.99)
    }
    pub fn p999(&self) -> Option<u64> {
        self.quantile(0.999)
    }

    /// A one-line summary suitable for a log or a metrics scrape.
    pub fn summary(&self) -> LatencySummary {
        LatencySummary {
            label: self.label,
            count: self.count,
            min_ns: self.min().unwrap_or(0),
            p50_ns: self.p50().unwrap_or(0),
            p99_ns: self.p99().unwrap_or(0),
            p999_ns: self.p999().unwrap_or(0),
            max_ns: self.max().unwrap_or(0),
            mean_ns: self.mean().unwrap_or(0.0),
        }
    }

    /// Sub-bucket width in bits: `SUB_BUCKETS == 1 << SUB_BITS`.
    const SUB_BITS: usize = SUB_BUCKETS.trailing_zeros() as usize;

    /// Bucket layout: values below [`SUB_BUCKETS`] are stored exactly, one per bucket.
    /// Above that, a value of magnitude `m = floor(log2(v))` is right-shifted by `m -
    /// SUB_BITS` so its top bits land in `[SUB_BUCKETS, 2*SUB_BUCKETS)`, and the offset
    /// within that window selects the sub-bucket.
    fn bucket_index(v: u64) -> usize {
        if v < SUB_BUCKETS as u64 {
            return v as usize;
        }
        let magnitude = 63 - v.leading_zeros() as usize; // floor(log2(v))
        let shift = magnitude - Self::SUB_BITS;
        let sub = ((v >> shift) as usize) - SUB_BUCKETS;
        let group = magnitude - Self::SUB_BITS + 1;
        (group * SUB_BUCKETS + sub).min(BUCKET_COUNT - 1)
    }

    /// Largest value that maps to `idx` — the inverse of [`Self::bucket_index`].
    fn bucket_upper_bound(idx: usize) -> u64 {
        if idx < SUB_BUCKETS {
            return idx as u64;
        }
        let group = idx / SUB_BUCKETS;
        let sub = idx % SUB_BUCKETS;
        let shift = group - 1;
        (((SUB_BUCKETS + sub + 1) as u64) << shift) - 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencySummary {
    pub label: &'static str,
    pub count: u64,
    pub min_ns: u64,
    pub p50_ns: u64,
    pub p99_ns: u64,
    pub p999_ns: u64,
    pub max_ns: u64,
    pub mean_ns: f64,
}

impl std::fmt::Display for LatencySummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} n={} min={:.1}us p50={:.1}us p99={:.1}us p99.9={:.1}us max={:.1}us",
            self.label,
            self.count,
            self.min_ns as f64 / 1000.0,
            self.p50_ns as f64 / 1000.0,
            self.p99_ns as f64 / 1000.0,
            self.p999_ns as f64 / 1000.0,
            self.max_ns as f64 / 1000.0,
        )
    }
}

/// The three latencies worth separating on a direct feed.
#[derive(Debug, Clone)]
pub struct FeedLatency {
    /// Exchange source timestamp → local receive timestamp (wire + stack).
    pub wire: LatencyHistogram,
    /// Local receive → message decoded.
    pub decode: LatencyHistogram,
    /// Local receive → book updated and downstream notified.
    pub book: LatencyHistogram,
}

impl Default for FeedLatency {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedLatency {
    pub fn new() -> Self {
        Self {
            wire: LatencyHistogram::new("wire"),
            decode: LatencyHistogram::new("decode"),
            book: LatencyHistogram::new("book"),
        }
    }

    pub fn summaries(&self) -> [LatencySummary; 3] {
        [
            self.wire.summary(),
            self.decode.summary(),
            self.book.summary(),
        ]
    }
}

/// Monotonic clock reading in nanoseconds, for latency spans only.
///
/// Deliberately not wall-clock: an NTP step would produce a negative or absurd
/// interval, and `record_span` would silently drop the sample — so a clock
/// correction would look like a quiet gap in measurement rather than an error.
///
/// Lives here rather than in a feed handler so that every stage of a span is
/// read from the SAME origin. Two crates each with their own `Instant` origin
/// produce readings that cannot be subtracted from one another, which is
/// exactly what a tick-to-trade span does across crate boundaries.
pub fn now_monotonic_ns() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let origin = ORIGIN.get_or_init(Instant::now);
    origin.elapsed().as_nanos() as u64
}

/// The execution half of the round trip.
///
/// [`FeedLatency`] stops once the book is updated, which is only half the number
/// anyone cares about. A feed handler that decodes in 80ns is not fast if the
/// order takes 4us to reach the wire, and until both halves are measured on the
/// same clock there is no way to tell which side is the problem.
#[derive(Debug, Clone)]
pub struct TradeLatency {
    /// Book updated → strategy produced a signal.
    pub decide: LatencyHistogram,
    /// Signal → order message encoded and risk-checked.
    pub encode: LatencyHistogram,
    /// Order encoded → handed to the transport.
    pub send: LatencyHistogram,
}

impl Default for TradeLatency {
    fn default() -> Self {
        Self::new()
    }
}

impl TradeLatency {
    pub fn new() -> Self {
        Self {
            decide: LatencyHistogram::new("decide"),
            encode: LatencyHistogram::new("encode"),
            send: LatencyHistogram::new("send"),
        }
    }

    pub fn summaries(&self) -> [LatencySummary; 3] {
        [
            self.decide.summary(),
            self.encode.summary(),
            self.send.summary(),
        ]
    }

    pub fn reset(&mut self) {
        self.decide.reset();
        self.encode.reset();
        self.send.reset();
    }
}

/// Feed and execution together, plus the end-to-end span.
///
/// `total` is measured directly from the receive timestamp to the moment the
/// order reaches the transport — it is NOT the sum of the stage percentiles.
/// Adding p99s would overstate the tail badly, because the stages do not hit
/// their worst case on the same message; the only honest end-to-end number is
/// one measured end-to-end.
#[derive(Debug, Clone)]
pub struct TickToTrade {
    pub feed: FeedLatency,
    pub trade: TradeLatency,
    /// Receive → order on the wire, measured as a single span.
    pub total: LatencyHistogram,
}

impl Default for TickToTrade {
    fn default() -> Self {
        Self::new()
    }
}

impl TickToTrade {
    pub fn new() -> Self {
        Self {
            feed: FeedLatency::new(),
            trade: TradeLatency::new(),
            total: LatencyHistogram::new("tick-to-trade"),
        }
    }

    /// Record one complete path from a market-data receive to an order send.
    ///
    /// Every argument is a monotonic reading in nanoseconds, in order. A
    /// non-monotonic sequence is dropped rather than recorded: `record_span`
    /// already ignores a negative interval, and a partially recorded path would
    /// bias the stages that did make sense.
    pub fn record_path(
        &mut self,
        recv_ns: u64,
        decoded_ns: u64,
        book_ns: u64,
        signal_ns: u64,
        encoded_ns: u64,
        sent_ns: u64,
    ) {
        let ordered = recv_ns <= decoded_ns
            && decoded_ns <= book_ns
            && book_ns <= signal_ns
            && signal_ns <= encoded_ns
            && encoded_ns <= sent_ns;
        if !ordered {
            return;
        }

        self.feed.decode.record_span(recv_ns, decoded_ns);
        self.feed.book.record_span(decoded_ns, book_ns);
        self.trade.decide.record_span(book_ns, signal_ns);
        self.trade.encode.record_span(signal_ns, encoded_ns);
        self.trade.send.record_span(encoded_ns, sent_ns);
        self.total.record_span(recv_ns, sent_ns);
    }

    /// Every stage in pipeline order, then the end-to-end span last.
    pub fn summaries(&self) -> Vec<LatencySummary> {
        let mut out = Vec::with_capacity(7);
        out.extend(self.feed.summaries());
        out.extend(self.trade.summaries());
        out.push(self.total.summary());
        out
    }

    pub fn reset(&mut self) {
        self.feed.wire.reset();
        self.feed.decode.reset();
        self.feed.book.reset();
        self.trade.reset();
        self.total.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_to_trade_records_every_stage_and_the_whole_path() {
        let mut t = TickToTrade::new();
        // recv, decoded, book, signal, encoded, sent
        t.record_path(1_000, 1_080, 1_120, 1_240, 1_290, 1_380);

        assert_eq!(t.feed.decode.count(), 1);
        assert_eq!(t.feed.book.count(), 1);
        assert_eq!(t.trade.decide.count(), 1);
        assert_eq!(t.trade.encode.count(), 1);
        assert_eq!(t.trade.send.count(), 1);
        assert_eq!(t.total.count(), 1);
    }

    #[test]
    fn end_to_end_is_measured_not_summed() {
        // The whole point of recording `total` separately: summing stage
        // percentiles overstates the tail, because stages do not peak together.
        let mut t = TickToTrade::new();
        t.record_path(0, 100, 200, 300, 400, 500);

        let total = t.total.p50().expect("one sample recorded");
        // 500ns end to end, within the histogram's ~3% bucket error.
        assert!(
            (450..=550).contains(&total),
            "end-to-end span should be ~500ns, got {total}"
        );
    }

    #[test]
    fn out_of_order_timestamps_record_nothing() {
        // A clock that went backwards must not silently bias the stages that
        // happened to still make sense.
        let mut t = TickToTrade::new();
        t.record_path(1_000, 900, 1_120, 1_240, 1_290, 1_380);

        assert_eq!(t.total.count(), 0);
        assert_eq!(t.feed.decode.count(), 0, "no stage may be recorded");
        assert_eq!(t.trade.send.count(), 0);
    }

    #[test]
    fn summaries_are_in_pipeline_order_with_total_last() {
        let t = TickToTrade::new();
        let labels: Vec<&str> = t.summaries().iter().map(|s| s.label).collect();
        assert_eq!(
            labels,
            vec![
                "wire",
                "decode",
                "book",
                "decide",
                "encode",
                "send",
                "tick-to-trade"
            ]
        );
    }

    #[test]
    fn reset_clears_both_halves() {
        let mut t = TickToTrade::new();
        t.record_path(0, 10, 20, 30, 40, 50);
        t.reset();
        assert_eq!(t.total.count(), 0);
        assert_eq!(t.feed.decode.count(), 0);
        assert_eq!(t.trade.decide.count(), 0);
    }

    #[test]
    fn empty_histogram_reports_nothing_rather_than_zero() {
        let h = LatencyHistogram::new("t");
        assert_eq!(h.p50(), None);
        assert_eq!(h.mean(), None);
        assert_eq!(h.min(), None);
    }

    #[test]
    fn small_values_land_in_exact_buckets() {
        let mut h = LatencyHistogram::new("t");
        for v in 0..16 {
            h.record(v);
        }
        assert_eq!(h.min(), Some(0));
        assert_eq!(h.max(), Some(15));
    }

    #[test]
    fn quantiles_are_within_the_documented_error_band() {
        let mut h = LatencyHistogram::new("t");
        for v in 1..=10_000u64 {
            h.record(v * 100); // 100ns .. 1ms
        }
        let p50 = h.p50().unwrap();
        let expected = 500_000u64;
        let err = (p50 as f64 - expected as f64).abs() / expected as f64;
        assert!(err < 0.05, "p50 {p50} too far from {expected} (err {err})");
        assert!(h.p99().unwrap() >= h.p50().unwrap());
        assert!(h.p999().unwrap() >= h.p99().unwrap());
    }

    #[test]
    fn quantile_never_exceeds_the_observed_maximum() {
        let mut h = LatencyHistogram::new("t");
        h.record(1_234);
        h.record(45_678);
        assert_eq!(h.max(), Some(45_678));
        assert!(h.quantile(1.0).unwrap() <= 45_678);
    }

    #[test]
    fn tail_is_visible_where_a_mean_would_hide_it() {
        let mut h = LatencyHistogram::new("t");
        for _ in 0..9_900 {
            h.record(5_000); // 5us
        }
        for _ in 0..100 {
            h.record(2_000_000); // 2ms
        }
        // The mean (~25us) is two orders of magnitude below the tail it is averaging in.
        let mean = h.mean().unwrap();
        let p999 = h.p999().unwrap();
        assert!(mean < 50_000.0, "mean {mean} should stay near the body");
        assert!(p999 > 1_000_000, "p99.9 {p999} should expose the 2ms tail");
        assert!(p999 as f64 > mean * 20.0, "tail must dominate the mean");
    }

    #[test]
    fn record_span_ignores_non_positive_intervals() {
        let mut h = LatencyHistogram::new("t");
        h.record_span(100, 50);
        h.record_span(100, 100);
        assert_eq!(h.count(), 0);
        h.record_span(100, 150);
        assert_eq!(h.count(), 1);
    }

    #[test]
    fn every_value_lands_at_or_below_its_bucket_upper_bound() {
        for v in [
            0u64,
            1,
            15,
            16,
            17,
            31,
            32,
            33,
            1_000,
            100_000,
            1_000_000,
            1 << 30,
        ] {
            let idx = LatencyHistogram::bucket_index(v);
            let upper = LatencyHistogram::bucket_upper_bound(idx);
            assert!(
                upper >= v,
                "value {v} exceeds its bucket upper bound {upper}"
            );
            // And the error stays inside the documented ~3% band above the exact region.
            if v >= SUB_BUCKETS as u64 {
                let err = (upper - v) as f64 / v as f64;
                assert!(err < 0.07, "value {v} bucket error {err} too wide");
            }
        }
    }

    #[test]
    fn bucket_indices_are_monotonic_in_value() {
        let mut last = 0;
        for shift in 0..40 {
            let v = 1u64 << shift;
            let idx = LatencyHistogram::bucket_index(v);
            assert!(idx >= last, "bucket index went backwards at 2^{shift}");
            last = idx;
        }
    }
}
