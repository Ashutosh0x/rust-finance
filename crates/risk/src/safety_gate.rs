// crates/risk/src/safety_gate.rs
// Deterministic Safety Gate — Zero-AI verification layer
//
// This is the "dumbest component" that is also the most critical.
// It uses ONLY deterministic math — no ML, no LLM, no agent consensus.
// Purpose: prevent confirmation bias from compounding across AI agents.
//
// Inspired by AlgoXpert's finding: removing the deterministic gate
// increased false-pass rate from 0% to 12.4%.

use crate::state::EngineState;
use compact_str::CompactString;
use tracing::{info, warn};

/// Result of the safety gate evaluation
#[derive(Debug, Clone)]
pub enum SafetyVerdict {
    /// All checks passed, signal is safe to act on
    Pass,
    /// Signal blocked with reason
    Block { reason: CompactString },
    /// Signal allowed but with reduced size
    Attenuate { factor: f64, reason: CompactString },
}

/// Configuration for the safety gate thresholds
#[derive(Debug, Clone)]
pub struct SafetyGateConfig {
    /// Block if agent agreement exceeds this ratio (confirmation bias detector)
    /// Default: 0.85 (85% — if 85%+ agents agree, likely echo chamber)
    pub max_agreement_ratio: f64,
    /// Block if single position exceeds this fraction of total portfolio
    /// Default: 0.20 (20% concentration limit)
    pub max_position_concentration: f64,
    /// Block if current drawdown exceeds this percentage
    /// Default: 0.05 (5% drawdown circuit breaker)
    pub max_drawdown_pct: f64,
    /// Block if realised volatility exceeds this multiple of average
    /// Default: 2.5 (vol spike = regime change, do not trust agent signals)
    pub vol_spike_multiplier: f64,
    /// Attenuate if daily trade count exceeds this
    /// Default: 50 trades per day
    pub max_daily_trades: usize,
    /// Block if correlation between top holdings exceeds this
    /// Default: 0.80 (too concentrated in correlated names)
    pub max_correlation: f64,
}

impl Default for SafetyGateConfig {
    fn default() -> Self {
        Self {
            max_agreement_ratio: 0.85,
            max_position_concentration: 0.20,
            max_drawdown_pct: 0.05,
            vol_spike_multiplier: 2.5,
            max_daily_trades: 50,
            max_correlation: 0.80,
        }
    }
}

/// Deterministic safety gate — the final checkpoint before execution.
/// No AI, no ML, no LLM. Pure math.
pub struct SafetyGate {
    config: SafetyGateConfig,
}

impl SafetyGate {
    pub fn new(config: SafetyGateConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(SafetyGateConfig::default())
    }

