// crates/signals/src/indicators.rs
// Zero-allocation technical indicators for the hot path.
// All functions operate on price slices and return computed values.

/// Exponential Moving Average (EMA)
/// smoothing = 2 / (period + 1)
pub fn ema(prices: &[f64], period: usize) -> Vec<f64> {
    if prices.is_empty() || period == 0 {
        return Vec::new();
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut result = Vec::with_capacity(prices.len());

    // Seed with SMA of first `period` values
    if prices.len() < period {
        return vec![prices.iter().sum::<f64>() / prices.len() as f64];
    }
    let sma: f64 = prices[..period].iter().sum::<f64>() / period as f64;
    for _ in 0..period - 1 {
        result.push(f64::NAN);
    }
    result.push(sma);

    for i in period..prices.len() {
        let prev = result[i - 1];
        let ema_val = prices[i] * k + prev * (1.0 - k);
        result.push(ema_val);
    }
    result
}

/// Simple Moving Average (SMA)
pub fn sma(prices: &[f64], period: usize) -> Vec<f64> {
    if prices.len() < period || period == 0 {
        return Vec::new();
    }
    let mut result = Vec::with_capacity(prices.len() - period + 1);
    let mut sum: f64 = prices[..period].iter().sum();
    result.push(sum / period as f64);
    for i in period..prices.len() {
        sum += prices[i] - prices[i - period];
        result.push(sum / period as f64);
    }
    result
}

/// Relative Strength Index (RSI)
/// Standard 14-period RSI with Wilder's smoothing
pub fn rsi(prices: &[f64], period: usize) -> Vec<f64> {
    if prices.len() < period + 1 || period == 0 {
        return Vec::new();
    }
    let mut gains = Vec::with_capacity(prices.len() - 1);
    let mut losses = Vec::with_capacity(prices.len() - 1);

    for i in 1..prices.len() {
        let change = prices[i] - prices[i - 1];
        if change > 0.0 {
            gains.push(change);
            losses.push(0.0);
        } else {
            gains.push(0.0);
            losses.push(-change);
        }
    }

    // Initial average gain/loss
    let mut avg_gain: f64 = gains[..period].iter().sum::<f64>() / period as f64;
    let mut avg_loss: f64 = losses[..period].iter().sum::<f64>() / period as f64;

    let mut result = Vec::with_capacity(prices.len() - period);

    // First RSI value
    let rs = if avg_loss == 0.0 { 100.0 } else { avg_gain / avg_loss };
    result.push(100.0 - (100.0 / (1.0 + rs)));

    // Subsequent values with Wilder's smoothing
    for i in period..gains.len() {
        avg_gain = (avg_gain * (period as f64 - 1.0) + gains[i]) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + losses[i]) / period as f64;
        let rs = if avg_loss == 0.0 { 100.0 } else { avg_gain / avg_loss };
        result.push(100.0 - (100.0 / (1.0 + rs)));
    }
    result
}

/// MACD (Moving Average Convergence Divergence)
/// Returns (macd_line, signal_line, histogram)
/// Default periods: fast=12, slow=26, signal=9
pub struct MacdResult {
    pub macd_line: Vec<f64>,
    pub signal_line: Vec<f64>,
    pub histogram: Vec<f64>,
}

pub fn macd(prices: &[f64], fast: usize, slow: usize, signal_period: usize) -> Option<MacdResult> {
    if prices.len() < slow {
        return None;
    }
    let ema_fast = ema(prices, fast);
    let ema_slow = ema(prices, slow);

    // MACD line = fast EMA - slow EMA (aligned from slow-1 index)
    let mut macd_line = Vec::with_capacity(prices.len());
    for i in 0..prices.len() {
        if i < slow - 1 || ema_fast[i].is_nan() || ema_slow[i].is_nan() {
            macd_line.push(f64::NAN);
        } else {
            macd_line.push(ema_fast[i] - ema_slow[i]);
        }
    }

    // Signal line = EMA of MACD line (skip NaN values)
    let valid_macd: Vec<f64> = macd_line.iter().filter(|v| !v.is_nan()).copied().collect();
    let signal_ema = ema(&valid_macd, signal_period);

    let mut signal_line = Vec::with_capacity(prices.len());
    let offset = prices.len() - valid_macd.len();
    for _ in 0..offset {
        signal_line.push(f64::NAN);
    }
    for i in 0..valid_macd.len() {
        if i < signal_ema.len() {
            signal_line.push(signal_ema[i]);
        } else {
            signal_line.push(f64::NAN);
        }
    }

    // Histogram = MACD - Signal
    let mut histogram = Vec::with_capacity(prices.len());
    for i in 0..prices.len() {
        if macd_line[i].is_nan() || i >= signal_line.len() || signal_line[i].is_nan() {
            histogram.push(f64::NAN);
        } else {
            histogram.push(macd_line[i] - signal_line[i]);
        }
    }

    Some(MacdResult {
        macd_line,
        signal_line,
        histogram,
    })
}

