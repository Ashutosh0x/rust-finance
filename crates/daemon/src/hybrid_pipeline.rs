// ============================================================
// crates/daemon/src/hybrid_pipeline.rs
//
// The full RustForge intelligence pipeline:
// Market Data → Quant Models → Swarm Sim → Knowledge Graph
//              → Dexter AI → Risk Gate → Execution
//
// This is the core loop that runs in the daemon.
// Each stage feeds into the next via typed structs —
// no string glue, no serialization overhead on the hot path.
// ============================================================

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

use ai::dexter::DexterSignal;
use common::events::BotEvent; // assuming BotEvent is in common::events
use risk::gate::RiskVerdict;
use std::fmt;

#[derive(Debug, Clone)]
pub struct OrderRequest {
    pub side: String,
    pub symbol: String,
}

use knowledge_graph::graph::FinancialGraph;
use knowledge_graph::impact::ImpactEngine;
use knowledge_graph::query::{GraphQuery, GraphQueryEngine};
use swarm_sim::{MarketState as SwarmMarketState, SwarmConfig, SwarmEngine, SwarmStep};

// Mock MarketEvent based on usage if it doesn't already exist where expected
#[derive(Debug, Clone)]
pub struct LocalMarketEvent {
    pub symbol: String,
    pub price: f64,
    pub price_history: Vec<f64>,
    pub vwap: Option<f64>,
    pub order_book: Option<LocalOrderBook>,
}

#[derive(Debug, Clone)]
pub struct LocalOrderBook {
    // mock methods
}

impl LocalOrderBook {
    pub fn imbalance(&self, _levels: usize) -> f64 {
        0.0
    }
}

// ── Stage outputs (typed pipeline bus) ──────────────────────────────────────

/// Output of the quant analysis stage.
/// Computed from live market data — zero LLM calls, sub-ms.
#[derive(Debug, Clone)]
pub struct QuantSnapshot {
    pub symbol: String,
    pub price: f64,
    pub rsi_14: f64,
    pub garch_vol_forecast: f64, // GARCH(1,1) next-period variance
    pub heston_implied_vol: f64, // From your existing pricing crate
    pub vwap: f64,
    pub order_book_imbalance: f64,   // [-1, 1], from live order book
    pub momentum_signal: f64,        // Composite: 1h + 1d + MACD
    pub bsm_fair_value: Option<f64>, // Option fair value if chain available
}

impl risk::gate::QuantSnapshotLike for QuantSnapshot {
    fn garch_vol_forecast(&self) -> f64 {
        self.garch_vol_forecast
    }
}

/// Fused intelligence — quant + swarm + graph combined
/// This is what Dexter AI receives as context.
#[derive(Debug, Clone)]
pub struct FusedContext {
    pub quant: QuantSnapshot,
    pub swarm: SwarmStep,
    pub graph_context: String, // to_prompt_block() string
    pub impact_table: String,  // ImpactEngine output
}

