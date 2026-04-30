// crates/execution/src/almgren_chriss.rs
//
// Almgren-Chriss Optimal Execution Model
//
// Source: Almgren & Chriss (2000/2001) — "Optimal Execution of Portfolio
//         Transactions" — the gold standard for optimal execution.
//         arXiv 2507.06345 (Jan 2026 updated) — RL for trade execution validates
//         AC as the baseline that RL agents learn to approximate.
//
// The model minimizes:
//   E[cost] + λ × Var[cost]
//
// where cost includes:
//   - Permanent impact: g(v) = γ × v (price moves permanently per unit traded)
//   - Temporary impact: h(v) = ε × sgn(v) + η × v (immediate execution cost)
//   - Timing risk: σ × √(Σ xⱼ²) (variance of remaining position exposure)
//
// Optimal trajectory (closed form for linear impact):
//   xⱼ = X × sinh(κ(T-tⱼ)) / sinh(κT)
//   where κ = √(λσ² / η) is the "urgency" parameter
//
// Key insight: higher κ → trade faster (front-load), lower κ → trade evenly (TWAP).
// When λ=0 (risk-neutral), the optimal trajectory IS TWAP.
// When λ→∞ (infinitely risk-averse), execute immediately.

/// Configuration for the Almgren-Chriss model.
#[derive(Debug, Clone)]
pub struct AlmgrenChrissConfig {
    /// Risk aversion parameter λ.
    /// Higher = more urgency to execute quickly.
    /// Typical values: 1e-6 to 1e-4.
    pub risk_aversion: f64,
    /// Daily volatility σ (as a decimal, e.g., 0.02 for 2%).
    pub sigma: f64,
    /// Permanent impact coefficient γ.
    /// Price moves permanently by γ × (quantity/ADV) per unit traded.
    /// Typical: 0.1 to 0.5 for liquid stocks.
    pub gamma_permanent: f64,
    /// Temporary impact coefficient η.
    /// Immediate price impact per unit of trading rate.
    /// Typical: 0.01 to 0.1.
    pub eta_temporary: f64,
    /// Fixed cost per trade (spread cost) ε.
    /// In price units (e.g., half-spread).
    pub epsilon_fixed: f64,
    /// Average daily volume (for impact normalization).
    pub adv: f64,
}

impl AlmgrenChrissConfig {
    /// Default for a liquid equity / crypto asset.
    pub fn liquid_default(sigma: f64, adv: f64) -> Self {
        Self {
            risk_aversion: 1e-5,
            sigma,
            gamma_permanent: 0.1,
            eta_temporary: 0.05,
            epsilon_fixed: 0.01,
            adv,
        }
    }

    /// Aggressive execution preset (high urgency).
    pub fn aggressive(sigma: f64, adv: f64) -> Self {
        Self {
            risk_aversion: 1e-3,
            ..Self::liquid_default(sigma, adv)
        }
    }

    /// Passive execution preset (minimize impact, accept timing risk).
    pub fn passive(sigma: f64, adv: f64) -> Self {
        Self {
            risk_aversion: 1e-7,
            ..Self::liquid_default(sigma, adv)
        }
    }
}

/// A single slice in the optimal execution trajectory.
#[derive(Debug, Clone)]
pub struct ExecutionSlice {
    /// Time index (0-based).
    pub time_idx: usize,
    /// Fraction of time elapsed [0, 1].
    pub time_frac: f64,
    /// Quantity to execute in this slice.
    pub quantity: f64,
    /// Cumulative quantity executed so far.
    pub cumulative: f64,
    /// Remaining quantity after this slice.
    pub remaining: f64,
    /// Estimated cost of this slice (in price units × quantity).
    pub estimated_cost: f64,
}

/// Result of the Almgren-Chriss optimization.
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    /// Total quantity to execute.
    pub total_quantity: f64,
    /// Number of time slices.
    pub num_slices: usize,
    /// Urgency parameter κ.
    pub kappa: f64,
    /// Individual execution slices.
    pub slices: Vec<ExecutionSlice>,
    /// Expected total cost (in price units × total_quantity).
    pub expected_cost: f64,
    /// Variance of total cost.
    pub cost_variance: f64,
    /// Implementation shortfall (expected, in bps).
    pub expected_is_bps: f64,
}

/// Almgren-Chriss optimal execution engine.
///
/// Computes the optimal trading trajectory that minimizes the expected cost
/// plus risk-aversion-weighted variance.
///
/// Usage:
/// ```ignore
/// let config = AlmgrenChrissConfig::liquid_default(0.02, 1_000_000.0);
/// let engine = AlmgrenChrissEngine::new(config);
///
/// // Execute 10,000 shares over 20 time slices
/// let plan = engine.compute_trajectory(10_000.0, 20);
///
/// for slice in &plan.slices {
///     println!("t={}: trade {} units", slice.time_idx, slice.quantity);
/// }
/// ```
pub struct AlmgrenChrissEngine {
    config: AlmgrenChrissConfig,
}

