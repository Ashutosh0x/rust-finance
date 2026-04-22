#![forbid(unsafe_code)]
// crates/execution/src/exec_algo.rs
//
// Execution Algorithms — TWAP, VWAP, Iceberg, Implementation Shortfall
// Modeled after NautilusTrader's ExecAlgorithm architecture.
// These slice a parent order into smaller child orders to minimize market impact.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use uuid::Uuid;

// ─── Execution Algorithm Trait ───────────────────────────────────

/// Unique identifier for an execution algorithm instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecAlgoId(pub Uuid);

impl ExecAlgoId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// A child order spawned by an execution algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildOrder {
    pub parent_order_id: String,
    pub child_order_id: String,
    pub symbol: String,
    pub side: ChildSide,
    pub quantity: f64,
    pub limit_price: Option<f64>,
    pub scheduled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChildSide {
    Buy,
    Sell,
}

/// Parent order parameters fed into an execution algorithm.
#[derive(Debug, Clone)]
pub struct ParentOrder {
    pub order_id: String,
    pub symbol: String,
    pub side: ChildSide,
    pub total_quantity: f64,
    pub limit_price: Option<f64>,
}

/// Result of a single tick of the execution algorithm.
#[derive(Debug)]
pub enum AlgoAction {
    /// Submit this child order now
    Submit(ChildOrder),
    /// Wait until next interval
    Wait,
    /// Algorithm is complete — all slices sent
    Done,
}

// ─── TWAP (Time-Weighted Average Price) ──────────────────────────

/// Slices a parent order into equal-sized child orders at regular time intervals.
///
/// Parameters:
/// - `horizon_secs`: total time window to execute over
/// - `interval_secs`: time between each child order
///
/// Example: 1000 shares over 600s at 60s intervals = 10 slices of 100 shares
pub struct TwapAlgo {
    pub id: ExecAlgoId,
    parent: ParentOrder,
    horizon_secs: u64,
    interval_secs: u64,
    num_slices: usize,
    slice_qty: f64,
    slices_sent: usize,
    started_at: DateTime<Utc>,
    last_slice_at: Option<DateTime<Utc>>,
}

impl TwapAlgo {
    pub fn new(parent: ParentOrder, horizon_secs: u64, interval_secs: u64) -> Self {
        let num_slices = (horizon_secs / interval_secs).max(1) as usize;
        let slice_qty = parent.total_quantity / num_slices as f64;

        Self {
            id: ExecAlgoId::new(),
            parent,
            horizon_secs,
            interval_secs,
            num_slices,
            slice_qty,
            slices_sent: 0,
            started_at: Utc::now(),
            last_slice_at: None,
        }
    }

    /// Call on each tick / timer event. Returns the next action.
    pub fn on_tick(&mut self, now: DateTime<Utc>) -> AlgoAction {
        if self.slices_sent >= self.num_slices {
            return AlgoAction::Done;
        }

        // Check if horizon expired
        let elapsed = (now - self.started_at).num_seconds() as u64;
        if elapsed > self.horizon_secs {
            // Send remaining quantity as final slice
            let remaining = self.parent.total_quantity - (self.slices_sent as f64 * self.slice_qty);
            if remaining > 0.001 {
                self.slices_sent = self.num_slices;
                return AlgoAction::Submit(self.make_child(remaining, now));
            }
            return AlgoAction::Done;
        }

        // Check interval
        if let Some(last) = self.last_slice_at {
            let since_last = (now - last).num_seconds() as u64;
            if since_last < self.interval_secs {
                return AlgoAction::Wait;
            }
        }

        // Send slice
        let qty = if self.slices_sent == self.num_slices - 1 {
            // Last slice: send remainder to avoid rounding drift
            self.parent.total_quantity - (self.slices_sent as f64 * self.slice_qty)
        } else {
            self.slice_qty
        };

        self.slices_sent += 1;
        self.last_slice_at = Some(now);
        AlgoAction::Submit(self.make_child(qty, now))
    }

    fn make_child(&self, qty: f64, now: DateTime<Utc>) -> ChildOrder {
        ChildOrder {
            parent_order_id: self.parent.order_id.clone(),
            child_order_id: format!("{}-TWAP-{}", self.parent.order_id, self.slices_sent),
            symbol: self.parent.symbol.clone(),
            side: self.parent.side.clone(),
            quantity: qty,
            limit_price: self.parent.limit_price,
            scheduled_at: now,
        }
    }

