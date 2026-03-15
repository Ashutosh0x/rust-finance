use common::Action;

/// Proximal Policy Optimization (PPO) Actor-Critic Agent stub
pub struct PpoAgent {
    pub hidden_dim: usize,
    pub learning_rate: f64,
}

impl PpoAgent {
    pub fn new(hidden_dim: usize, learning_rate: f64) -> Self {
        Self { hidden_dim, learning_rate }
    }

    /// Takes market state vector (prices, depths, inventory) + AI sentiment and returns logits
    pub fn forward_actor(&self, _state: &[f64], anthropic_sentiment: Option<f64>) -> Vec<f64> {
        // Bias logits based on Anthropic textual analysis
        let bias = anthropic_sentiment.unwrap_or(0.0);
        // Mock output: [Buy Prob, Sell Prob, Hold Prob] adjusted by AI bias
        vec![0.2 + bias, 0.1 - bias, 0.7] 
    }

    pub fn forward_critic(&self, _state: &[f64]) -> f64 {
        // Expected Value of the state
        0.5
    }

    pub fn select_action(&self, state: &[f64], anthropic_sentiment: Option<f64>) -> Action {
        let logits = self.forward_actor(state, anthropic_sentiment);
        // Simple argmax for deterministic execution
        let max_idx = logits.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;

        match max_idx {
            0 => Action::Buy { token: "SOL".to_string(), size: 1.0, confidence: logits[0] as f32 },
            1 => Action::Sell { token: "SOL".to_string(), size: 1.0, confidence: logits[1] as f32 },
            _ => Action::Hold,
        }
    }
}
