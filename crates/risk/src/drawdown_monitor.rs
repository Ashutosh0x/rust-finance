use tracing::info;

pub struct DrawdownMonitor {
    peak_equity: f64,
    current_equity: f64,
    max_drawdown_percent: f64,
}

impl DrawdownMonitor {
    pub fn new(initial_equity: f64, max_drawdown_percent: f64) -> Self {
        Self {
            peak_equity: initial_equity,
            current_equity: initial_equity,
            max_drawdown_percent,
        }
    }

    pub fn update_equity(&mut self, equity: f64) {
        if equity > self.peak_equity {
            self.peak_equity = equity;
        }
        self.current_equity = equity;
    }

    pub fn is_drawdown_breached(&self) -> bool {
        let drawdown = (self.peak_equity - self.current_equity) / self.peak_equity;
        if drawdown > self.max_drawdown_percent {
            info!("Max drawdown breached! Peak: {}, Current: {}, Drawdown: {:.2}%", self.peak_equity, self.current_equity, drawdown * 100.0);
            true
        } else {
            false
        }
    }
}
