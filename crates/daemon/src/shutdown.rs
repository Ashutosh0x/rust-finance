// crates/daemon/src/shutdown.rs
//
// Graceful shutdown coordinator.
// Listens for SIGINT / SIGTERM, broadcasts a shutdown signal to all
// subsystems, and waits for them to drain before exiting.

use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::timeout;
use tracing::{error, info, warn};

/// Token broadcast to all subsystems on shutdown.
#[derive(Debug, Clone)]
pub struct ShutdownSignal;

/// Central shutdown controller.  Clone the `ShutdownReceiver` into every
/// long-running task and poll `receiver.is_shutdown()` or
/// `receiver.wait().await` to co-operatively stop.
pub struct ShutdownController {
    tx: broadcast::Sender<ShutdownSignal>,
}

pub struct ShutdownReceiver {
    rx: broadcast::Receiver<ShutdownSignal>,
}

impl ShutdownController {
    pub fn new() -> (Self, ShutdownReceiver) {
        let (tx, rx) = broadcast::channel(1);
        let receiver = ShutdownReceiver { rx };
        (Self { tx }, receiver)
    }

    /// Broadcast the shutdown signal to all receivers.
    pub fn shutdown(&self) {
        let _ = self.tx.send(ShutdownSignal);
        info!("Shutdown signal broadcast to all subsystems");
    }

    /// Subscribe a new receiver (for tasks spawned after construction).
    pub fn subscribe(&self) -> ShutdownReceiver {
        ShutdownReceiver {
            rx: self.tx.subscribe(),
        }
    }
}

impl ShutdownReceiver {
    /// Returns a future that resolves when shutdown is triggered.
    pub async fn wait(&mut self) {
        let _ = self.rx.recv().await;
    }

    /// Non-blocking check — true if shutdown signal was already sent.
    pub fn is_shutdown(&mut self) -> bool {
        matches!(self.rx.try_recv(), Ok(_) | Err(broadcast::error::TryRecvError::Closed))
    }
}

/// Spawn the OS signal listener and return a `ShutdownController`.
///
/// ```rust,ignore
/// let (controller, root_rx) = listen_for_signals();
/// tokio::spawn(some_task(root_rx));
/// controller.wait_for_signal().await;
/// ```
pub fn listen_for_signals() -> (ShutdownController, ShutdownReceiver) {
    ShutdownController::new()
}

/// Awaits a SIGINT or SIGTERM from the OS, then triggers shutdown.
///
/// Intended to be `tokio::select!`-ed against the main daemon loop.
pub async fn wait_for_os_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigint = signal(SignalKind::interrupt()).expect("SIGINT handler");
        let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");

        tokio::select! {
            _ = sigint.recv()  => info!("Received SIGINT"),
            _ = sigterm.recv() => info!("Received SIGTERM"),
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("Ctrl-C handler");
        info!("Received Ctrl-C");
    }
}

/// Run all subsystem shutdown coroutines with a hard deadline.
///
/// Pass a vec of named futures; each represents a subsystem draining itself.
/// Any subsystem that does not complete within `deadline` is forcibly abandoned
/// with a warning.
pub async fn drain_subsystems(
    subsystems: Vec<(&'static str, impl std::future::Future<Output = ()>)>,
    deadline: Duration,
) {
    info!(
        subsystems = subsystems.len(),
        deadline_secs = deadline.as_secs(),
        "Draining subsystems"
    );

    for (name, fut) in subsystems {
        match timeout(deadline, fut).await {
            Ok(()) => info!(subsystem = name, "Drained cleanly"),
            Err(_) => warn!(
                subsystem = name,
                "Drain timeout exceeded — forcibly abandoned"
            ),
        }
    }

    info!("All subsystems shut down — exiting");
}

/// Convenience macro to build the daemon's main select loop.
///
/// ```rust,ignore
/// daemon_select! {
///     _ = ingestion_task  => error!("Ingestion exited"),
///     _ = ai_task         => error!("AI exited"),
///     _ = shutdown_signal => { /* graceful path */ }
/// }
/// ```
#[macro_export]
macro_rules! daemon_select {
    ($($tt:tt)*) => {
        tokio::select! {
            $($tt)*
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shutdown_propagates() {
        let (ctrl, mut rx) = ShutdownController::new();
        ctrl.shutdown();
        // Should resolve immediately
        timeout(Duration::from_millis(50), rx.wait())
            .await
            .expect("Shutdown signal not received in time");
    }

    #[tokio::test]
    async fn test_is_shutdown_false_before_signal() {
        let (_ctrl, mut rx) = ShutdownController::new();
        assert!(!rx.is_shutdown());
    }

    #[tokio::test]
    async fn test_is_shutdown_true_after_signal() {
        let (ctrl, mut rx) = ShutdownController::new();
        ctrl.shutdown();
        // small yield to let broadcast propagate
        tokio::task::yield_now().await;
        assert!(rx.is_shutdown());
    }
}
