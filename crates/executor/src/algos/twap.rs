use common::Action;
use std::time::Duration;

pub struct TwapAlgo {
    pub total_size: f64,
    pub token: String,
    pub duration: Duration,
    pub slice_count: usize,
}

impl TwapAlgo {
    pub fn new(token: String, total_size: f64, duration: Duration, slice_count: usize) -> Self {
        Self {
            token,
            total_size,
            duration,
            slice_count,
        }
    }

    pub fn generate_slices(&self) -> Vec<(Action, Duration)> {
        let size_per_slice = self.total_size / (self.slice_count as f64);
        let interval = self.duration / (self.slice_count as u32);

        let mut schedule = Vec::new();
        for _ in 0..self.slice_count {
            schedule.push((
                Action::Buy {
                    token: self.token.clone(),
                    size: size_per_slice,
                    confidence: 0.9,
                },
                interval,
            ));
        }
        schedule
    }
}