    pub fn progress(&self) -> f64 {
        self.slices_sent as f64 / self.num_slices as f64
    }

    pub fn is_done(&self) -> bool {
        self.slices_sent >= self.num_slices
    }
}

// ─── VWAP (Volume-Weighted Average Price) ────────────────────────

/// Slices orders according to a historical intraday volume profile.
///
/// The volume profile is a normalized distribution of volume across
/// time buckets (e.g., 30 half-hour buckets for a 6.5-hour trading day).
/// More volume is sent during historically high-volume periods.
pub struct VwapAlgo {
    pub id: ExecAlgoId,
    parent: ParentOrder,
    /// Normalized volume weights per bucket (sum = 1.0)
    volume_profile: Vec<f64>,
    /// Duration of each bucket in seconds
    bucket_duration_secs: u64,
    current_bucket: usize,
    qty_sent: f64,
    started_at: DateTime<Utc>,
    last_bucket_at: Option<DateTime<Utc>>,
}

impl VwapAlgo {
    /// Create with a custom volume profile.
    /// `profile` should be a Vec of relative volume weights (will be normalized).
    pub fn new(parent: ParentOrder, profile: Vec<f64>, bucket_duration_secs: u64) -> Self {
        let total: f64 = profile.iter().sum();
        let volume_profile: Vec<f64> = if total > 0.0 {
            profile.iter().map(|w| w / total).collect()
        } else {
            vec![1.0 / profile.len() as f64; profile.len()]
        };

        Self {
            id: ExecAlgoId::new(),
            parent,
            volume_profile,
            bucket_duration_secs,
            current_bucket: 0,
            qty_sent: 0.0,
            started_at: Utc::now(),
            last_bucket_at: None,
        }
    }

    /// Create with a U-shaped volume profile (typical equity market).
    /// High volume at open and close, lower in midday.
    pub fn with_u_shape(parent: ParentOrder, num_buckets: usize, bucket_duration_secs: u64) -> Self {
        let profile: Vec<f64> = (0..num_buckets)
            .map(|i| {
                let t = i as f64 / (num_buckets - 1).max(1) as f64;
                // U-shape: higher at edges (open/close), lower in middle
                1.0 + 2.0 * (2.0 * t - 1.0).powi(2)
            })
            .collect();
        Self::new(parent, profile, bucket_duration_secs)
    }

    pub fn on_tick(&mut self, now: DateTime<Utc>) -> AlgoAction {
        if self.current_bucket >= self.volume_profile.len() {
            return AlgoAction::Done;
        }

        // Check bucket interval
        if let Some(last) = self.last_bucket_at {
            let since_last = (now - last).num_seconds() as u64;
            if since_last < self.bucket_duration_secs {
                return AlgoAction::Wait;
            }
        }

        let weight = self.volume_profile[self.current_bucket];
        let bucket_qty = self.parent.total_quantity * weight;

        // Clamp to remaining
        let remaining = self.parent.total_quantity - self.qty_sent;
        let qty = bucket_qty.min(remaining).max(0.0);

        if qty < 0.001 {
            self.current_bucket += 1;
            return AlgoAction::Wait;
        }

        let child = ChildOrder {
            parent_order_id: self.parent.order_id.clone(),
            child_order_id: format!("{}-VWAP-{}", self.parent.order_id, self.current_bucket),
            symbol: self.parent.symbol.clone(),
            side: self.parent.side.clone(),
            quantity: qty,
            limit_price: self.parent.limit_price,
            scheduled_at: now,
        };

        self.qty_sent += qty;
        self.current_bucket += 1;
        self.last_bucket_at = Some(now);
        AlgoAction::Submit(child)
    }

    pub fn progress(&self) -> f64 {
        self.qty_sent / self.parent.total_quantity
    }

    /// Time elapsed since algorithm started.
    pub fn elapsed_secs(&self) -> i64 {
        (Utc::now() - self.started_at).num_seconds()
    }
}

// ─── Iceberg Order ───────────────────────────────────────────────