impl FusedContext {
    /// Builds the full system prompt block injected into Dexter's Claude call.
    /// Called once per analysis cycle — the most important function in the codebase.
    pub fn to_dexter_system_prompt(&self) -> String {
        format!(
            r#"You are Dexter, RustForge's elite quantitative analyst.
Provide concise, specific, actionable analysis. No generic statements.

=== QUANTITATIVE SNAPSHOT: {} ===
Price:              ${:.2}
RSI(14):            {:.1} ({})
GARCH Vol Forecast: {:.2}% (next period)
Heston IV:          {:.2}%
VWAP:               ${:.2}
OB Imbalance:       {:.2} ({})
Momentum:           {:.3}
{}

=== SWARM SIMULATION (5,000 agents, round {}) ===
Direction:     {:?}
Conviction:    {:?}
Bulls:         {:.1}% | Bears: {:.1}%
Net Flow:      ${:+.0}K
Regime:        {:?}
Confidence:    {:.1}%
{}

=== KNOWLEDGE GRAPH CONTEXT ===
{}

=== IMPACT ANALYSIS ===
{}
"#,
            self.quant.symbol,
            self.quant.price,
            self.quant.rsi_14,
            rsi_label(self.quant.rsi_14),
            self.quant.garch_vol_forecast * 100.0,
            self.quant.heston_implied_vol * 100.0,
            self.quant.vwap,
            self.quant.order_book_imbalance,
            imbalance_label(self.quant.order_book_imbalance),
            self.quant.momentum_signal,
            self.quant
                .bsm_fair_value
                .map(|v| format!("BSM Fair Value: ${:.2}", v))
                .unwrap_or_default(),
            self.swarm.round,
            self.swarm.signal.direction,
            self.swarm.signal.conviction,
            self.swarm.signal.bullish_prob * 100.0,
            self.swarm.signal.bearish_prob * 100.0,
            self.swarm.net_flow_usd / 1_000.0,
            self.swarm.signal.regime.clone(),
            self.swarm.signal.confidence * 100.0,
            "Swarm narrative here", //self.swarm.signal.to_prompt_context(),
            self.graph_context,
            self.impact_table,
        )
    }
}

impl ai::dexter::FusedContextLike for FusedContext {
    fn to_dexter_system_prompt(&self) -> String {
        self.to_dexter_system_prompt()
    }

    fn get_symbol(&self) -> String {
        self.quant.symbol.clone()
    }
}

fn rsi_label(rsi: f64) -> &'static str {
    if rsi > 70.0 {
        "OVERBOUGHT"
    } else if rsi < 30.0 {
        "OVERSOLD"
    } else {
        "neutral"
    }
}

fn imbalance_label(imb: f64) -> &'static str {
    if imb > 0.3 {
        "bid heavy"
    } else if imb < -0.3 {
        "ask heavy"
    } else {
        "balanced"
    }
}

// ── Pipeline orchestrator ────────────────────────────────────────────────────

pub struct HybridPipeline {
    graph: Arc<RwLock<FinancialGraph>>,
    swarm: Arc<RwLock<SwarmEngine>>,
    event_tx: broadcast::Sender<BotEvent>,
    config: PipelineConfig,
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub symbols: Vec<String>,
    /// Only send to execution if swarm confidence > this threshold
    pub min_swarm_confidence: f64,
    /// Only send to execution if Dexter confidence > this threshold
    pub min_dexter_confidence: f64,
    /// How many rounds between full pipeline cycles
    pub pipeline_interval_rounds: u32,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            symbols: vec!["NVDA".into(), "AAPL".into(), "TSLA".into()],
            min_swarm_confidence: 0.60,
            min_dexter_confidence: 0.65,
            pipeline_interval_rounds: 5,
        }
    }
}

impl HybridPipeline {
    pub fn new(
        graph: Arc<RwLock<FinancialGraph>>,
        swarm: Arc<RwLock<SwarmEngine>>,
        event_tx: broadcast::Sender<BotEvent>,
        config: PipelineConfig,
    ) -> Self {
        Self {
            graph,
            swarm,
            event_tx,
            config,
        }
    }

