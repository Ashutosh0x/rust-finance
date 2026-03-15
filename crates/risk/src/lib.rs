use common::{Action, Result};
use tracing::{warn, info};

pub mod daily_loss_limit;
pub mod drawdown_monitor;
pub mod var;
pub mod pnl_attribution;
pub mod garch;
pub mod kill_switch;

use daily_loss_limit::DailyLossLimit;
use drawdown_monitor::DrawdownMonitor;
use std::sync::Mutex;

pub struct RiskManager {
    max_position_size: f64,
    min_confidence: f32,
    daily_loss_limit: Mutex<DailyLossLimit>,
    drawdown_monitor: Mutex<DrawdownMonitor>,
}

impl RiskManager {
    pub fn new(max_position_size: f64, min_confidence: f32, initial_equity: f64, max_daily_loss: f64, max_drawdown_pct: f64) -> Self {
        Self {
            max_position_size,
            min_confidence,
            daily_loss_limit: Mutex::new(DailyLossLimit::new(max_daily_loss)),
            drawdown_monitor: Mutex::new(DrawdownMonitor::new(initial_equity, max_drawdown_pct)),
        }
    }

    pub fn update_equity_and_pnl(&self, equity: f64, pnl_delta: f64) {
        if let Ok(mut dl) = self.daily_loss_limit.lock() {
            dl.update_pnl(pnl_delta);
        }
        if let Ok(mut dm) = self.drawdown_monitor.lock() {
            dm.update_equity(equity);
        }
    }

    pub fn is_halt_required(&self) -> bool {
        let breached_loss = self.daily_loss_limit.lock().map(|l| l.is_limit_breached()).unwrap_or(false);
        let breached_dd = self.drawdown_monitor.lock().map(|d| d.is_drawdown_breached()).unwrap_or(false);
        breached_loss || breached_dd
    }

    pub fn check_action(&self, action: Action) -> Result<Action> {
        if self.is_halt_required() {
            warn!("Risk: Trading halted globally due to loss or drawdown breaches.");
            return Ok(Action::Hold);
        }

        match action {
            Action::Buy { token, size, confidence } => {
                if confidence < self.min_confidence {
                    warn!("Risk: Buy rejected due to low confidence: {} < {}", confidence, self.min_confidence);
                    return Ok(Action::Hold);
                }

                let adjusted_size = size.min(self.max_position_size);
                if adjusted_size < size {
                    info!("Risk: Buy size capped: {} -> {}", size, adjusted_size);
                }

                Ok(Action::Buy { token, size: adjusted_size, confidence })
            }
            Action::Sell { .. } => Ok(action), // Sells usually allowed by risk unless closing at bad price
            Action::Hold => Ok(Action::Hold),
        }
    }
}