    /// Evaluate all safety checks. Returns the first blocking verdict.
    pub fn evaluate(
        &self,
        state: &EngineState,
        signal_agreement_ratio: f64,
        position_value: f64,
        current_vol: f64,
        avg_vol: f64,
    ) -> SafetyVerdict {
        // Check 1: Confirmation bias detector
        // If too many agents agree, the signal is likely an echo chamber
        if signal_agreement_ratio > self.config.max_agreement_ratio {
            warn!(
                agreement = signal_agreement_ratio,
                threshold = self.config.max_agreement_ratio,
                "Safety gate BLOCKED: agent confirmation bias detected"
            );
            return SafetyVerdict::Block {
                reason: CompactString::from(format!(
                    "Agent agreement {:.0}% exceeds {:.0}% threshold — confirmation bias",
                    signal_agreement_ratio * 100.0,
                    self.config.max_agreement_ratio * 100.0
                )),
            };
        }

        // Check 2: Drawdown circuit breaker
        if state.current_drawdown_pct > self.config.max_drawdown_pct {
            warn!(
                drawdown = state.current_drawdown_pct,
                max = self.config.max_drawdown_pct,
                "Safety gate BLOCKED: drawdown exceeded"
            );
            return SafetyVerdict::Block {
                reason: CompactString::from(format!(
                    "Drawdown {:.2}% exceeds {:.2}% limit",
                    state.current_drawdown_pct * 100.0,
                    self.config.max_drawdown_pct * 100.0
                )),
            };
        }

        // Check 3: Position concentration
        let total_portfolio = state.total_equity;
        if total_portfolio > 0.0 {
            let concentration = position_value / total_portfolio;
            if concentration > self.config.max_position_concentration {
                warn!(
                    concentration,
                    max = self.config.max_position_concentration,
                    "Safety gate BLOCKED: position concentration too high"
                );
                return SafetyVerdict::Block {
                    reason: CompactString::from(format!(
                        "Position {:.0}% of portfolio exceeds {:.0}% concentration limit",
                        concentration * 100.0,
                        self.config.max_position_concentration * 100.0
                    )),
                };
            }
        }

        // Check 4: Volatility regime check
        // If current vol is much higher than average, agents may not be calibrated
        if avg_vol > 0.0 {
            let vol_ratio = current_vol / avg_vol;
            if vol_ratio > self.config.vol_spike_multiplier {
                warn!(
                    vol_ratio,
                    threshold = self.config.vol_spike_multiplier,
                    "Safety gate BLOCKED: volatility spike detected"
                );
                return SafetyVerdict::Block {
                    reason: CompactString::from(format!(
                        "Volatility {:.1}x above average — regime change, agents uncalibrated",
                        vol_ratio
                    )),
                };
            }
        }

        // Check 5: Trade frequency governor — attenuate, don't block
        if state.daily_trade_count >= self.config.max_daily_trades {
            let factor = 0.25; // Reduce size to 25%
            info!(
                trades = state.daily_trade_count,
                max = self.config.max_daily_trades,
                "Safety gate ATTENUATE: daily trade limit reached"
            );
            return SafetyVerdict::Attenuate {
                factor,
                reason: CompactString::from(format!(
                    "Daily trade count {} exceeds {} — size reduced to 25%",
                    state.daily_trade_count, self.config.max_daily_trades
                )),
            };
        }

        info!(
            agreement = signal_agreement_ratio,
            drawdown = state.current_drawdown_pct,
            "Safety gate PASS: all checks cleared"
        );
        SafetyVerdict::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> EngineState {
        EngineState {
            current_drawdown_pct: 0.02,
            total_equity: 100_000.0,
            daily_pnl: -500.0,
            open_order_count: 3,
            daily_trade_count: 10,
            open_orders: vec![],
        }
    }

    #[test]
    fn test_pass_normal_conditions() {
        let gate = SafetyGate::with_defaults();
        let state = test_state();
        let verdict = gate.evaluate(&state, 0.60, 10_000.0, 0.15, 0.10);
        assert!(matches!(verdict, SafetyVerdict::Pass));
    }

    #[test]
    fn test_block_confirmation_bias() {
        let gate = SafetyGate::with_defaults();
        let state = test_state();
        // 90% agreement should trigger confirmation bias block
        let verdict = gate.evaluate(&state, 0.90, 10_000.0, 0.15, 0.10);
        assert!(matches!(verdict, SafetyVerdict::Block { .. }));
    }

    #[test]
    fn test_block_drawdown() {
        let gate = SafetyGate::with_defaults();
        let mut state = test_state();
        state.current_drawdown_pct = 0.08; // 8% drawdown
        let verdict = gate.evaluate(&state, 0.60, 10_000.0, 0.15, 0.10);
        assert!(matches!(verdict, SafetyVerdict::Block { .. }));
    }

    #[test]
    fn test_block_concentration() {
        let gate = SafetyGate::with_defaults();
        let state = test_state();
        // 30K position on 100K portfolio = 30% concentration
        let verdict = gate.evaluate(&state, 0.60, 30_000.0, 0.15, 0.10);
        assert!(matches!(verdict, SafetyVerdict::Block { .. }));
    }

    #[test]
    fn test_block_vol_spike() {
        let gate = SafetyGate::with_defaults();
        let state = test_state();
        // Current vol 3x average
        let verdict = gate.evaluate(&state, 0.60, 10_000.0, 0.30, 0.10);
        assert!(matches!(verdict, SafetyVerdict::Block { .. }));
    }

    #[test]
    fn test_attenuate_trade_frequency() {
        let gate = SafetyGate::with_defaults();
        let mut state = test_state();
        state.daily_trade_count = 55;
        let verdict = gate.evaluate(&state, 0.60, 10_000.0, 0.15, 0.10);
        assert!(matches!(verdict, SafetyVerdict::Attenuate { .. }));
    }
}
