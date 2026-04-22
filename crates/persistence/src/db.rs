use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx_core::pool::PoolOptions;
use sqlx_postgres::PgPool;

pub async fn connect_db(db_url: &str) -> Result<PgPool> {
    let pool = PoolOptions::<sqlx_postgres::Postgres>::new()
        .max_connections(50)
        .connect(db_url)
        .await?;

    Ok(pool)
}

/// Row representation for the `market_ticks` TimescaleDB hypertable.
#[derive(Debug, Clone)]
pub struct MarketTick {
    pub time: DateTime<Utc>,
    pub symbol: String,
    pub bid: f64,
    pub ask: f64,
    pub last_price: f64,
    pub volume: f64,
}
