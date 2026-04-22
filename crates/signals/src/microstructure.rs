// crates/signals/src/microstructure.rs
//
// Market Microstructure Signals — OFI, Microprice, Kyle's Lambda, VPIN, Amihud
// These are the signals used by Jane Street, IMC, and Citadel for sub-second alpha.
// Reference: Cont et al. (2014), Kyle (1985), Easley et al. (2012)

use std::collections::VecDeque;

// ─── Order Flow Imbalance (OFI) ──────────────────────────────────

/// Measures the net change in bid/ask volume to predict short-term price direction.
///
/// Formula: OFI = Σ(ΔBidSize - ΔAskSize) over window
///
/// Interpretation:
/// - OFI > 0: More buying pressure (aggressive buyers lifting offers)
/// - OFI < 0: More selling pressure (aggressive sellers hitting bids)
///
/// Reference: Cont, Kukanov & Stoikov (2014) "The Price Impact of Order Book Events"
pub struct OrderFlowImbalance {
    prev_bid_size: f64,
    prev_ask_size: f64,
    prev_bid_price: f64,
    prev_ask_price: f64,
    ofi_window: VecDeque<f64>,
    window_size: usize,
    initialized: bool,
}

impl OrderFlowImbalance {
    pub fn new(window_size: usize) -> Self {
        Self {
            prev_bid_size: 0.0,
            prev_ask_size: 0.0,
            prev_bid_price: 0.0,
            prev_ask_price: 0.0,
            ofi_window: VecDeque::with_capacity(window_size + 1),
            window_size,
            initialized: false,
        }
    }

    /// Update with new top-of-book data. Returns current OFI value.
    pub fn update(&mut self, bid_price: f64, bid_size: f64, ask_price: f64, ask_size: f64) -> f64 {
        if !self.initialized {
            self.prev_bid_price = bid_price;
            self.prev_bid_size = bid_size;
            self.prev_ask_price = ask_price;
            self.prev_ask_size = ask_size;
            self.initialized = true;
            return 0.0;
        }

        // Compute bid-side flow
        let delta_bid = if bid_price > self.prev_bid_price {
            bid_size
        } else if bid_price == self.prev_bid_price {
            bid_size - self.prev_bid_size
        } else {
            -self.prev_bid_size
        };

        // Compute ask-side flow
        let delta_ask = if ask_price < self.prev_ask_price {
            ask_size
        } else if ask_price == self.prev_ask_price {
            ask_size - self.prev_ask_size
        } else {
            -self.prev_ask_size
        };

        let ofi_tick = delta_bid - delta_ask;

        self.ofi_window.push_back(ofi_tick);
        if self.ofi_window.len() > self.window_size {
            self.ofi_window.pop_front();
        }

        self.prev_bid_price = bid_price;
        self.prev_bid_size = bid_size;
        self.prev_ask_price = ask_price;
        self.prev_ask_size = ask_size;

        // Return normalized OFI
        let sum: f64 = self.ofi_window.iter().sum();
        let abs_sum: f64 = self.ofi_window.iter().map(|x| x.abs()).sum();
        if abs_sum > 0.0 {
            sum / abs_sum
        } else {
            0.0
        }
    }

    /// Raw (unnormalized) OFI — useful for Kyle's Lambda regression.
    pub fn raw_ofi(&self) -> f64 {
        self.ofi_window.iter().sum()
    }
}

// ─── Microprice ──────────────────────────────────────────────────

/// Size-weighted midpoint — better than arithmetic mid for market making.
///
/// Formula: microprice = ask × (bid_size / (bid_size + ask_size))
///                     + bid × (ask_size / (bid_size + ask_size))
///
/// When bid_size >> ask_size, microprice is pulled toward the ask (upward pressure).
/// This provides a ~50-tick lead over arithmetic mid in predicting price direction.
#[inline]
pub fn microprice(bid: f64, ask: f64, bid_size: f64, ask_size: f64) -> f64 {
    let total = bid_size + ask_size;
    if total < 1e-10 {
        return (bid + ask) / 2.0;
    }
    // Bid-size-weighted: more bids → price likely to go up → microprice closer to ask
    ask * (bid_size / total) + bid * (ask_size / total)
}

