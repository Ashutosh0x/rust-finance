//! Daemon bootstrap: wires all data sources, risk chain, and execution
//! into a running system.

use common::time::SequenceGenerator;
use execution::alpaca_executor::AlpacaExecutor;
use execution::gateway::ExecutionGateway;
use execution::mock_executor::MockExecutor;
use ingestion::multiplexer::Multiplexer;
use ingestion::source::{DataType, Subscription};
use ingestion::sources::*;
use risk::interceptor::*;
use risk::state::EngineState;
use std::sync::Arc;
use tracing::{info, warn};

/// Configuration loaded from .env + CLI flags.
pub struct DaemonConfig {
    pub use_mock: bool,
    pub symbols_equity: Vec<String>,
    pub symbols_crypto: Vec<String>,
    pub symbols_polymarket: Vec<String>,
    pub starting_equity: f64,
    pub max_position_size: f64,
    pub max_drawdown_pct: f64,
    pub max_daily_loss: f64,
}

impl DaemonConfig {
    /// Load from environment variables with sensible defaults.
    pub fn from_env() -> Self {
        let use_mock = std::env::var("USE_MOCK")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        Self {
            use_mock,
            symbols_equity: env_list("SYMBOLS_EQUITY", "AAPL,MSFT,GOOGL,TSLA"),
            symbols_crypto: env_list("SYMBOLS_CRYPTO", "BTCUSDT,ETHUSDT"),
            symbols_polymarket: env_list("SYMBOLS_POLYMARKET", ""),
            starting_equity: env_f64("STARTING_EQUITY", 100_000.0),
            max_position_size: env_f64("MAX_POSITION_SIZE", 10_000.0),
            max_drawdown_pct: env_f64("MAX_DRAWDOWN_PCT", 5.0),
            max_daily_loss: env_f64("MAX_DAILY_LOSS", 2_000.0),
        }
    }
}

fn env_list(key: &str, default: &str) -> Vec<String> {
    std::env::var(key)
        .unwrap_or_else(|_| default.to_string())
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_string())
        .collect()
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Build and return the multiplexed data stream + risk chain + executor.
pub async fn bootstrap(
    config: &DaemonConfig,
) -> (
    ingestion::source::MarketStream,
    RiskChain,
    Box<dyn ExecutionGateway>,
    EngineState,
) {
    let seq_gen = Arc::new(SequenceGenerator::new());

    // ─── Build data source multiplexer ───────────────────────────

    let mut mux = Multiplexer::new();

    if config.use_mock {
        info!("Running in MOCK mode — no API keys required");
        mux = mux.add_source(MockSource::new(Arc::clone(&seq_gen)));
    } else {
        // Equity sources (Finnhub + Alpaca)
        if !config.symbols_equity.is_empty() {
            match FinnhubSource::from_env(Arc::clone(&seq_gen)) {
                Ok(source) => {
                    info!("Finnhub source configured");
                    mux = mux.add_source(source);
                }
                Err(e) => warn!(error = %e, "Finnhub unavailable, skipping"),
            }

            match AlpacaSource::from_env(Arc::clone(&seq_gen)) {
                Ok(source) => {
                    info!("Alpaca data source configured");
                    mux = mux.add_source(source);
                }
                Err(e) => warn!(error = %e, "Alpaca data unavailable, skipping"),
            }
        }

        // Crypto source (Binance)
        if !config.symbols_crypto.is_empty() {
            let binance = if std::env::var("BINANCE_TESTNET").is_ok() {
                BinanceSource::new(Arc::clone(&seq_gen)).testnet()
            } else {
                BinanceSource::new(Arc::clone(&seq_gen))
            };
            info!("Binance source configured");
            mux = mux.add_source(binance);
        }

        // Prediction market source (Polymarket)
        if !config.symbols_polymarket.is_empty() {
            let poly = PolymarketSource::new(Arc::clone(&seq_gen));
            info!("Polymarket source configured");
            mux = mux.add_source(poly);
        }
    }

    // Merge all symbols into one subscription
    let all_symbols: Vec<String> = config
        .symbols_equity
        .iter()
        .chain(config.symbols_crypto.iter())
        .chain(config.symbols_polymarket.iter())
        .cloned()
        .collect();

    let subscription = Subscription {
        symbols: all_symbols,
        data_types: vec![DataType::Trades, DataType::Quotes, DataType::OrderBookL1],
    };

    let market_stream = mux.connect(&subscription).await;

    // ─── Build risk chain ────────────────────────────────────────

    let risk_chain = RiskChain::new()
        .add(MaxPositionSize {
            max_quantity: config.max_position_size,
        })
        .add(MaxDrawdown {
            max_drawdown_pct: config.max_drawdown_pct,
        })
        .add(MaxOpenOrders { max_orders: 20 })
        .add(DailyLossLimit {
            max_daily_loss: config.max_daily_loss,
        });

    info!("Risk chain configured: 4 interceptors");

    // ─── Build executor ──────────────────────────────────────────

    let executor: Box<dyn ExecutionGateway> = if config.use_mock {
        Box::new(MockExecutor::new())
    } else {
        match AlpacaExecutor::from_env(true) {
            Ok(exec) => {
                info!("Alpaca paper executor configured");
                Box::new(exec)
            }
            Err(e) => {
                warn!(error = %e, "Alpaca executor unavailable, falling back to mock");
                Box::new(MockExecutor::new())
            }
        }
    };

    // ─── Build engine state ──────────────────────────────────────

    let state = EngineState::new(config.starting_equity);

    (market_stream, risk_chain, executor, state)
}