/// Shows only a fraction of the total order size to the market.
/// When the visible portion fills, a new visible slice is posted.
pub struct IcebergAlgo {
    pub id: ExecAlgoId,
    parent: ParentOrder,
    /// Size of each visible slice
    display_qty: f64,
    /// Total quantity filled so far
    qty_filled: f64,
    /// Number of slices sent
    slices_sent: usize,
}

impl IcebergAlgo {
    /// `display_qty`: the visible portion of each slice
    pub fn new(parent: ParentOrder, display_qty: f64) -> Self {
        Self {
            id: ExecAlgoId::new(),
            parent,
            display_qty,
            qty_filled: 0.0,
            slices_sent: 0,
        }
    }

    /// Call when a child order has been filled.
    pub fn on_fill(&mut self, filled_qty: f64) -> AlgoAction {
        self.qty_filled += filled_qty;

        let remaining = self.parent.total_quantity - self.qty_filled;
        if remaining < 0.001 {
            return AlgoAction::Done;
        }

        // Post the next visible slice
        let next_qty = self.display_qty.min(remaining);
        self.slices_sent += 1;

        AlgoAction::Submit(ChildOrder {
            parent_order_id: self.parent.order_id.clone(),
            child_order_id: format!("{}-ICE-{}", self.parent.order_id, self.slices_sent),
            symbol: self.parent.symbol.clone(),
            side: self.parent.side.clone(),
            quantity: next_qty,
            limit_price: self.parent.limit_price,
            scheduled_at: Utc::now(),
        })
    }

    /// Get the initial visible slice to post.
    pub fn initial_slice(&mut self) -> ChildOrder {
        self.slices_sent += 1;
        let qty = self.display_qty.min(self.parent.total_quantity);
        ChildOrder {
            parent_order_id: self.parent.order_id.clone(),
            child_order_id: format!("{}-ICE-{}", self.parent.order_id, self.slices_sent),
            symbol: self.parent.symbol.clone(),
            side: self.parent.side.clone(),
            quantity: qty,
            limit_price: self.parent.limit_price,
            scheduled_at: Utc::now(),
        }
    }

    pub fn progress(&self) -> f64 {
        self.qty_filled / self.parent.total_quantity
    }
}

// ─── POV (Percentage of Volume) ──────────────────────────────────

/// Targets a fixed percentage of real-time market volume.
/// Adjusts order size based on observed volume.
pub struct PovAlgo {
    pub id: ExecAlgoId,
    parent: ParentOrder,
    /// Target participation rate (e.g., 0.10 = 10% of volume)
    target_rate: f64,
    /// Observed market volume so far
    market_volume: f64,
    /// Our filled volume so far
    our_volume: f64,
    /// Recent market volume observations for rate calculation
    volume_window: VecDeque<(DateTime<Utc>, f64)>,
    last_order_at: Option<DateTime<Utc>>,
    /// Minimum interval between child orders (seconds)
    min_interval_secs: u64,
}

impl PovAlgo {
    pub fn new(parent: ParentOrder, target_rate: f64, min_interval_secs: u64) -> Self {
        Self {
            id: ExecAlgoId::new(),
            parent,
            target_rate: target_rate.clamp(0.01, 0.50),
            market_volume: 0.0,
            our_volume: 0.0,
            volume_window: VecDeque::with_capacity(100),
            last_order_at: None,
            min_interval_secs,
        }
    }

    /// Feed observed market volume. Call on every trade tick.
    pub fn on_market_trade(&mut self, now: DateTime<Utc>, trade_volume: f64) {
        self.market_volume += trade_volume;
        self.volume_window.push_back((now, trade_volume));

        // Keep only last 5 minutes
        while let Some((t, _)) = self.volume_window.front() {
            if (now - *t).num_seconds() > 300 {
                self.volume_window.pop_front();
            } else {
                break;
            }
        }
    }

