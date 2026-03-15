// crates/persistence/src/worker.rs
//
// Asynchronous database worker queue.
// Buffers market ticks, signals, and execution fills, and batch-inserts them
// to TimescaleDB, fully decoupling I/O latency from the critical trading path.

use std::time::Duration;
use sqlx::{PgPool, Postgres, QueryBuilder};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, trace};
use metrics::{record_db_write_latency_ms, set_db_queue_depth};
use crate::db::MarketTick;

const BATCH_SIZE: usize = 5000;
const BATCH_TIMEOUT: Duration = Duration::from_millis(100);

pub enum DbEvent {
    MarketTick(MarketTick),
    // Signal(SignalRecord),
    // Fill(FillRecord),
}

pub struct AsyncDbWorker {
    pool: PgPool,
    rx: mpsc::Receiver<DbEvent>,
}

impl AsyncDbWorker {
    pub fn new(pool: PgPool, rx: mpsc::Receiver<DbEvent>) -> Self {
        Self { pool, rx }
    }

    pub async fn run(mut self) {
        let mut tick_batch: Vec<MarketTick> = Vec::with_capacity(BATCH_SIZE);
        let mut timer = interval(BATCH_TIMEOUT);

        loop {
            tokio::select! {
                Some(event) = self.rx.recv() => {
                    match event {
                        DbEvent::MarketTick(tick) => {
                            tick_batch.push(tick);
                            if tick_batch.len() >= BATCH_SIZE {
                                self.flush_ticks(&mut tick_batch).await;
                            }
                        }
                    }
                    // Update queue depth metric
                    set_db_queue_depth(tick_batch.len() as f64);
                }
                _ = timer.tick() => {
                    if !tick_batch.is_empty() {
                        self.flush_ticks(&mut tick_batch).await;
                        set_db_queue_depth(0.0);
                    }
                }
            }
        }
    }

    async fn flush_ticks(&self, batch: &mut Vec<MarketTick>) {
        let start = std::time::Instant::now();
        let batch_len = batch.len();

        let mut query_builder: QueryBuilder<Postgres> = QueryBuilder::new(
            "INSERT INTO market_ticks (time, symbol, bid, ask, last_price, volume) "
        );

        query_builder.push_values(batch.drain(..), |mut b, tick| {
            b.push_bind(tick.time)
             .push_bind(tick.symbol)
             .push_bind(tick.bid)
             .push_bind(tick.ask)
             .push_bind(tick.last_price)
             .push_bind(tick.volume);
        });

        match query_builder.build().execute(&self.pool).await {
            Ok(_) => {
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                record_db_write_latency_ms("market_ticks", ms);
                trace!("Flushed {} ticks to DB in {:.2}ms", batch_len, ms);
            }
            Err(e) => {
                error!("Failed to bulk insert {} ticks: {}", batch_len, e);
                // Fallback: If bulk fails (e.g. constraints), could fall back to single inserts
                // but for quant tick data, we usually just drop to avoid stalling pipeline.
            }
        }
    }
}
