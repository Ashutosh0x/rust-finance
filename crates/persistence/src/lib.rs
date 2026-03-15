use rusqlite::{params, Connection};
use serde::{Serialize, Deserialize};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;
use chrono::{Utc, DateTime};

pub mod repositories;

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

/// Spawns a background writer thread. Returns a Sender<PersistCommand>.
pub fn spawn_writer(db_path: &Path) -> anyhow::Result<Sender<PersistCommand>> {
    let (tx, rx) = mpsc::channel::<PersistCommand>();
    let db_path = db_path.to_owned();

    thread::Builder::new().name("db-writer".into()).spawn(move || {
        let conn = Connection::open(&db_path).expect("Critical: Failed to open persistence SQLite database");
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
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
            CREATE INDEX IF NOT EXISTS idx_trades_token ON trades(token);
            "#,
        ).expect("Critical: Failed to initialize SQLite tables");

        while let Ok(cmd) = rx.recv() {
            match cmd {
                PersistCommand::InsertTrade(rec) => {
                    let _ = conn.execute(
                        "INSERT OR IGNORE INTO trades (tx_sig, token, entry_price, exit_price, size, pnl, ts) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                        params![
                            rec.tx_sig,
                            rec.token,
                            rec.entry_price,
                            rec.exit_price,
                            rec.size,
                            rec.pnl,
                            rec.ts.to_rfc3339(),
                        ],
                    );
                }
                PersistCommand::Flush => {
                    let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL);");
                }
            }
        }
    })?;

    Ok(tx)
}
