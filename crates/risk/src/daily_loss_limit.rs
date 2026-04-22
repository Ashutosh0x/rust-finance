use tracing::warn;

pub struct DailyLossLimit {
    max_daily_loss: f64,
    current_pnl: f64,
}

impl DailyLossLimit {
    pub fn new(max_daily_loss: f64) -> Self {
        Self {
            max_daily_loss,
            current_pnl: 0.0,
        }
    }

    pub fn update_pnl(&mut self, pnl_delta: f64) {
        self.current_pnl += pnl_delta;
    }

    pub fn is_limit_breached(&self) -> bool {
        if self.current_pnl < -self.max_daily_loss {
            warn!(
                "Daily loss limit breached! PnL: {}, Limit: -{}",
                self.current_pnl, self.max_daily_loss
            );
            true
        } else {
            false
        }
    }
}