/// Microprice imbalance: normalized [-1, 1] signal.
/// +1 = extreme buying pressure, -1 = extreme selling pressure.
#[inline]
pub fn microprice_imbalance(bid_size: f64, ask_size: f64) -> f64 {
    let total = bid_size + ask_size;
    if total < 1e-10 {
        return 0.0;
    }
    (bid_size - ask_size) / total
}

// ─── Kyle's Lambda (Price Impact Coefficient) ────────────────────

/// Estimates the price impact per unit of signed order flow.
///
/// λ = Cov(ΔP, SignedFlow) / Var(SignedFlow)
///
/// Higher λ → market is thin / illiquid / informed trading present.
/// Lower λ → deep liquidity, safe to trade larger sizes.
///
/// Reference: Kyle (1985) "Continuous Auctions and Insider Trading"
pub struct KyleLambda {
    price_changes: VecDeque<f64>,
    signed_flows: VecDeque<f64>,
    window_size: usize,
    prev_mid: f64,
    initialized: bool,
}

impl KyleLambda {
    pub fn new(window_size: usize) -> Self {
        Self {
            price_changes: VecDeque::with_capacity(window_size + 1),
            signed_flows: VecDeque::with_capacity(window_size + 1),
            window_size,
            prev_mid: 0.0,
            initialized: false,
        }
    }

    /// Update with a new trade event.
    /// `trade_price`: the fill price
    /// `mid_price`: current midpoint
    /// `trade_size`: signed quantity (positive = buy, negative = sell)
    pub fn update(&mut self, mid_price: f64, signed_trade_size: f64) -> f64 {
        if !self.initialized {
            self.prev_mid = mid_price;
            self.initialized = true;
            return 0.0;
        }

        let delta_p = mid_price - self.prev_mid;
        self.prev_mid = mid_price;

        self.price_changes.push_back(delta_p);
        self.signed_flows.push_back(signed_trade_size);

        if self.price_changes.len() > self.window_size {
            self.price_changes.pop_front();
            self.signed_flows.pop_front();
        }

        self.compute_lambda()
    }

    fn compute_lambda(&self) -> f64 {
        let n = self.price_changes.len();
        if n < 10 {
            return 0.0;
        }

        let nf = n as f64;
        let mean_dp: f64 = self.price_changes.iter().sum::<f64>() / nf;
        let mean_sf: f64 = self.signed_flows.iter().sum::<f64>() / nf;

        let mut cov = 0.0;
        let mut var_sf = 0.0;

        for i in 0..n {
            let dp_dev = self.price_changes[i] - mean_dp;
            let sf_dev = self.signed_flows[i] - mean_sf;
            cov += dp_dev * sf_dev;
            var_sf += sf_dev * sf_dev;
        }

        if var_sf < 1e-15 {
            return 0.0;
        }

        (cov / var_sf).max(0.0) // Lambda should be non-negative
    }

    /// Current estimated lambda value.
    pub fn lambda(&self) -> f64 {
        self.compute_lambda()
    }
}

// ─── VPIN (Volume-Synchronized Probability of Informed Trading) ──

/// Measures order flow toxicity — the probability that a counterparty is informed.
///
/// High VPIN → dangerous to provide liquidity (adverse selection risk).
/// Low VPIN → safe to quote tighter spreads.
///
/// Reference: Easley, López de Prado & O'Hara (2012)
pub struct Vpin {
    bucket_volume: f64,
    buy_volume: f64,
    sell_volume: f64,
    current_bucket_volume: f64,
    buckets: VecDeque<(f64, f64)>, // (buy_vol, sell_vol) per bucket
    num_buckets: usize,
}

impl Vpin {
    /// `bucket_volume`: volume per bucket (e.g., 1/50th of daily volume)
    /// `num_buckets`: number of buckets in the window (typically 50)
    pub fn new(bucket_volume: f64, num_buckets: usize) -> Self {
        Self {
            bucket_volume,
            buy_volume: 0.0,
            sell_volume: 0.0,
            current_bucket_volume: 0.0,
            buckets: VecDeque::with_capacity(num_buckets + 1),
            num_buckets,
        }
    }

