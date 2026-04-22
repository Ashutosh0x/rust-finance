#![forbid(unsafe_code)]
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;

pub mod db;
pub mod dragonfly;
pub mod repositories;
pub mod worker;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TradeRecord {
    pub tx_sig: String,
    pub token: String,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub size: f64,
    pub pnl: Option<f64>,
    pub ts: DateTime<Utc>,
}

pub enum PersistCommand {
    InsertTrade(TradeRecord),
    Flush,
}

/// Spawns a background writer thread that persists trades to a local SQLite file.
/// Uses sqlx-sqlite sub-crate (not the sqlx umbrella) to avoid the zeroize
/// conflict with solana-sdk when sqlx-mysql gets pulled unconditionally.
pub fn spawn_writer(db_path: &Path) -> anyhow::Result<Sender<PersistCommand>> {
    let (tx, rx) = mpsc::channel::<PersistCommand>();
    let db_path = db_path.to_string_lossy().to_string();

    thread::Builder::new().name("db-writer".into()).spawn(move || {
        // Build a single-threaded tokio runtime for the blocking writer thread
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create writer runtime");

        rt.block_on(async move {
            use sqlx_sqlite::SqlitePoolOptions;
            use sqlx_core::executor::Executor;

            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect(&format!("sqlite://{}?mode=rwc", db_path))
                .await
                .expect("Critical: Failed to open persistence SQLite database");

            pool.execute(
                sqlx_core::query::query(
                    r#"
                    CREATE TABLE IF NOT EXISTS trades (
                        id INTEGER PRIMARY KEY,
                        tx_sig TEXT UNIQUE,
                        token TEXT,
                        entry_price REAL,
                        exit_price REAL,
                        size REAL,
                        pnl REAL,
                        ts TEXT
                    );
                    "#,
                )
            )
            .await
            .expect("Critical: Failed to initialize SQLite tables");

            pool.execute(
                sqlx_core::query::query("CREATE INDEX IF NOT EXISTS idx_trades_token ON trades(token)")
            )
            .await
            .ok();

            while let Ok(cmd) = rx.recv() {
                match cmd {
                    PersistCommand::InsertTrade(rec) => {
                        let _ = sqlx_core::query::query(
                            "INSERT OR IGNORE INTO trades (tx_sig, token, entry_price, exit_price, size, pnl, ts) VALUES (?, ?, ?, ?, ?, ?, ?)",
                        )
                        .bind(&rec.tx_sig)
                        .bind(&rec.token)
                        .bind(rec.entry_price)
                        .bind(rec.exit_price)
                        .bind(rec.size)
                        .bind(rec.pnl)
                        .bind(rec.ts.to_rfc3339())
                        .execute(&pool)
                        .await;
                    }
                    PersistCommand::Flush => {
                        // WAL checkpoint handled internally
                    }
                }
            }
        });
    })?;

    Ok(tx)
}
