use rand::seq::SliceRandom;
use rand::thread_rng;

/// Run Monte Carlo simulation on the trade returns to test system robustness
/// Generates `num_simulations` alternate realities by shuffling sequence of returns.
pub fn run_monte_carlo(returns: &[f64], num_simulations: usize) -> Vec<f64> {
    let mut rng = thread_rng();
    let mut final_equities = Vec::with_capacity(num_simulations);
    
    let initial_equity = 10000.0; // Assume 10k start

    for _ in 0..num_simulations {
        let mut sim_returns = returns.to_vec();
        sim_returns.shuffle(&mut rng);
        
        let mut equity = initial_equity;
        for r in sim_returns {
            equity += r; // If returns are raw PnL values. If percentages, equity *= (1.0 + r)
        }
        final_equities.push(equity);
    }
    
    final_equities.sort_by(|a, b| a.partial_cmp(b).unwrap());
    final_equities
}

pub fn calculate_ruin_probability(final_equities: &[f64], ruin_threshold: f64) -> f64 {
    if final_equities.is_empty() {
        return 0.0;
    }
    let ruined = final_equities.iter().filter(|&&e| e <= ruin_threshold).count();
    ruined as f64 / final_equities.len() as f64
}
