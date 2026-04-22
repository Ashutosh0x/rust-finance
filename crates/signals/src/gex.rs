// crates/signals/src/gex.rs
// Gamma Exposure (GEX) — maps dealer gamma at every strike
// Identifies price magnets, pin risk zones, and volatility flip points
// Positive GEX = dealers hedge by selling rallies/buying dips (vol dampening)
// Negative GEX = dealers hedge by buying rallies/selling dips (vol amplifying)

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct OptionsContract {
    pub symbol: String,
    pub expiry: String, // "2026-03-21"
    pub strike: f64,
    pub contract_type: ContractType,
    /// Gamma per contract (from options chain data)
    pub gamma: f64,
    /// Open interest in contracts
    pub open_interest: u64,
    /// 100 shares per contract standard
    pub multiplier: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContractType {
    Call,
    Put,
}

/// GEX at a specific strike price
#[derive(Debug, Clone)]
pub struct StrikeGex {
    pub strike: f64,
    /// Net GEX in USD (calls - puts)
    pub net_gex_usd: f64,
    /// Call GEX contribution
    pub call_gex_usd: f64,
    /// Put GEX contribution (negative by convention for puts)
    pub put_gex_usd: f64,
    /// Magnitude — visual bar height
    pub magnitude: f64,
}

#[derive(Debug, Clone)]
pub struct GexSurface {
    pub symbol: String,
    pub spot_price: f64,
    pub strikes: Vec<StrikeGex>,
    /// Total net GEX across all strikes (positive = dampening regime)
    pub total_net_gex: f64,
    /// Strike where GEX flips from positive to negative going DOWN
    pub flip_point: Option<f64>,
    /// Largest positive GEX strike = price magnet (pin risk)
    pub max_positive_strike: Option<f64>,
    /// Largest negative GEX strike = vol acceleration zone
    pub max_negative_strike: Option<f64>,
    /// Gamma regime
    pub regime: GammaRegime,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GammaRegime {
    /// Total net GEX > 0 — dealers are long gamma, market self-stabilizes
    PositiveGamma,
    /// Total net GEX < 0 — dealers are short gamma, market is unstable
    NegativeGamma,
    /// Near zero — transition zone
    Neutral,
}

pub struct GexCalculator;

impl GexCalculator {
    /// Compute full GEX surface from an options chain
    /// spot_price: current underlying price
    pub fn compute(symbol: &str, spot_price: f64, contracts: &[OptionsContract]) -> GexSurface {
        let mut by_strike: HashMap<u64, (f64, f64)> = HashMap::new(); // strike_cents → (call_gex, put_gex)

        for contract in contracts {
            if contract.symbol != symbol {
                continue;
            }

            // GEX = gamma × open_interest × multiplier × spot² × 0.01
            // The spot² × 0.01 converts gamma (per 1% move) to dollar gamma
            let gex = contract.gamma
                * contract.open_interest as f64
                * contract.multiplier
                * spot_price
                * spot_price
                * 0.01;

            let strike_key = (contract.strike * 100.0) as u64;
            let entry = by_strike.entry(strike_key).or_insert((0.0, 0.0));

            match contract.contract_type {
                ContractType::Call => entry.0 += gex, // calls: dealers long gamma (positive)
                ContractType::Put => entry.1 -= gex, // puts: dealers short gamma (negative by convention)
            }
        }

        let mut strikes: Vec<StrikeGex> = by_strike
            .iter()
            .map(|(k, (call_gex, put_gex))| {
                let strike = *k as f64 / 100.0;
                let net = call_gex + put_gex;
                StrikeGex {
                    strike,
                    net_gex_usd: net,
                    call_gex_usd: *call_gex,
                    put_gex_usd: *put_gex,
                    magnitude: net.abs(),
                }
            })
            .collect();

        strikes.sort_by(|a, b| a.strike.partial_cmp(&b.strike).unwrap());

        let total_net_gex: f64 = strikes.iter().map(|s| s.net_gex_usd).sum();

        // Find GEX flip point: highest strike below spot where GEX goes negative
        let flip_point = Self::find_flip_point(&strikes, spot_price);

        // Price magnets: largest positive and negative GEX strikes
        let max_positive_strike = strikes
            .iter()
            .filter(|s| s.net_gex_usd > 0.0)
            .max_by(|a, b| a.net_gex_usd.partial_cmp(&b.net_gex_usd).unwrap())
            .map(|s| s.strike);

        let max_negative_strike = strikes
            .iter()
            .filter(|s| s.net_gex_usd < 0.0)
            .min_by(|a, b| a.net_gex_usd.partial_cmp(&b.net_gex_usd).unwrap())
            .map(|s| s.strike);

        let regime = if total_net_gex > 500_000_000.0 {
            GammaRegime::PositiveGamma
        } else if total_net_gex < -500_000_000.0 {
            GammaRegime::NegativeGamma
        } else {
            GammaRegime::Neutral
        };

        tracing::info!(
            symbol, spot_price, total_net_gex,
            regime = ?regime,
            flip = flip_point,
            "GEX surface computed"
        );

        GexSurface {
            symbol: symbol.to_string(),
            spot_price,
            strikes,
            total_net_gex,
            flip_point,
            max_positive_strike,
            max_negative_strike,
            regime,
        }
    }

    /// Find the flip point: the strike below spot where cumulative GEX turns negative
    fn find_flip_point(strikes: &[StrikeGex], spot: f64) -> Option<f64> {
        // Look at strikes below spot, scan upward
        let below_spot: Vec<&StrikeGex> = strikes.iter().filter(|s| s.strike <= spot).collect();
        let mut cumulative = 0.0;
        let mut last_positive_strike: Option<f64> = None;

        for s in below_spot.iter().rev() {
            cumulative += s.net_gex_usd;
            if cumulative > 0.0 && last_positive_strike.is_none() {
                last_positive_strike = Some(s.strike);
            } else if cumulative < 0.0 && last_positive_strike.is_some() {
                return last_positive_strike;
            }
        }
        last_positive_strike
    }

    /// Summarize key levels for TUI display
    pub fn key_levels(surface: &GexSurface) -> Vec<(f64, String)> {
        let mut levels = Vec::new();
        if let Some(f) = surface.flip_point {
            levels.push((f, "GEX flip".to_string()));
        }
        if let Some(p) = surface.max_positive_strike {
            levels.push((p, "Pin magnet".to_string()));
        }
        if let Some(n) = surface.max_negative_strike {
            levels.push((n, "Vol accelerator".to_string()));
        }
        levels.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        levels
    }
}