    /// Classify trade direction using tick rule and update VPIN.
    /// Returns current VPIN value [0, 1].
    pub fn update(&mut self, trade_price: f64, prev_price: f64, volume: f64) -> f64 {
        // Tick rule: if trade > prev → buy, if trade < prev → sell
        let is_buy = trade_price >= prev_price;

        if is_buy {
            self.buy_volume += volume;
        } else {
            self.sell_volume += volume;
        }
        self.current_bucket_volume += volume;

        // Check if bucket is full
        if self.current_bucket_volume >= self.bucket_volume {
            self.buckets.push_back((self.buy_volume, self.sell_volume));
            if self.buckets.len() > self.num_buckets {
                self.buckets.pop_front();
            }

            // Reset for next bucket
            self.buy_volume = 0.0;
            self.sell_volume = 0.0;
            self.current_bucket_volume = 0.0;
        }

        self.compute_vpin()
    }

    fn compute_vpin(&self) -> f64 {
        if self.buckets.is_empty() {
            return 0.0;
        }

        let mut total_imbalance = 0.0;
        let mut total_volume = 0.0;

        for (buy, sell) in &self.buckets {
            total_imbalance += (buy - sell).abs();
            total_volume += buy + sell;
        }

        if total_volume < 1e-10 {
            return 0.0;
        }

        (total_imbalance / total_volume).clamp(0.0, 1.0)
    }

    /// Current VPIN value [0, 1]. >0.7 is considered toxic.
    pub fn value(&self) -> f64 {
        self.compute_vpin()
    }
}

// ─── Amihud Illiquidity Ratio ────────────────────────────────────

/// Measures market illiquidity: |return| / dollar_volume
///
/// Higher → less liquid (each dollar of volume moves the price more).
/// Used for position sizing: reduce size in illiquid names.
///
/// Reference: Amihud (2002) "Illiquidity and stock returns"
pub struct AmihudIlliquidity {
    ratios: VecDeque<f64>,
    window_size: usize,
}

impl AmihudIlliquidity {
    pub fn new(window_size: usize) -> Self {
        Self {
            ratios: VecDeque::with_capacity(window_size + 1),
            window_size,
        }
    }

    /// Update with daily |return| and dollar volume.
    pub fn update(&mut self, abs_return: f64, dollar_volume: f64) -> f64 {
        if dollar_volume < 1.0 {
            return self.value();
        }

        let ratio = abs_return / dollar_volume;
        self.ratios.push_back(ratio);
        if self.ratios.len() > self.window_size {
            self.ratios.pop_front();
        }

        self.value()
    }

    /// Average Amihud ratio over the window. Multiply by 1e6 for readability.
    pub fn value(&self) -> f64 {
        if self.ratios.is_empty() {
            return 0.0;
        }
        self.ratios.iter().sum::<f64>() / self.ratios.len() as f64
    }
}

// ─── Trade Flow Classifier ──────────────────────────────────────

/// Lee-Ready algorithm: classifies trades as buyer- or seller-initiated.
///
/// 1. If trade > midpoint → buyer-initiated
/// 2. If trade < midpoint → seller-initiated
/// 3. If trade == midpoint → use tick rule (compare to previous trade)
pub struct TradeClassifier {
    prev_trade_price: f64,
    initialized: bool,
}

impl TradeClassifier {
    pub fn new() -> Self {
        Self {
            prev_trade_price: 0.0,
            initialized: false,
        }
    }

