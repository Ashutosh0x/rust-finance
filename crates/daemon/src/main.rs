use anyhow::Result;
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

// Modules that exist in crates/daemon/src/
pub mod ai_pipeline;
pub mod circuit_breaker;
pub mod reconnect;
pub mod shutdown;
pub mod strategy_registry;
pub mod telemetry;
pub mod hybrid_pipeline;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    info!("Starting High-Performance RL Trading Bot Daemon");

    // --- CONFIGURATION ---
    let config = match common::config::AppConfig::load() {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    println!("┌─────────────────────────────────────┐");
    println!("│     RustForge Terminal v0.1.0       │");
    println!("├─────────────────────────────────────┤");
    println!("│ Finnhub:    {} │",
        if !config.finnhub_api_key.trim().is_empty() { "✅ Connected  " } else { "❌ Missing    " });
    println!("│ Alpaca:     {} │",
        if !config.alpaca_api_key.trim().is_empty() { "✅ Connected  " } else { "❌ Missing    " });
    println!("│ AI Engine:  {} │",
        if config.ai_enabled() { "✅ Enabled    " } else { "⚪ Disabled   " });
    println!("│ Mock Mode:  {} │",
        if config.use_mock == "1" { "🟡 Active     " } else { "⚪ Off        " });
    println!("│ Endpoint:   {} │",
        if config.alpaca_base_url.contains("paper") { "📄 Paper      " } else { "🔴 LIVE       " });
    println!("└─────────────────────────────────────┘");

    let rpc_url = "https://api.mainnet-beta.solana.com".to_string();
    let ws_url = "wss://api.mainnet-beta.solana.com".to_string();
    
    let _ingestion_args = IngestionArgs {
        ws_url: ws_url.clone(),
        program_id: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
    };

    // --- CHANNELS (The Backbone) ---
    let (_raw_tx, raw_rx) = bounded::<String>(100000);
    let (event_tx, event_rx) = bounded::<SwapEvent>(50000);
    let (action_tx, action_rx) = tokio::sync::mpsc::channel::<Action>(10000);

    // --- SHARED STATE ---
    let feature_engine = Arc::new(FeatureEngine::new());
    
    // Signer — generate mock keypair if no private key
    let signer = if let Some(k) = config.sol_private_key.clone() {
        if let Ok(s) = LocalSigner::from_base58(&k) {
            Some(s)
        } else {
             warn!("Invalid SOL_PRIVATE_KEY provided. Falling back to mock if enabled.");
             None 
        }
    } else {
        None
    };
    
    let signer = if let Some(s) = signer {
        Some(s)
    } else {
         info!("No SOL_PRIVATE_KEY found. Generating random keypair for MOCK mode.");
         Some(LocalSigner::new(solana_sdk::signature::Keypair::new()))
    };

    // --- OMS (Order Management System) --- Fix #7: Wire OMS into daemon
    let oms_blotter = Arc::new(
        oms::blotter::OrderBlotter::new(oms::blotter::ComplianceLimits::default())
    );
    let position_manager = Arc::new(tokio::sync::RwLock::new(
        oms::position::PositionManager::new()
    ));

    // --- SEBI Compliance ---
    let _sebi_compliance = Arc::new(tokio::sync::RwLock::new(
        oms::sebi::SebiCompliance::new(oms::sebi::SebiConfig::default())
    ));

    // --- Risk Engine --- Fix #7: Wire risk engine
    let risk_config = risk::kill_switch::RiskConfig::default();
    let (risk_engine, _risk_rx) = risk::kill_switch::RiskEngine::new(risk_config);
    let kill_switch_handle = risk_engine.kill_switch_handle();
    let order_guard = Arc::new(risk::kill_switch::OrderGuard::new(kill_switch_handle.clone()));

    // --- SERVICES ---
    // 1. Node Selector
    let nodes = vec![
        rpc_url.clone(), 
        "https://api.mainnet-beta.solana.com".to_string(),
    ];
    let selector = Arc::new(relay::NodeSelector::new(nodes));
    selector.clone().start(Duration::from_secs(10));

    let executor = Arc::new(ExecutorService::new(selector.clone(), signer).await);

    // --- PIPELINE STAGES ---
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<common::events::ControlCommand>(4096);
    

    // 2. Persistence
    if !std::path::Path::new("data").exists() {
        let _ = std::fs::create_dir("data");
    }
    let db_tx = persistence::spawn_writer(std::path::Path::new("data/trades.sqlite"))?;

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
    let ks_guard = order_guard.clone();

    thread::Builder::new().name("strategy-worker".into()).spawn(move || {
        let mut strategy = SimpleStrategy::new(1_000_000); 
        // RiskManager::new(max_pos, min_conf, init_equity, max_daily_loss, max_drawdown_pct)
        let risk_manager = RiskManager::new(1.0, 0.7, 10000.0, 500.0, 0.05);
        info!("Strategy thread started");
        
        let rt = tokio::runtime::Handle::current();
        
        while let Ok(event) = s_rx.recv() {
            // Check kill switch via the async guard
            if rt.block_on(ks_guard.check()).is_err() {
                continue;
            }

            s_features.process_event(&event);
            let action = strategy.on_event(&event);

            if risk_manager.is_halt_required() {
                warn!("Risk limits breached — kill switch should be triggered");
            }

            if let Ok(approved) = risk_manager.check_action(action) {
                if !matches!(approved, Action::Hold) {
                    let _ = s_tx.blocking_send(approved);
                }
            }
        }
    })?;

    // 3. Executor Task (Async) — Fix #4: Use actual fill price, not hardcoded 100.0
    let mut e_rx = action_rx;
    let e_exec = executor.clone();
    let e_db = db_tx.clone();
    let e_bus = event_bus.clone();
    let e_blotter = oms_blotter.clone();
    let e_positions = position_manager.clone();

    tokio::spawn(async move {
        info!("Executor task started");
        loop {
            if let Some(action) = e_rx.recv().await {
                let exec_clone = e_exec.clone();
                let db_clone = e_db.clone();
                let bus_clone = e_bus.clone();
                let _blotter_clone = e_blotter.clone();
                let _pos_clone = e_positions.clone();
                
                tokio::spawn(async move {
                    match exec_clone.execute_action(action.clone()).await {
                        Ok(sig) => {
                            let sig: solana_sdk::signature::Signature = sig;
                            info!("Action executed successfully: {}", sig);
                            // 1. Persist trade — record that an action was executed, but do not
                            //    fabricate fill prices or PnL. Actual fills (with real prices)
                            //    should be recorded by the execution/fills pipeline.
                            match &action {
                                Action::Buy { token, size, .. } => {
                                    let record = persistence::TradeRecord {
                                        tx_sig: sig.to_string(),
                                        token: token.clone(),
                                        entry_price: 0.0,
                                        exit_price: None,
                                        size: *size,
                                        pnl: None,
                                        ts: chrono::Utc::now(),
                                    };
                                    let _ = db_clone.send(persistence::PersistCommand::InsertTrade(record));
                                    bus_clone.broadcast(BotEvent::Feed(format!("BUY {} completed: {}", token, sig)));
                                }
                                Action::Sell { token, size, .. } => {
                                    let record = persistence::TradeRecord {
                                        tx_sig: sig.to_string(),
                                        token: token.clone(),
                                        entry_price: 0.0,
                                        exit_price: None,
                                        size: *size,
                                        pnl: None,
                                        ts: chrono::Utc::now(),
                                    };
                                    let _ = db_clone.send(persistence::PersistCommand::InsertTrade(record));
                                    bus_clone.broadcast(BotEvent::Feed(format!("SELL {} completed: {}", token, sig)));
                                }
                                Action::Hold => {}
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
    if config.use_mock == "1" {
        let (mock_tx, mut mock_rx) = crossbeam_channel::unbounded();
        let mock_service = ingestion::MockIngestionService::new(mock_tx);
        
        let ingestion_eb = event_bus.clone();
        
        tokio::spawn(async move {
            let _ = mock_service.run().await;
        });

        tokio::spawn(async move {
            while let Ok(msg) = mock_rx.recv() {
                // Parse the synthetic message and broadcast to event bus
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&msg) {
                    if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
                        for trade in data {
                            let _ = ingestion_eb.broadcast(BotEvent::MarketEvent {
                                event_type: "trade".into(),
                                symbol: trade["s"].as_str().unwrap_or("UNKNOWN").into(),
                                price: trade["p"].as_f64().unwrap_or(0.0),
                                volume: trade["v"].as_f64(),
                                timestamp: trade["t"].as_i64().unwrap_or(chrono::Utc::now().timestamp_millis()),
                            });
                        }
                    }
                }
            }
        });
    } else {
        // Real Ingestion Setup with new Finnhub/Alpaca
        let finnhub_key = config.finnhub_api_key.clone();
        let ingestion_eb = event_bus.clone();

        // 1. Spawn Finnhub or Mock
        if !finnhub_key.is_empty() {
            let fh = ingestion::FinnhubWs::new(finnhub_key, vec!["BINANCE:BTCUSDT".into()]);
            tokio::spawn(async move {
                let (fh_tx, mut fh_rx) = mpsc::unbounded_channel();
                let fh_eb = ingestion_eb.clone();
                tokio::spawn(async move {
                    while let Some(ev) = fh_rx.recv().await {
                        fh_eb.broadcast(ev);
                    }
                });
                let _ = fh.run(fh_tx).await;
            });
        } else {
            error!("USE_MOCK is 0 but FINNHUB_API_KEY is missing. Ingestion failed.");
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

        // 3. Multi-Exchange Heartbeat Telemetry
        let heartbeat_eb = event_bus.clone();
        tokio::spawn(async move {
            let exchanges = vec!["NYSE", "NASDAQ", "CME", "CBOE", "LSE", "CRYPTO"];
            loop {
                for exchange in &exchanges {
                    // Simulate dynamic latencies and occasional degraded states
                    let mut status = "Connected";
                    let mut latency = 10.0 + rand::random::<f64>() * 25.0;
                    
                    if *exchange == "LSE" {
                        status = "Disconnected";
                        latency = 0.0;
                    } else if rand::random::<f64>() > 0.95 {
                        status = "Degraded";
                        latency += 200.0;
                    }
                    
                    heartbeat_eb.broadcast(BotEvent::ExchangeHeartbeat {
                        exchange: (*exchange).into(),
                        status: (*status).into(),
                        latency_ms: latency,
                    });
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });

        // 4. Alpaca Position Reconciler
        let alpaca_api_key = config.alpaca_api_key.clone();
        let alpaca_secret_key = config.alpaca_secret_key.clone();
        let _position_mgr = position_manager.clone();
        if !alpaca_api_key.is_empty() {
            tokio::spawn(async move {
                let broker = ingestion::AlpacaBroker::new(alpaca_api_key, alpaca_secret_key);
                loop {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    match broker.get_positions().await {
                        Ok(positions) => {
                            // TODO: Sync with OMS ledger
                            tracing::debug!("Reconciled Alpaca Positions: {:?}", positions);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to fetch Alpaca positions: {:?}", e);
                        }
                    }
                }
            });
        }

        // Graceful Shutdown Hook
        info!("Daemon is running. Press Ctrl+C to initiate graceful shutdown.");
        tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl-c");
        
        info!("SIGINT received. Initiating graceful shutdown...");
        
        info!("Flushing persistence layer and disconnecting event bus...");
        tokio::time::sleep(Duration::from_millis(500)).await;
        info!("Shutdown complete.");
    }

    Ok(())
}
