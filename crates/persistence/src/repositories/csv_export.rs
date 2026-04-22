// use rusqlite::Connection;
use anyhow::{Context, Result};
use sqlx_core::connection::Connection;
use sqlx_core::row::Row;
use sqlx_sqlite::{SqliteConnectOptions, SqliteConnection};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;

/// Exports the `trades` table to a CSV file for Backtesting and ML calibration.
/// Uses sqlx-sqlite directly to avoid zeroize conflicts with solana-sdk.
pub async fn export_trades_to_csv(db_path: &Path, out_path: &Path) -> Result<()> {
    let db_url = format!("sqlite://{}", db_path.to_string_lossy());
    let opts = SqliteConnectOptions::from_str(&db_url)?;
    let mut conn = SqliteConnection::connect_with(&opts)
        .await
        .context("Failed to connect to SQLite database for CSV export")?;

    let rows = sqlx_core::query::query(
        "SELECT tx_sig, token, entry_price, exit_price, size, pnl, ts FROM trades",
    )
    .fetch_all(&mut conn)
    .await
    .context("Failed to fetch trades from database")?;

    let mut file = File::create(out_path).context("Failed to create CSV file")?;

    // Write CSV header
    writeln!(file, "tx_sig,token,entry_price,exit_price,size,pnl,ts")?;

    for row in rows {
        let tx_sig: String = row.try_get(0).unwrap_or_default();
        let token: String = row.try_get(1).unwrap_or_default();
        let entry_price: f64 = row.try_get(2).unwrap_or(0.0);
        let exit_price: f64 = row.try_get(3).unwrap_or(0.0);
        let size: f64 = row.try_get(4).unwrap_or(0.0);
        let pnl: f64 = row.try_get(5).unwrap_or(0.0);
        let ts: String = row.try_get(6).unwrap_or_default();

        writeln!(
            file,
            "{},{},{},{},{},{},{}",
            tx_sig, token, entry_price, exit_price, size, pnl, ts
        )?;
    }

    Ok(())
}