    /// Main pipeline cycle — called every N rounds from the daemon.
    pub async fn run_cycle(&self, market: &LocalMarketEvent) {
        let symbol = &market.symbol;

        // ── Stage 1: Quant snapshot ────────────────────────────────────────
        let quant = self.compute_quant(market);
        debug!(
            "[Pipeline] Quant: RSI={:.1} vol={:.2}%",
            quant.rsi_14,
            quant.garch_vol_forecast * 100.0
        );

        // ── Stage 2: Latest swarm step (zero-copy read) ────────────────────
        let swarm_step = {
            let swarm = self.swarm.read().await;
            let stats = swarm.stats();
            build_swarm_step_from_stats(&stats, market.price)
        };

        // ── Stage 3: Knowledge graph context ──────────────────────────────
        let (graph_ctx, impact_table) = {
            let graph = self.graph.read().await;
            let engine = GraphQueryEngine::new(&*graph);
            let ctx = engine.full_context_for_symbol(symbol);
            let impact = ImpactEngine::new(&*graph);
            let table = impact.impact_table(symbol, 0.05, 5);
            (ctx.to_prompt_block(), table)
        };

        let fused = FusedContext {
            quant,
            swarm: swarm_step,
            graph_context: graph_ctx,
            impact_table,
        };

        // ── Stage 4: Dexter AI analysis ────────────────────────────────────
        let dexter_signal = match ai::dexter::analyse(&fused).await {
            Ok(s) => s,
            Err(e) => {
                warn!("[Pipeline] Dexter failed: {}", e);
                return;
            }
        };

        info!(
            "[Pipeline] Dexter: {:?} conf={:.2} | Swarm: {:?} conf={:.2}",
            dexter_signal.direction,
            dexter_signal.confidence,
            fused.swarm.signal.direction,
            fused.swarm.signal.confidence,
        );

        // ── Stage 5: Risk gate ─────────────────────────────────────────────
        let verdict = risk::gate::evaluate(&dexter_signal, &fused.swarm.signal, &fused.quant);

        match verdict {
            RiskVerdict::Approved(order) => {
                // ── Stage 6: Execution ─────────────────────────────────────
                info!(
                    "[Pipeline] EXECUTING: {:?} {} @ market",
                    order.side, order.symbol
                );
                // let _ = self.event_tx.send(BotEvent::TradeSignal(dexter_signal));
                // execution::submit(order).await;
            }
            RiskVerdict::Rejected(reason) => {
                debug!("[Pipeline] Risk rejected: {}", reason);
            }
            RiskVerdict::Hedge(_hedge_order) => {
                // info!("[Pipeline] Hedging: {:?}", hedge_order);
                // execution::submit(hedge_order).await;
            }
        }
    }

    fn compute_quant(&self, market: &LocalMarketEvent) -> QuantSnapshot {
        // use pricing::{bsm, garch, heston};

        // Mocking quant values for now since pricing crate might not have layout needed
        let garch_vol = 0.01; //garch::forecast_variance(market.price_history.as_slice());
        let heston_iv = 0.02; //heston::implied_vol(market.price, market.price_history.as_slice());
        let rsi = 50.0; // crate::indicators::rsi_14(market.price_history.as_slice());
        let momentum = 0.0; // crate::indicators::composite_momentum(market.price_history.as_slice());
        let imbalance = market
            .order_book
            .as_ref()
            .map(|ob| ob.imbalance(5))
            .unwrap_or(0.0);

        QuantSnapshot {
            symbol: market.symbol.clone(),
            price: market.price,
            rsi_14: rsi,
            garch_vol_forecast: garch_vol,
            heston_implied_vol: heston_iv,
            vwap: market.vwap.unwrap_or(market.price),
            order_book_imbalance: imbalance,
            momentum_signal: momentum,
            bsm_fair_value: None, // populated if options chain is available
        }
    }
}

// Placeholder — replace with real broadcast channel consumer in production
fn build_swarm_step_from_stats(stats: &swarm_sim::engine::EngineStats, price: f64) -> SwarmStep {
    use swarm_sim::signal::{Conviction, MarketRegime, SignalDirection, SwarmSignal};

    let market_state = SwarmMarketState::new("LIVE", price);
    let buy_frac = stats.long_agents as f64 / stats.agent_count as f64;
    let sell_frac = stats.short_agents as f64 / stats.agent_count as f64;

    let signal = SwarmSignal::from_round(stats.round, &market_state, buy_frac, sell_frac, 0.0);

    SwarmStep {
        round: stats.round,
        signal,
        actions_count: stats.long_agents + stats.short_agents,
        net_flow_usd: (stats.total_long_usd - stats.total_short_usd),
        buy_count: stats.long_agents,
        sell_count: stats.short_agents,
        hold_count: stats.flat_agents,
        price_after: stats.mid_price,
        realized_pnl_total: 0.0,
    }
}
