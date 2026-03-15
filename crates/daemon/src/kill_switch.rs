use std::sync::atomic::{AtomicBool, Ordering};
use tracing::warn;

pub struct KillSwitch {
    is_halted: AtomicBool,
}

impl KillSwitch {
    pub fn new() -> Self {
        Self {
            is_halted: AtomicBool::new(false),
        }
    }

    pub fn trigger(&self, reason: &str) {
        warn!("KILL SWITCH TRIGGERED: {}", reason);
        self.is_halted.store(true, Ordering::SeqCst);
    }

    pub fn is_halted(&self) -> bool {
        self.is_halted.load(Ordering::SeqCst)
    }
}