impl AlmgrenChrissEngine {
    pub fn new(config: AlmgrenChrissConfig) -> Self {
        Self { config }
    }

    /// Compute the optimal execution trajectory.
    ///
    /// `total_qty`: total quantity to execute (positive for buy, negative for sell)
    /// `num_slices`: number of time buckets to divide execution into
    pub fn compute_trajectory(&self, total_qty: f64, num_slices: usize) -> ExecutionPlan {
        let c = &self.config;
        let n = num_slices.max(1);
        let abs_qty = total_qty.abs();

        if abs_qty < 1e-10 || n == 0 {
            return ExecutionPlan {
                total_quantity: total_qty,
                num_slices: 0,
                kappa: 0.0,
                slices: vec![],
                expected_cost: 0.0,
                cost_variance: 0.0,
                expected_is_bps: 0.0,
            };
        }

        // Compute urgency parameter κ
        // κ = √(λσ² / η)
        // Higher λ or σ → trade faster (front-load)
        // Higher η (impact) → trade slower (spread out)
        let kappa = if c.eta_temporary > 1e-15 {
            (c.risk_aversion * c.sigma * c.sigma / c.eta_temporary).sqrt()
        } else {
            0.0 // Degenerate: no temporary impact → TWAP
        };

        let mut slices = Vec::with_capacity(n);
        let mut cumulative = 0.0;
        let sign = total_qty.signum();

        // Optimal trajectory: xⱼ = X × sinh(κ(T-tⱼ)) / sinh(κT)
        // For discrete time: compute fraction at each step
        let sinh_kt = (kappa * n as f64).sinh();

        for j in 0..n {
            let t_frac = j as f64 / n as f64;

            let trade_qty = if kappa.abs() < 1e-10 || sinh_kt.abs() < 1e-10 {
                // Risk-neutral or no impact: TWAP (equal slices)
                abs_qty / n as f64
            } else {
                // Almgren-Chriss optimal discrete trajectory:
                // Holdings at step j: x_j = X × sinh(κ(N-j)) / sinh(κN)
                // Trade at step j:    n_j = x_j - x_{j+1}
                let remaining_j = (n - j) as f64;
                let remaining_j1 = (n - j - 1) as f64;

                let x_j = abs_qty * (kappa * remaining_j).sinh() / sinh_kt;
                let x_j1 = if j + 1 < n {
                    abs_qty * (kappa * remaining_j1).sinh() / sinh_kt
                } else {
                    0.0 // Last step: liquidate remaining
                };

                (x_j - x_j1).max(0.0)
            };

            let signed_qty = trade_qty * sign;
            cumulative += trade_qty;

            // Estimated cost for this slice
            let trade_rate = trade_qty / c.adv.max(1.0);
            let temp_impact = c.epsilon_fixed + c.eta_temporary * trade_rate;
            let perm_impact = c.gamma_permanent * trade_rate;
            let slice_cost = trade_qty * (temp_impact + perm_impact / 2.0);

            slices.push(ExecutionSlice {
                time_idx: j,
                time_frac: t_frac,
                quantity: signed_qty,
                cumulative,
                remaining: abs_qty - cumulative,
                estimated_cost: slice_cost,
            });
        }

        // Normalize: ensure cumulative equals total (floating point cleanup)
        if let Some(last) = slices.last_mut() {
            let adjustment = abs_qty - cumulative;
            last.quantity += adjustment * sign;
            last.remaining = 0.0;
        }

        // Total expected cost
        let expected_cost: f64 = slices.iter().map(|s| s.estimated_cost).sum();

        // Cost variance (simplified: proportional to remaining position exposure)
        let exposure_sum: f64 = slices.iter().map(|s| s.remaining.powi(2)).sum();
        let cost_variance = c.sigma * c.sigma * exposure_sum / (n as f64);

        // Implementation shortfall in bps
        let notional = abs_qty * 1.0; // Assume unit price for bps calc
        let expected_is_bps = if notional > 1e-10 {
            expected_cost / notional * 10_000.0
        } else {
            0.0
        };

        ExecutionPlan {
            total_quantity: total_qty,
            num_slices: n,
            kappa,
            slices,
            expected_cost,
            cost_variance,
            expected_is_bps,
        }
    }

    /// Square-root market impact model (Almgren 2005 empirical).
    ///
    /// impact = σ × η × √(qty / ADV)
    ///
    /// This is the simplified, empirically-validated version used in practice.
    pub fn sqrt_impact(&self, qty: f64) -> f64 {
        let c = &self.config;
        if c.adv < 1e-10 {
            return 0.0;
        }
        c.sigma * c.eta_temporary * (qty.abs() / c.adv).sqrt()
    }

