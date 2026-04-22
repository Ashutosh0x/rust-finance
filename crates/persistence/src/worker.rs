// crates/persistence/src/worker.rs
//
// Asynchronous database worker queue.
// Uses sqlx-postgres sub-crate directly (not the sqlx umbrella)
// to avoid sqlx-mysql pulling rsa/zeroize which conflicts with solana-sdk.

use crate::db::MarketTick;
use sqlx_core::query_builder::QueryBuilder;
use sqlx_postgres::{PgPool, Postgres};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, trace};

const BATCH_SIZE: usize = 5000;
const BATCH_TIMEOUT: Duration = Duration::from_millis(100);

pub enum DbEvent {
    MarketTick(MarketTick),
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
                }
                _ = timer.tick() => {
                    if !tick_batch.is_empty() {
                        self.flush_ticks(&mut tick_batch).await;
                    }
                }
            }
        }
    }

    async fn flush_ticks(&self, batch: &mut Vec<MarketTick>) {
        let start = std::time::Instant::now();
        let batch_len = batch.len();

        let mut query_builder: QueryBuilder<Postgres> = QueryBuilder::new(
            "INSERT INTO market_ticks (time, symbol, bid, ask, last_price, volume) ",
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
                trace!("Flushed {} ticks to DB in {:.2}ms", batch_len, ms);
            }
            Err(e) => {
                error!("Failed to bulk insert {} ticks: {}", batch_len, e);
            }
        }
    }
}
