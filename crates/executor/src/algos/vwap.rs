use common::Action;

pub struct VwapAlgo {
    pub total_size: f64,
    pub token: String,
    pub volume_profile: Vec<f64>, // Historic volume % per time bin
}

impl VwapAlgo {
    pub fn new(token: String, total_size: f64, volume_profile: Vec<f64>) -> Self {
        Self {
            token,
            total_size,
            volume_profile,
        }
    }

    /// Calculates trade sizes based on expected historical volume distribution
    pub fn generate_slices(&self) -> Vec<Action> {
        let mut schedule = Vec::new();
        for vp in &self.volume_profile {
            let slice_size = self.total_size * vp;
            schedule.push(Action::Buy {
                token: self.token.clone(),
                size: slice_size,
                confidence: 0.9,
            });
        }
        schedule
    }
}
