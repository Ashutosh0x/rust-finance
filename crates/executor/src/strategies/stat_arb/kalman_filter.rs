/// Returns a Kalman Filter based hedge ratio for pairs trading
pub struct KalmanFilter {
    estimate: f64,
    error_cov: f64,
    process_noise: f64,
    measurement_noise: f64,
}

impl KalmanFilter {
    pub fn new(process_noise: f64, measurement_noise: f64) -> Self {
        Self {
            estimate: 1.0, // Initial hedge ratio guess
            error_cov: 1.0,
            process_noise,
            measurement_noise,
        }
    }

    /// Step the filter with a new observation (e.g., instantaneous price ratio AssetA/AssetB)
    pub fn step(&mut self, observation: f64) -> f64 {
        // Prediction
        let mut predicted_estimate = self.estimate;
        let predicted_error_cov = self.error_cov + self.process_noise;

        // Kalman Gain
        let kalman_gain = predicted_error_cov / (predicted_error_cov + self.measurement_noise);

        // Update
        self.estimate = predicted_estimate + kalman_gain * (observation - predicted_estimate);
        self.error_cov = (1.0 - kalman_gain) * predicted_error_cov;

        self.estimate
    }
}
