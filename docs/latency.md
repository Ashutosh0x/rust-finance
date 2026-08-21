# Latency measurement

What is measured, what it costs to measure it, and what is still not wired.

## The model

Two halves, on one monotonic clock:

```
  recv ──▶ decoded ──▶ book ──▶ signal ──▶ encoded ──▶ sent
       wire      decode    book     decide    encode    send
       └───────── FeedLatency ─────┘└──────── TradeLatency ────┘
       └──────────────────── total ───────────────────────────┘
```

| Stage | Span |
|---|---|
| `wire` | exchange source timestamp → local receive (wire + stack) |
| `decode` | local receive → message decoded |
| `book` | decoded → book updated |
| `decide` | book updated → strategy produced a signal |
| `encode` | signal → order encoded and risk-checked |
| `send` | encoded → handed to the transport |
| `total` | receive → order on the wire |

### `total` is measured, never summed

`TickToTrade::record_path` records the end-to-end span directly.

Summing stage percentiles would overstate the tail badly: the stages do not hit
their worst case on the same message, so adding six p99s describes a message
that never existed. The only honest end-to-end number is one measured
end-to-end. `end_to_end_is_measured_not_summed` pins this.

### A non-monotonic path records nothing

If the timestamps handed to `record_path` are not strictly non-decreasing, the
whole path is dropped — not just the offending stage.

`record_span` already ignores a negative interval, so a backwards clock would
otherwise leave a *partially* recorded path: several stages present, one
missing, and a `total` that does not equal what happened. That is worse than a
missing sample, because it looks complete.

## The histogram

Fixed-bucket log-linear, the same scheme HdrHistogram uses minus the
auto-resizing:

- 16 sub-buckets per power of two → **≤ ~3.1% relative error**
- 40 magnitudes → 1 ns … ~18 minutes
- 640 buckets, constant memory, **no allocation on record**
- No sorting at query time

An empty histogram returns `None`, never `0`. A zero would read as "instant"
rather than "never measured", and `empty_histogram_reports_nothing_rather_than_zero`
keeps it that way.

## What the instrumentation costs

Measured by `benchmarks/benches/07_latency_recording.rs`. **Windows, where
`Instant` reads `QueryPerformanceCounter`** — a Linux vDSO `clock_gettime` is
typically faster, so re-run before quoting these on a production host.

| Operation | Time |
|---|---|
| `record` | ~6–8 ns |
| `record_span` | ~6.6 ns |
| **`now_monotonic_ns`** | **~42 ns** |
| `record_path` (6 stages + total) | ~24.8 ns |
| `p99` | ~62 ns |
| `quantile(0.999)` | ~63 ns |
| `summary` (6 quantiles + min/max/mean) | ~181 ns |

### The result that matters

**Reading the clock costs roughly six times more than recording a sample**, and
a span needs two readings. Instrumentation cost is therefore dominated by
timestamping, not by the histogram.

That is why `record_path` takes six timestamps as arguments instead of reading
the clock itself: seven histogram updates for ~25 ns, versus ~250 ns if each
stage read the clock twice. **Take a timestamp once at each boundary and reuse
it** — do not call `now_monotonic_ns()` inside a stage that already has one.

It also sets the bar for whether this stays on by default. An ITCH decode is
tens of nanoseconds (`06_direct_exchange_feeds`), so a stage that reads the
clock twice more than doubles its own cost. Stage boundaries have to be chosen
so that measurement is a small fraction of what is being measured.

The query path is off the hot path — `summary` walks 640 buckets and is a
dashboard refresh, not a per-message cost.

## Why the clock lives in `exchange-core`

`now_monotonic_ns` is in `exchange_core::latency`, not in a feed handler.

Each `Instant` origin is process-local but *crate-local by construction* if
every crate defines its own. Two origins produce readings that cannot be
subtracted from each other — which is precisely what a tick-to-trade span does,
since it crosses `nasdaq`/`nyse` into `exchange` and on into execution. One
origin, one shared function.

It is deliberately not wall-clock. An NTP step would produce a negative
interval, `record_span` would drop the sample, and a clock correction would look
like a quiet gap in measurement rather than an error.

## Status — what is actually wired

| Path | State |
|---|---|
| `decode` on the ITCH live path | **Recorded** |
| `book` on the ITCH live path | **Recorded**, only when a book event was produced |
| `wire` | **Not recorded** — needs the exchange source timestamp compared against local receive |
| NYSE XDP, all stages | **Not recorded** — the handler builds a `FeedLatency` and never writes to it |
| `TradeLatency` (decide/encode/send) | **Not recorded** — no execution path calls it yet |
| Anything reading `.latency()` | **Nothing** — no TUI panel, no metrics export |

### The bug this replaced

The live path previously called `handler.on_message(raw, 0)`, and
`ItchFeedHandler` skips recording when `recv_ns` is 0. Both transports share
that call site, so **every histogram was fed nothing wherever real traffic
flows**, while the feature was listed in the README. The receive timestamp is
now taken as the datagram comes off the socket and threaded through.

Charging `book` only when an event was produced is the same kind of care: a
message that never reached the book would otherwise dilute the tail the
histogram exists to expose.

## Running the benchmarks

```bash
cargo bench -p benchmarks --bench 07_latency_recording

# Quick pass
cargo bench -p benchmarks --bench 07_latency_recording -- \
  --warm-up-time 1 --measurement-time 3
```

Criterion writes HTML reports to `target/criterion/`.