    /// Returns +1.0 for buy, -1.0 for sell.
    pub fn classify(&mut self, trade_price: f64, bid: f64, ask: f64) -> f64 {
        let mid = (bid + ask) / 2.0;

        let sign = if trade_price > mid {
            1.0 // Buyer-initiated
        } else if trade_price < mid {
            -1.0 // Seller-initiated
        } else if self.initialized {
            // Tick rule
            if trade_price > self.prev_trade_price {
                1.0
            } else if trade_price < self.prev_trade_price {
                -1.0
            } else {
                0.0 // Indeterminate
            }
        } else {
            0.0
        };

        self.prev_trade_price = trade_price;
        self.initialized = true;
        sign
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_microprice_buying_pressure() {
        // Bid size >> ask size → microprice should be closer to ask
        let mp = microprice(100.0, 101.0, 1000.0, 100.0);
        let mid = 100.5;
        assert!(mp > mid, "Microprice {} should be > mid {}", mp, mid);
    }

    #[test]
    fn test_microprice_equal_sizes() {
        // Equal sizes → microprice == midpoint
        let mp = microprice(100.0, 102.0, 500.0, 500.0);
        assert!((mp - 101.0).abs() < 0.001);
    }

    #[test]
    fn test_ofi_buying_pressure() {
        let mut ofi = OrderFlowImbalance::new(10);

        // Initialize
        ofi.update(100.0, 500.0, 101.0, 500.0);

        // Bid size increases, ask size decreases → buying pressure
        let val = ofi.update(100.0, 800.0, 101.0, 200.0);
        assert!(val > 0.0, "OFI should be positive for buying pressure: {}", val);
    }

    #[test]
    fn test_ofi_selling_pressure() {
        let mut ofi = OrderFlowImbalance::new(10);

        ofi.update(100.0, 500.0, 101.0, 500.0);

        // Ask size increases, bid size decreases → selling pressure
        let val = ofi.update(100.0, 200.0, 101.0, 800.0);
        assert!(val < 0.0, "OFI should be negative for selling pressure: {}", val);
    }

    #[test]
    fn test_kyle_lambda_positive() {
        let mut kyle = KyleLambda::new(50);

        // Simulate: large buys cause price to go up
        for i in 0..50 {
            let buy_pressure = 100.0 + i as f64 * 0.5; // Consistent buy flow
            let mid = 100.0 + i as f64 * 0.01; // Price gradually rises
            kyle.update(mid, buy_pressure);
        }

        let lambda = kyle.lambda();
        assert!(lambda >= 0.0, "Lambda should be non-negative: {}", lambda);
    }

    #[test]
    fn test_vpin_zero_on_balanced_flow() {
        let mut vpin = Vpin::new(1000.0, 10);

        // Perfectly balanced flow → VPIN should be low
        for i in 0..100 {
            let price = 100.0 + if i % 2 == 0 { 0.01 } else { -0.01 };
            let prev = if i % 2 == 0 { 99.99 } else { 100.01 };
            vpin.update(price, prev, 100.0);
        }

        let val = vpin.value();
        assert!(val < 0.5, "Balanced flow VPIN should be low: {}", val);
    }

    #[test]
    fn test_vpin_high_on_one_sided_flow() {
        let mut vpin = Vpin::new(500.0, 10);

        // All buys → VPIN should be high
        for _ in 0..100 {
            vpin.update(100.01, 100.0, 100.0); // All upticks = buys
        }

        let val = vpin.value();
        assert!(val > 0.5, "One-sided flow VPIN should be high: {}", val);
    }

    #[test]
    fn test_amihud_illiquidity() {
        let mut amihud = AmihudIlliquidity::new(20);

        // Low volume → high illiquidity
        let low_liq = amihud.update(0.02, 100_000.0);

        // High volume → low illiquidity
        let mut amihud2 = AmihudIlliquidity::new(20);
        let high_liq = amihud2.update(0.02, 100_000_000.0);

        assert!(low_liq > high_liq, "Low volume should be more illiquid");
    }

    #[test]
    fn test_trade_classifier() {
        let mut classifier = TradeClassifier::new();

        // Trade above midpoint → buy
        let sign = classifier.classify(100.75, 100.0, 101.0);
        assert!((sign - 1.0).abs() < 0.01);

        // Trade below midpoint → sell
        let sign = classifier.classify(100.25, 100.0, 101.0);
        assert!((sign + 1.0).abs() < 0.01);
    }
}