    /// Estimate optimal number of slices for a given quantity.
    ///
    /// Rule of thumb: participation rate should be ~5-15% of ADV per slice.
    /// More slices = less impact per slice but more timing risk.
    pub fn optimal_num_slices(&self, qty: f64, target_participation: f64) -> usize {
        let c = &self.config;
        if c.adv < 1e-10 || target_participation < 1e-10 {
            return 1;
        }
        let qty_per_slice = c.adv * target_participation;
        let n = (qty.abs() / qty_per_slice).ceil() as usize;
        n.max(1).min(1000)
    }

    /// Update config with new volatility (e.g., from GARCH).
    pub fn update_sigma(&mut self, new_sigma: f64) {
        self.config.sigma = new_sigma;
    }

    /// Update config with new risk aversion (e.g., from regime detector).
    pub fn update_risk_aversion(&mut self, new_lambda: f64) {
        self.config.risk_aversion = new_lambda;
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_twap_when_risk_neutral() {
        let config = AlmgrenChrissConfig {
            risk_aversion: 0.0, // Risk-neutral → TWAP
            sigma: 0.02,
            gamma_permanent: 0.1,
            eta_temporary: 0.05,
            epsilon_fixed: 0.01,
            adv: 1_000_000.0,
        };
        let engine = AlmgrenChrissEngine::new(config);
        let plan = engine.compute_trajectory(10_000.0, 10);

        assert_eq!(plan.slices.len(), 10);

        // All slices should be approximately equal (TWAP)
        for slice in &plan.slices {
            let expected = 10_000.0 / 10.0;
            assert!((slice.quantity - expected).abs() < 1.0,
                "TWAP slice should be ~{}: got {}", expected, slice.quantity);
        }
    }

    #[test]
    fn test_urgent_frontloads() {
        let config = AlmgrenChrissConfig {
            risk_aversion: 1e-2, // Very urgent
            sigma: 0.02,
            gamma_permanent: 0.1,
            eta_temporary: 0.05,
            epsilon_fixed: 0.01,
            adv: 1_000_000.0,
        };
        let engine = AlmgrenChrissEngine::new(config);
        let plan = engine.compute_trajectory(10_000.0, 10);

        // First slice should be larger than last slice (front-loaded)
        assert!(plan.slices[0].quantity > plan.slices[9].quantity,
            "Urgent execution should front-load: first={}, last={}",
            plan.slices[0].quantity, plan.slices[9].quantity);
    }

    #[test]
    fn test_total_quantity_conserved() {
        let config = AlmgrenChrissConfig::liquid_default(0.02, 1_000_000.0);
        let engine = AlmgrenChrissEngine::new(config);
        let plan = engine.compute_trajectory(10_000.0, 20);

        let total: f64 = plan.slices.iter().map(|s| s.quantity).sum();
        assert!((total - 10_000.0).abs() < 1.0,
            "Total traded should equal input: {}", total);
    }

    #[test]
    fn test_sell_order_negative() {
        let config = AlmgrenChrissConfig::liquid_default(0.02, 1_000_000.0);
        let engine = AlmgrenChrissEngine::new(config);
        let plan = engine.compute_trajectory(-5_000.0, 10);

        for slice in &plan.slices {
            assert!(slice.quantity <= 0.0,
                "Sell order slices should be negative: {}", slice.quantity);
        }
    }

    #[test]
    fn test_sqrt_impact_scales() {
        let config = AlmgrenChrissConfig::liquid_default(0.02, 1_000_000.0);
        let engine = AlmgrenChrissEngine::new(config);

        let small = engine.sqrt_impact(1_000.0);
        let large = engine.sqrt_impact(100_000.0);

        assert!(large > small, "Larger orders should have more impact");
        // 100x quantity → 10x impact (square root)
        let ratio = large / small;
        assert!((ratio - 10.0).abs() < 0.1,
            "Impact should scale as sqrt: ratio={}", ratio);
    }

    #[test]
    fn test_expected_cost_positive() {
        let config = AlmgrenChrissConfig::liquid_default(0.02, 1_000_000.0);
        let engine = AlmgrenChrissEngine::new(config);
        let plan = engine.compute_trajectory(10_000.0, 20);

        assert!(plan.expected_cost > 0.0, "Expected cost should be positive");
        assert!(plan.expected_is_bps > 0.0, "Expected IS should be positive");
    }

    #[test]
    fn test_kappa_increases_with_urgency() {
        let passive = AlmgrenChrissConfig::passive(0.02, 1_000_000.0);
        let aggressive = AlmgrenChrissConfig::aggressive(0.02, 1_000_000.0);

        let eng_p = AlmgrenChrissEngine::new(passive);
        let eng_a = AlmgrenChrissEngine::new(aggressive);

        let plan_p = eng_p.compute_trajectory(10_000.0, 20);
        let plan_a = eng_a.compute_trajectory(10_000.0, 20);

        assert!(plan_a.kappa > plan_p.kappa,
            "Aggressive should have higher kappa: agg={}, pass={}", plan_a.kappa, plan_p.kappa);
    }
}