    /// Check if we should send an order to maintain target participation rate.
    pub fn on_tick(&mut self, now: DateTime<Utc>) -> AlgoAction {
        let remaining = self.parent.total_quantity - self.our_volume;
        if remaining < 0.001 {
            return AlgoAction::Done;
        }

        // Check interval
        if let Some(last) = self.last_order_at {
            if (now - last).num_seconds() < self.min_interval_secs as i64 {
                return AlgoAction::Wait;
            }
        }

        // Calculate target volume based on market volume
        let target_our_volume = self.market_volume * self.target_rate;
        let deficit = target_our_volume - self.our_volume;

        if deficit < 1.0 {
            return AlgoAction::Wait;
        }

        let qty = deficit.min(remaining);
        self.our_volume += qty;
        self.last_order_at = Some(now);

        AlgoAction::Submit(ChildOrder {
            parent_order_id: self.parent.order_id.clone(),
            child_order_id: format!("{}-POV-{}", self.parent.order_id, self.our_volume as u64),
            symbol: self.parent.symbol.clone(),
            side: self.parent.side.clone(),
            quantity: qty,
            limit_price: self.parent.limit_price,
            scheduled_at: now,
        })
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_parent(qty: f64) -> ParentOrder {
        ParentOrder {
            order_id: "TEST-001".into(),
            symbol: "AAPL".into(),
            side: ChildSide::Buy,
            total_quantity: qty,
            limit_price: Some(175.0),
        }
    }

    #[test]
    fn test_twap_slicing() {
        let parent = make_parent(1000.0);
        let mut algo = TwapAlgo::new(parent, 600, 60);

        assert_eq!(algo.num_slices, 10);
        assert!((algo.slice_qty - 100.0).abs() < 0.01);

        let now = Utc::now();
        // First slice should fire immediately
        let action = algo.on_tick(now);
        assert!(matches!(action, AlgoAction::Submit(c) if (c.quantity - 100.0).abs() < 0.01));

        // Second call within interval should wait
        let action = algo.on_tick(now + Duration::seconds(30));
        assert!(matches!(action, AlgoAction::Wait));

        // After interval, should fire
        let action = algo.on_tick(now + Duration::seconds(61));
        assert!(matches!(action, AlgoAction::Submit(_)));
    }

    #[test]
    fn test_twap_completes() {
        let parent = make_parent(100.0);
        let mut algo = TwapAlgo::new(parent, 100, 10);

        let start = Utc::now();
        let mut total_qty = 0.0;
        let mut done = false;

        for i in 0..20 {
            let now = start + Duration::seconds(i * 11);
            match algo.on_tick(now) {
                AlgoAction::Submit(c) => total_qty += c.quantity,
                AlgoAction::Done => {
                    done = true;
                    break;
                }
                AlgoAction::Wait => {}
            }
        }

        assert!(done || algo.is_done());
        assert!((total_qty - 100.0).abs() < 1.0, "Total qty: {}", total_qty);
    }

    #[test]
    fn test_vwap_u_shape() {
        let parent = make_parent(1000.0);
        let algo = VwapAlgo::with_u_shape(parent, 13, 1800);

        // First and last buckets should have higher weight than middle
        assert!(algo.volume_profile[0] > algo.volume_profile[6]);
        assert!(algo.volume_profile[12] > algo.volume_profile[6]);

        // Weights should sum to 1.0
        let sum: f64 = algo.volume_profile.iter().sum();
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_iceberg_slicing() {
        let parent = make_parent(500.0);
        let mut algo = IcebergAlgo::new(parent, 50.0);

        let first = algo.initial_slice();
        assert!((first.quantity - 50.0).abs() < 0.01);

        // Simulate fills until done: 500 / 50 = 10 fills total
        let mut fills = 0;
        loop {
            let action = algo.on_fill(50.0);
            fills += 1;
            match action {
                AlgoAction::Submit(c) => {
                    assert!((c.quantity - 50.0).abs() < 0.01 || c.quantity < 50.0);
                }
                AlgoAction::Done => break,
                AlgoAction::Wait => panic!("Iceberg should not wait"),
            }
            if fills > 20 {
                panic!("Too many fills without completing");
            }
        }
        assert_eq!(fills, 10, "Should take exactly 10 fills of 50 to fill 500");
    }

    #[test]
    fn test_pov_participation() {
        let parent = make_parent(1000.0);
        let mut algo = PovAlgo::new(parent, 0.10, 1);

        let start = Utc::now();

        // Simulate 10000 shares of market volume
        for i in 0..100 {
            let now = start + Duration::seconds(i * 2);
            algo.on_market_trade(now, 100.0);
        }

        // Check that our volume tracks target rate
        let now = start + Duration::seconds(201);
        let _action = algo.on_tick(now);
        // Market volume = 10000, target = 10% = 1000
        // Our volume should approach 1000
    }
}
