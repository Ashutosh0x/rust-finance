use rusqlite::Connection;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use anyhow::{Result, Context};

/// Exports the `trades` table to a CSV file for Backtesting and ML calibration.
pub fn export_trades_to_csv(db_path: &Path, out_path: &Path) -> Result<()> {
    let conn = Connection::open(db_path).context("Failed to open SQLite database for CSV export")?;
    let mut stmt = conn.prepare("SELECT tx_sig, token, entry_price, exit_price, size, pnl, ts FROM trades")?;
    
    let mut rows = stmt.query([])?;
    let mut file = File::create(out_path).context("Failed to create CSV file")?;
    
    // Write CSV header
    writeln!(file, "tx_sig,token,entry_price,exit_price,size,pnl,ts")?;
    
    while let Some(row) = rows.next()? {
        let tx_sig: String = row.get(0).unwrap_or_default();
        let token: String = row.get(1).unwrap_or_default();
        let entry_price: f64 = row.get(2).unwrap_or(0.0);
        let exit_price: f64 = row.get(3).unwrap_or(0.0);
        let size: f64 = row.get(4).unwrap_or(0.0);
        let pnl: f64 = row.get(5).unwrap_or(0.0);
        let ts: String = row.get(6).unwrap_or_default();
        
        writeln!(file, "{},{},{},{},{},{},{}", tx_sig, token, entry_price, exit_price, size, pnl, ts)?;
    }
    
    Ok(())
}
