//! Core engine loop: consumes multiplexed market data, runs strategies,
//! checks risk, and dispatches orders.

use crate::processor::Processor;
use crate::strategy::Strategy;
use common::events::*;
use common::time::{Clock, RealtimeClock, SequenceGenerator, UnixNanos};
use execution::gateway::{ExecutionGateway, OpenRequest, TimeInForce};
use futures::StreamExt;
use ingestion::source::MarketStream;
use risk::interceptor::{RiskChain, RiskVerdict};
use risk::state::EngineState;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Events emitted to the TUI via broadcast channel.
#[derive(Debug, Clone)]
pub enum TuiEvent {
    MarketUpdate(MarketEvent),
    OrderUpdate(OrderEvent),
    Signal(SignalEvent),
    StateSnapshot {
        equity: f64,
        daily_pnl: f64,
        drawdown_pct: f64,
        open_orders: usize,
    },
    Audit(AuditTick),
}

/// The main engine that ties everything together.
pub struct Engine {
    market_stream: MarketStream,
    strategy: Box<dyn Strategy>,
    risk_chain: RiskChain,
    executor: Box<dyn ExecutionGateway>,
    state: EngineState,
    clock: RealtimeClock,
    seq_gen: Arc<SequenceGenerator>,
    tui_tx: broadcast::Sender<TuiEvent>,
    order_counter: u64,
}

impl Engine {
    pub fn new(
        market_stream: MarketStream,
        strategy: Box<dyn Strategy>,
        risk_chain: RiskChain,
        executor: Box<dyn ExecutionGateway>,
        state: EngineState,
        tui_tx: broadcast::Sender<TuiEvent>,
    ) -> Self {
        Self {
            market_stream,
            strategy,
            risk_chain,
            executor,
            state,
            clock: RealtimeClock,
            seq_gen: Arc::new(SequenceGenerator::new()),
            tui_tx,
            order_counter: 0,
        }
    }

    /// Run the engine loop. This is the hot path.
    pub async fn run(mut self) {
        info!(
            strategy = self.strategy.name(),
            executor = self.executor.name(),
            "Engine started"
        );

        let mut tick_count: u64 = 0;
        let mut last_state_broadcast = UnixNanos::ZERO;

        while let Some(result) = self.market_stream.next().await {
            let envelope = match result {
                Ok(env) => env,
                Err(e) => {
                    warn!(error = %e, "Market stream error, continuing");
                    continue;
                }
            };

            tick_count += 1;

            // 1. Forward market data to TUI
            let _ = self.tui_tx.send(TuiEvent::MarketUpdate(
                envelope.payload.clone(),
            ));

            // 2. Emit audit tick
            let audit = AuditTick {
                ts: self.clock.now(),
                sequence_id: self.seq_gen.next_id(),
                event: AuditEvent::MarketDataReceived {
                    symbol: compact_str::CompactString::new(
                        envelope.payload.symbol(),
                    ),
                    source: compact_str::CompactString::new("multiplexer"),
                },
            };
            let _ = self.tui_tx.send(TuiEvent::Audit(audit));

            // 3. Run strategy
            let signals = self.strategy.on_market_event(&envelope).await;

            // 4. Process signals through risk + execution
            for signal in signals {
                let _ = self.tui_tx.send(TuiEvent::Signal(signal.clone()));

                self.order_counter += 1;
                let client_order_id = compact_str::CompactString::new(
                    format!("RF-{:08}", self.order_counter),
                );

                let request = OpenRequest {
                    client_order_id: client_order_id.clone(),
                    symbol: signal.symbol.clone(),
                    side: signal.direction,
                    quantity: signal.confidence * 100.0, // Scale by confidence
                    order_type: common::events::OrderType::Market,
                    limit_price: None,
                    time_in_force: TimeInForce::DAY,
                };

                // Risk check
                let verdict = self.risk_chain.evaluate(&self.state, &request);

                match verdict {
                    RiskVerdict::Approved => {
                        debug!(
                            order_id = %client_order_id,
                            symbol = %signal.symbol,
                            "Order approved by risk chain"
                        );

                        match self.executor.submit_order(request).await {
                            Ok(order_event) => {
                                let _ = self.tui_tx.send(
                                    TuiEvent::OrderUpdate(order_event),
                                );
                            }
                            Err(e) => {
                                error!(
                                    order_id = %client_order_id,
                                    error = %e,
                                    "Order submission failed"
                                );
                            }
                        }
                    }
                    RiskVerdict::Blocked { reason } => {
                        warn!(
                            order_id = %client_order_id,
                            reason = %reason,
                            "Order blocked by risk"
                        );
                    }
                    RiskVerdict::Modified { new_request, reason } => {
                        info!(
                            reason = %reason,
                            "Order modified by risk, submitting adjusted"
                        );
                        let _ = self.executor.submit_order(new_request).await;
                    }
                }
            }

            // 5. Periodic state broadcast to TUI (every 500ms)
            let now = self.clock.now();
            if now - last_state_broadcast > 500_000_000 {
                // 500ms in ns
                self.state.update_drawdown(self.state.total_equity);
                let _ = self.tui_tx.send(TuiEvent::StateSnapshot {
                    equity: self.state.total_equity,
                    daily_pnl: self.state.daily_pnl,
                    drawdown_pct: self.state.current_drawdown_pct,
                    open_orders: self.state.open_order_count,
                });
                last_state_broadcast = now;
            }

            if tick_count % 10_000 == 0 {
                debug!(
                    ticks = tick_count,
                    equity = self.state.total_equity,
                    "Engine heartbeat"
                );
            }
        }

        info!(total_ticks = tick_count, "Engine stopped — stream exhausted");
    }
}
