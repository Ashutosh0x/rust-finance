// ═══════════════════════════════════════════════════════════════════════════════
// Volatility Regime Detection
//
// Classifies current market conditions based on GARCH annualized volatility.
// Used to adapt agent behavior, position sizing, and risk thresholds.
// ═══════════════════════════════════════════════════════════════════════════════

use serde::{Deserialize, Serialize};

/// Volatility regime classification based on annualized vol.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VolRegime {
    Low,    // < 15% annualized — calm, momentum strategies work well
    Normal, // 15-30% — typical equity market conditions
    High,   // 30-50% — elevated risk, reduce exposure
    Crisis, // > 50% — extreme stress, minimal exposure
}

impl VolRegime {
    /// Classify regime from annualized vol percentage (e.g. 0.25 = 25%).
    pub fn from_annualized_vol(ann_vol: f64) -> Self {
        let vol_pct = ann_vol * 100.0;
        match vol_pct {
            v if v < 15.0 => Self::Low,
            v if v < 30.0 => Self::Normal,
            v if v < 50.0 => Self::High,
            _ => Self::Crisis,
        }
    }

    /// Position size multiplier per regime.
    /// Scales position sizes inversely with volatility.
    pub fn position_scale(&self) -> f64 {
        match self {
            Self::Low => 1.5, // lever up in calm
            Self::Normal => 1.0,
            Self::High => 0.5,   // reduce exposure
            Self::Crisis => 0.2, // minimal exposure
        }
    }

    /// Momentum faction weight per regime.
    /// Momentum strategies work best in low-vol, trending markets.
    pub fn momentum_weight(&self) -> f64 {
        match self {
            Self::Low => 0.50, // momentum works in calm markets
            Self::Normal => 0.40,
            Self::High => 0.20,   // mean reversion dominates
            Self::Crisis => 0.10, // pure risk management
        }
    }

    /// Mean-reversion faction weight per regime.
    pub fn mean_reversion_weight(&self) -> f64 {
        match self {
            Self::Low => 0.20,
            Self::Normal => 0.30,
            Self::High => 0.50,
            Self::Crisis => 0.30,
        }
    }

    /// Risk management strength per regime.
    pub fn risk_mgmt_weight(&self) -> f64 {
        match self {
            Self::Low => 0.10,
            Self::Normal => 0.20,
            Self::High => 0.40,
            Self::Crisis => 0.70,
        }
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Low => "LOW VOL",
            Self::Normal => "NORMAL",
            Self::High => "HIGH VOL",
            Self::Crisis => "CRISIS",
        }
    }

    /// Emoji indicator for terminal output.
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Low => "🟢",
            Self::Normal => "🟡",
            Self::High => "🟠",
            Self::Crisis => "🔴",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regime_classification() {
        assert_eq!(VolRegime::from_annualized_vol(0.10), VolRegime::Low);
        assert_eq!(VolRegime::from_annualized_vol(0.20), VolRegime::Normal);
        assert_eq!(VolRegime::from_annualized_vol(0.40), VolRegime::High);
        assert_eq!(VolRegime::from_annualized_vol(0.60), VolRegime::Crisis);
    }

    #[test]
    fn test_position_scale_monotonically_decreasing() {
        let regimes = [
            VolRegime::Low,
            VolRegime::Normal,
            VolRegime::High,
            VolRegime::Crisis,
        ];
        for i in 0..regimes.len() - 1 {
            assert!(
                regimes[i].position_scale() >= regimes[i + 1].position_scale(),
                "{:?} scale should >= {:?} scale",
                regimes[i],
                regimes[i + 1]
            );
        }
    }

    #[test]
    fn test_risk_weight_increases_with_vol() {
        assert!(VolRegime::Crisis.risk_mgmt_weight() > VolRegime::Low.risk_mgmt_weight());
    }
}