/// Bollinger Bands
/// Returns (upper, middle, lower)
pub struct BollingerBands {
    pub upper: Vec<f64>,
    pub middle: Vec<f64>,
    pub lower: Vec<f64>,
}

pub fn bollinger_bands(prices: &[f64], period: usize, num_std: f64) -> Option<BollingerBands> {
    if prices.len() < period {
        return None;
    }
    let middle = sma(prices, period);
    let mut upper = Vec::with_capacity(middle.len());
    let mut lower = Vec::with_capacity(middle.len());

    for (i, &mid) in middle.iter().enumerate() {
        let window = &prices[i..i + period];
        let variance: f64 = window.iter().map(|p| (p - mid).powi(2)).sum::<f64>() / period as f64;
        let std_dev = variance.sqrt();
        upper.push(mid + num_std * std_dev);
        lower.push(mid - num_std * std_dev);
    }

    Some(BollingerBands { upper, middle, lower })
}

/// Volume Weighted Average Price (VWAP)
/// prices and volumes must be same length
pub fn vwap(prices: &[f64], volumes: &[f64]) -> Vec<f64> {
    if prices.len() != volumes.len() || prices.is_empty() {
        return Vec::new();
    }
    let mut cumulative_pv = 0.0;
    let mut cumulative_vol = 0.0;
    let mut result = Vec::with_capacity(prices.len());

    for i in 0..prices.len() {
        cumulative_pv += prices[i] * volumes[i];
        cumulative_vol += volumes[i];
        if cumulative_vol > 0.0 {
            result.push(cumulative_pv / cumulative_vol);
        } else {
            result.push(0.0);
        }
    }
    result
}

/// Average True Range (ATR)
/// Requires high, low, close arrays of equal length
pub fn atr(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<f64> {
    if highs.len() != lows.len() || lows.len() != closes.len() || closes.len() < period + 1 {
        return Vec::new();
    }

    let mut true_ranges = Vec::with_capacity(closes.len() - 1);
    for i in 1..closes.len() {
        let hl = highs[i] - lows[i];
        let hc = (highs[i] - closes[i - 1]).abs();
        let lc = (lows[i] - closes[i - 1]).abs();
        true_ranges.push(hl.max(hc).max(lc));
    }

    // First ATR = SMA of first `period` true ranges
    let mut result = Vec::with_capacity(true_ranges.len());
    let first_atr: f64 = true_ranges[..period].iter().sum::<f64>() / period as f64;
    for _ in 0..period - 1 {
        result.push(f64::NAN);
    }
    result.push(first_atr);

    // Wilder's smoothing
    for i in period..true_ranges.len() {
        let prev = result[i - 1];
        let atr_val = (prev * (period as f64 - 1.0) + true_ranges[i]) / period as f64;
        result.push(atr_val);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sma_basic() {
        let prices = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = sma(&prices, 3);
        assert_eq!(result.len(), 3);
        assert!((result[0] - 2.0).abs() < 1e-10);
        assert!((result[1] - 3.0).abs() < 1e-10);
        assert!((result[2] - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_ema_basic() {
        let prices = vec![10.0, 11.0, 12.0, 13.0, 14.0, 15.0];
        let result = ema(&prices, 3);
        assert_eq!(result.len(), prices.len());
        assert!(result[2].is_finite());
    }

    #[test]
    fn test_rsi_basic() {
        // Monotonically increasing prices should give RSI near 100
        let prices: Vec<f64> = (0..30).map(|i| 100.0 + i as f64).collect();
        let result = rsi(&prices, 14);
        assert!(!result.is_empty());
        assert!(result.last().unwrap() > &90.0);
    }

    #[test]
    fn test_rsi_decreasing() {
        // Monotonically decreasing prices should give RSI near 0
        let prices: Vec<f64> = (0..30).map(|i| 200.0 - i as f64).collect();
        let result = rsi(&prices, 14);
        assert!(!result.is_empty());
        assert!(result.last().unwrap() < &10.0);
    }

    #[test]
    fn test_bollinger_bands() {
        let prices: Vec<f64> = (0..20).map(|i| 100.0 + (i as f64).sin() * 5.0).collect();
        let bands = bollinger_bands(&prices, 10, 2.0).unwrap();
        assert_eq!(bands.upper.len(), bands.middle.len());
        assert_eq!(bands.middle.len(), bands.lower.len());
        for i in 0..bands.upper.len() {
            assert!(bands.upper[i] >= bands.middle[i]);
            assert!(bands.middle[i] >= bands.lower[i]);
        }
    }

    #[test]
    fn test_vwap() {
        let prices = vec![100.0, 101.0, 102.0, 103.0];
        let volumes = vec![1000.0, 2000.0, 1500.0, 500.0];
        let result = vwap(&prices, &volumes);
        assert_eq!(result.len(), 4);
        // First VWAP = 100.0
        assert!((result[0] - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_macd() {
        let prices: Vec<f64> = (0..50).map(|i| 100.0 + (i as f64 * 0.5).sin() * 10.0).collect();
        let result = macd(&prices, 12, 26, 9);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.macd_line.len(), prices.len());
    }
}
