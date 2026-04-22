use crate::dry_run::{simulate_fill, Fill, Order};
use std::sync::OnceLock;
use tracing::info;

static EXECUTION_MODE: OnceLock<String> = OnceLock::new();

fn execution_mode() -> &'static str {
    EXECUTION_MODE
        .get_or_init(|| std::env::var("EXECUTION_MODE").unwrap_or_else(|_| "dry_run".to_string()))
        .as_str()
}

pub async fn execute(order: Order) -> Result<Fill, String> {
    let mode = execution_mode();

    match mode {
        "dry_run" | "paper_trade" => {
            let fill = simulate_fill(&order);
            info!(
                "[DRY RUN] Simulated fill for {}: price = {:.4}, qty = {}",
                order.symbol, fill.fill_price, order.qty
            );
            Ok(fill)
        }
        "live" => {
            // TODO: Wire real exchange/RPC submission here
            info!(
                "[LIVE] Sending real order for {}: price = {}, qty = {}",
                order.symbol, order.price, order.qty
            );
            Err("Live execution not yet implemented — refusing to send real order".into())
        }
        _ => Err(format!(
            "[SECURITY SAFEGUARD] Unknown EXECUTION_MODE '{}'. Order blocked.",
            mode
        )),
    }
}
