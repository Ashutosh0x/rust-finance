#![forbid(unsafe_code)]
use daemon::bootstrap::{bootstrap, DaemonConfig};
use daemon::engine::{Engine, TuiEvent};
use daemon::strategy::SimpleMomentum;
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    tracing::info!("RustForge Terminal v0.5.0 starting...");

    // Load configuration
    let config = DaemonConfig::from_env();

    // Bootstrap all components
    let (market_stream, risk_chain, executor, state) = bootstrap(&config).await;

    // TUI broadcast channel (daemon -> tui)
    let (tui_tx, _tui_rx) = broadcast::channel::<TuiEvent>(4096);

    // Strategy
    let strategy = Box::new(SimpleMomentum::new(20));

    // Build and run engine
    let engine = Engine::new(
        market_stream,
        strategy,
        risk_chain,
        executor,
        state,
        tui_tx,
    );

    engine.run().await;

    Ok(())
}
