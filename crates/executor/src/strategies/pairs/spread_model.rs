pub struct SpreadModel {
    pub half_life: f64,
    pub mean: f64,
    pub std_dev: f64,
}

impl SpreadModel {
    pub fn new(half_life: f64, mean: f64, std_dev: f64) -> Self {
        Self {
            half_life,
            mean,
            std_dev,
        }
    }

    /// Calculates the current Z-Score of the spread (Price_A - Hedge_Ratio * Price_B)
    pub fn calculate_z_score(&self, current_spread: f64) -> f64 {
        if self.std_dev == 0.0 {
            return 0.0;
        }
        (current_spread - self.mean) / self.std_dev
    }

    /// Returns true if Z-Score exceeds entry threshold (e.g., 2.0 std devs)
    pub fn is_entry_signal(&self, z_score: f64, threshold: f64) -> bool {
        z_score.abs() > threshold
    }

    /// Returns true if Z-Score crosses 0 (reverted to mean)
    pub fn is_exit_signal(&self, old_z: f64, new_z: f64) -> bool {
        (old_z > 0.0 && new_z <= 0.0) || (old_z < 0.0 && new_z >= 0.0)
    }
}
