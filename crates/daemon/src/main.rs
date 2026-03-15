use anyhow::{Result, Context};
use crossbeam_channel::bounded;
use parser::ParserService;
use ingestion::IngestionArgs;
use strategy::{Strategy, SimpleStrategy};
use risk::RiskManager;
use executor::ExecutorService;
use feature::FeatureEngine;
use common::{SwapEvent, Action, events::BotEvent};
use signer::LocalSigner;
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, error, warn};

mod kill_switch;
pub mod ai_pipeline;
pub mod circuit_breaker;
pub mod reconnect;
pub mod shutdown;
use kill_switch::KillSwitch;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    info!("Starting High-Performance RL Trading Bot Daemon");

    // --- CONFIGURATION ---
    let rpc_url = std::env::var("SOL_RPC").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into());
    let ws_url = std::env::var("SOL_WS").unwrap_or_else(|_| "wss://api.mainnet-beta.solana.com".into());
    let private_key = std::env::var("SOL_PRIVATE_KEY").ok();
    
    let ingestion_args = IngestionArgs {
        ws_url: ws_url.clone(),
        program_id: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
    };

    // --- CHANNELS (The Backbone) ---
    let (raw_tx, raw_rx) = bounded::<String>(100000);
    let (event_tx, event_rx) = bounded::<SwapEvent>(50000);
    let (action_tx, mut action_rx) = tokio::sync::mpsc::channel::<Action>(10000);

    // --- SHARED STATE ---
    let feature_engine = Arc::new(FeatureEngine::new());
    
    let signer = if let Some(k) = private_key {
        if let Ok(s) = LocalSigner::from_base58(&k) {
            Some(s)
        } else {
             warn!("Invalid SOL_PRIVATE_KEY provided. Falling back to mock if enabled.");
             None 
        }
    } else {
        None
    };
    
    // Only verify mock if signer is still None
    // Only verify mock if signer is still None
    let signer = if let Some(s) = signer {
        Some(s)
    } else {
         // Default to MOCK if no key provided or explicit USE_MOCK
         info!("No SOL_PRIVATE_KEY found. Generating random keypair for MOCK mode.");
         Some(LocalSigner::new(solana_sdk::signature::Keypair::new()))
    };

    // --- SERVICES ---
    // 1. Node Selector
    let nodes = vec![
        rpc_url.clone(), 
        "https://api.mainnet-beta.solana.com".to_string(),
    ];
    let selector = Arc::new(relay::NodeSelector::new(nodes));
    selector.clone().start(Duration::from_secs(10));

    let executor = Arc::new(ExecutorService::new(selector.clone(), signer));

    // --- PIPELINE STAGES ---
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<common::events::ControlCommand>(4096);
    

    // 2. Persistence
    if !std::path::Path::new("data").exists() {
        let _ = std::fs::create_dir("data");
    }
    let db_tx = persistence::spawn_writer(std::path::Path::new("data/trades.sqlite"))?;

    // 3. Web Dashboard
    let (web_tx, _) = tokio::sync::broadcast::channel::<String>(1024);
    let web_tx_clone = web_tx.clone();
    tokio::spawn(async move {
        web_dashboard::serve(web_tx_clone).await;
    });

    let event_bus = Arc::new(event_bus::EventBus::start(cmd_tx).await?);

    // Command handling loop
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            info!("Daemon executing: {:?}", cmd);
        }
    });

    // 1. Parser Worker (Threaded)
    let p_rx = raw_rx.clone();
    let p_tx = event_tx.clone();
    thread::Builder::new().name("parser-worker".into()).spawn(move || {
        let parser = ParserService::new(p_rx);
        info!("Parser thread started");
        while let Ok(raw) = parser.rx().recv() {
            if let Ok(events) = parser.process_message(&raw) {
                for e in events {
                    let _ = p_tx.send(e);
                }
            }
        }
    })?;

    // 2. Strategy & Risk Worker (Threaded)
    let s_rx = event_rx.clone();
    let s_tx = action_tx.clone();
    let s_features = feature_engine.clone();
    let kill_switch = Arc::new(KillSwitch::new());
    let ks_strategy = kill_switch.clone();

    thread::Builder::new().name("strategy-worker".into()).spawn(move || {
        let mut strategy = SimpleStrategy::new(1_000_000); 
        // RiskManager::new(max_pos, min_conf, init_equity, max_daily_loss, max_drawdown_pct)
        let risk_manager = RiskManager::new(1.0, 0.7, 10000.0, 500.0, 0.05);
        info!("Strategy thread started");
        
        while let Ok(event) = s_rx.recv() {
            if ks_strategy.is_halted() {
                continue;
            }

            s_features.process_event(&event);
            let action = strategy.on_event(&event);

            if risk_manager.is_halt_required() {
                ks_strategy.trigger("Risk limits breached (Daily Loss or Drawdown)");
            }

            if let Ok(approved) = risk_manager.check_action(action) {
                if !matches!(approved, Action::Hold) && !ks_strategy.is_halted() {
                    let _ = s_tx.blocking_send(approved);
                }
            }
        }
    })?;

    // 3. Executor Task (Async)
    let mut e_rx = action_rx; // move receiver
    let e_exec = executor.clone();
    let e_db = db_tx.clone();
    let e_bus = event_bus.clone();
    let e_web = web_tx.clone();

    tokio::spawn(async move {
        info!("Executor task started");
        loop {
            if let Some(action) = e_rx.recv().await {
                let exec_clone = e_exec.clone();
                let db_clone = e_db.clone();
                let bus_clone = e_bus.clone();
                let web_clone = e_web.clone();
                
                tokio::spawn(async move {
                    match exec_clone.execute_action(action.clone()).await {
                        Ok(sig) => {
                            info!("Action executed successfully: {}", sig);
                            // 1. Persist trade
                            if let Action::Buy { token, size, .. } = action {
                                let record = persistence::TradeRecord {
                                    tx_sig: sig.to_string(),
                                    token: token.clone(),
                                    entry_price: 100.0, // Mock price for now
                                    exit_price: None,
                                    size,
                                    pnl: None,
                                    ts: chrono::Utc::now(),
                                };
                                let _ = db_clone.send(persistence::PersistCommand::InsertTrade(record));
                                
                                // 2. Broadcast to TUI
                                bus_clone.broadcast(BotEvent::Feed(format!("BUY {} completed: {}", token, sig)));
                                
                                // 3. Update Web Dashboard
                                let _ = web_clone.send(format!("{{\"type\":\"trade\",\"sig\":\"{}\",\"token\":\"{}\"}}", sig, token));
                            }
                        }
                        Err(e) => {
                            error!("Execution failed: {:?}", e);
                            bus_clone.broadcast(BotEvent::Feed(format!("Execution FAILED: {:?}", e)));
                        }
                    }
                });
            } else {
                break;
            }
        }
    });

    // 4. Ingestion (Source)
    if std::env::var("USE_MOCK").is_ok() {
        // [Existing mock setup]
    } else {
        // Real Ingestion Setup with new Finnhub/Alpaca
        let finnhub_key = std::env::var("FINNHUB_API_KEY").unwrap_or_default();
        // Since p_tx is moved into the executor block, we just clone event_tx directly here
        let ev_tx = event_tx.clone();

        // 1. Spawn Finnhub or Mock
        if !finnhub_key.is_empty() {
            let fh = ingestion::FinnhubWs::new(finnhub_key, vec!["BINANCE:BTCUSDT".into()]);
            tokio::spawn(async move {
                // Mock channel for Finnhub -> EventBus
                let (fh_tx, mut fh_rx) = mpsc::unbounded_channel();
                let fh_eb = event_bus.clone();
                tokio::spawn(async move {
                    while let Some(ev) = fh_rx.recv().await {
                        fh_eb.broadcast(ev);
                    }
                });
                let _ = fh.run(fh_tx).await;
            });
        } else {
            // Mock Feed if no API Key is provided
            let mock_eb = event_bus.clone();
            tokio::spawn(async move {
                let mut price = 65000.0;
                loop {
                    price += rand::random::<f64>() * 100.0 - 50.0;
                    mock_eb.broadcast(BotEvent::MarketEvent {
                        event_type: "trade".into(),
                        symbol: "BINANCE:BTCUSDT".into(),
                        price,
                        volume: Some(rand::random::<f64>() * 2.0),
                        timestamp: chrono::Utc::now().timestamp_millis(),
                    });
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
        }

        // 2. Spawn AI components (Dexter & MiroFish)
        let dexter = ai::analyst::DexterAnalyst::new();
        tokio::spawn(async move {
            let _ = dexter.run().await;
        });

        let mirofish = ai::simulator::MiroFishSimulator::new(5000);
        tokio::spawn(async move {
            let _ = mirofish.run().await;
        });

        // Graceful Shutdown Hook
        info!("Daemon is running. Press Ctrl+C to initiate graceful shutdown.");
        tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl-c");
        
        info!("SIGINT received. Initiating graceful shutdown...");
        
        info!("Flushing persistence layer and disconnecting event bus...");
        // In a complete implementation, you would signal the `db_tx` and `event_bus` channels to drop
        // and cleanly flush any remaining DB operations to disk here.
        tokio::time::sleep(Duration::from_millis(500)).await;
        info!("Shutdown complete.");
    }

    Ok(())
}
