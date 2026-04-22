//! Alpaca Trade Updates WebSocket — real-time fill, cancel, and reject events.
//!
//! Connects to `wss://paper-api.alpaca.markets/stream` (paper) or
//! `wss://api.alpaca.markets/stream` (live) and parses order lifecycle
//! events into strongly-typed `OrderEvent` variants.
//!
//! This closes the fire-and-forget gap in the execution layer by providing
//! real-time confirmation of order fills, partial fills, cancellations,
//! and rejections.
//!
//! ## Alpaca Streaming Protocol
//! 1. Connect to WebSocket endpoint
//! 2. Send auth: `{"action":"auth","key":"...","secret":"..."}`
//! 3. Subscribe: `{"action":"listen","data":{"streams":["trade_updates"]}}`
//! 4. Receive events: `{"stream":"trade_updates","data":{"event":"fill",...}}`

use anyhow::{Context, Result};
use common::events::{OrderAccepted, OrderCancelled, OrderEvent, OrderFilled, OrderRejected};
use compact_str::CompactString;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

// ═══════════════════════════════════════════════════════════════════════════════
// ─── Wire Types ─────────────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

/// Top-level WebSocket message from Alpaca trading stream.
#[derive(Debug, Deserialize)]
struct StreamMessage {
    stream: Option<String>,
    data: Option<TradeUpdateData>,
}

/// Authorization response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AuthResponse {
    stream: Option<String>,
    data: Option<AuthData>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AuthData {
    status: Option<String>,
    action: Option<String>,
}

/// Trade update payload.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TradeUpdateData {
    event: String,
    order: Option<TradeUpdateOrder>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    position_qty: Option<String>,
    #[serde(default)]
    price: Option<String>,
    #[serde(default)]
    qty: Option<String>,
}

/// Order details embedded in a trade update.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TradeUpdateOrder {
    id: String,
    client_order_id: Option<String>,
    symbol: Option<String>,
    side: Option<String>,
    #[serde(rename = "type")]
    order_type: Option<String>,
    status: Option<String>,
    qty: Option<String>,
    filled_qty: Option<String>,
    filled_avg_price: Option<String>,
    limit_price: Option<String>,
    stop_price: Option<String>,
    time_in_force: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    submitted_at: Option<String>,
    filled_at: Option<String>,
    expired_at: Option<String>,
    canceled_at: Option<String>,
    order_class: Option<String>,
    extended_hours: Option<bool>,
    legs: Option<Vec<TradeUpdateOrder>>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// ─── Client ─────────────────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

/// Configuration for the trade updates stream.
#[derive(Debug, Clone)]
pub struct TradeUpdateConfig {
    pub api_key: String,
    pub secret_key: String,
    pub paper: bool,
}

impl TradeUpdateConfig {
    /// Create from environment variables.
    pub fn from_env(paper: bool) -> Result<Self> {
        Ok(Self {
            api_key: std::env::var("ALPACA_API_KEY").context("ALPACA_API_KEY not set")?,
            secret_key: std::env::var("ALPACA_SECRET_KEY").context("ALPACA_SECRET_KEY not set")?,
            paper,
        })
    }

    fn ws_url(&self) -> String {
        if self.paper {
            "wss://paper-api.alpaca.markets/stream".to_string()
        } else {
            "wss://api.alpaca.markets/stream".to_string()
        }
    }
}

/// Runs the Alpaca trade updates WebSocket stream.
///
/// Connects, authenticates, subscribes to `trade_updates`, and forwards
/// parsed `OrderEvent` messages through the provided channel.
///
/// Reconnects automatically on connection loss with exponential backoff.
pub async fn run_trade_updates(
    config: TradeUpdateConfig,
    tx: mpsc::UnboundedSender<OrderEvent>,
) -> Result<()> {
    let mut backoff_secs = 1u64;
    let max_backoff = 60u64;

    loop {
        match run_trade_updates_inner(&config, &tx).await {
            Ok(()) => {
                info!("Trade updates stream ended cleanly. Reconnecting...");
                backoff_secs = 1;
            }
            Err(e) => {
                error!(
                    "Trade updates stream error: {:?}. Reconnecting in {}s...",
                    e, backoff_secs
                );
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(max_backoff);
    }
}

async fn run_trade_updates_inner(
    config: &TradeUpdateConfig,
    tx: &mpsc::UnboundedSender<OrderEvent>,
) -> Result<()> {
    let url = config.ws_url();
    info!("Connecting to Alpaca trade updates at {}", url);

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
        .await
        .context("Failed to connect to Alpaca trade updates WebSocket")?;

    info!("Trade updates WebSocket connected.");
    let (mut write, mut read) = ws_stream.split();

    // ── 1. Authenticate ────────────────────────────────────────────────────

    let auth_msg = serde_json::json!({
        "action": "auth",
        "key": config.api_key,
        "secret": config.secret_key,
    });
    write.send(Message::Text(auth_msg.to_string())).await?;
    debug!("Trade updates auth sent.");

    // Wait for auth response
    if let Some(Ok(Message::Text(msg))) = read.next().await {
        match serde_json::from_str::<AuthResponse>(&msg) {
            Ok(resp) => {
                let status = resp
                    .data
                    .as_ref()
                    .and_then(|d| d.status.as_deref())
                    .unwrap_or("unknown");
                if status == "authorized" {
                    info!("Trade updates authenticated successfully.");
                } else {
                    return Err(anyhow::anyhow!(
                        "Trade updates auth failed: status={}",
                        status
                    ));
                }
            }
            Err(e) => {
                warn!("Could not parse auth response: {}. Raw: {}", e, msg);
            }
        }
    }

    // ── 2. Subscribe to trade_updates ───────────────────────────────────────

    let sub_msg = serde_json::json!({
        "action": "listen",
        "data": {
            "streams": ["trade_updates"]
        }
    });
    write.send(Message::Text(sub_msg.to_string())).await?;
    info!("Subscribed to trade_updates stream.");

    // ── 3. Process messages ────────────────────────────────────────────────

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                debug!("Trade update raw: {}", text);

                match serde_json::from_str::<StreamMessage>(&text) {
                    Ok(sm) => {
                        if sm.stream.as_deref() == Some("trade_updates") {
                            if let Some(data) = sm.data {
                                if let Some(event) = parse_trade_update(&data) {
                                    let _ = tx.send(event);
                                }
                            }
                        } else if sm.stream.as_deref() == Some("authorization") {
                            // Already handled
                        } else {
                            debug!("Unknown stream: {:?}", sm.stream);
                        }
                    }
                    Err(e) => {
                        debug!("Unparseable trade update: {} — {}", e, text);
                    }
                }
            }
            Ok(Message::Ping(p)) => {
                let _ = write.send(Message::Pong(p)).await;
            }
            Ok(Message::Close(_)) => {
                info!("Trade updates WebSocket closed by server.");
                break;
            }
            Err(e) => {
                error!("Trade updates WebSocket error: {:?}", e);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Parse a trade update data payload into an `OrderEvent`.
fn parse_trade_update(data: &TradeUpdateData) -> Option<OrderEvent> {
    let order = data.order.as_ref()?;
    let client_id = CompactString::from(order.client_order_id.as_deref().unwrap_or(&order.id));
    let venue_id = CompactString::from(&order.id);

    match data.event.as_str() {
        // ── Order accepted by exchange ──────────────────────────────────
        "new" => {
            info!(
                "[TradeUpdate] Order accepted: {} ({}) status={}",
                client_id,
                order.symbol.as_deref().unwrap_or("?"),
                order.status.as_deref().unwrap_or("?"),
            );
            Some(OrderEvent::Accepted(OrderAccepted {
                client_order_id: client_id,
                venue_order_id: venue_id,
            }))
        }

        // ── Full fill ──────────────────────────────────────────────────
        "fill" => {
            let fill_price = data
                .price
                .as_deref()
                .and_then(|p| p.parse::<f64>().ok())
                .or_else(|| {
                    order
                        .filled_avg_price
                        .as_deref()
                        .and_then(|p| p.parse::<f64>().ok())
                })
                .unwrap_or(0.0);

            let fill_qty = data
                .qty
                .as_deref()
                .and_then(|q| q.parse::<f64>().ok())
                .or_else(|| {
                    order
                        .filled_qty
                        .as_deref()
                        .and_then(|q| q.parse::<f64>().ok())
                })
                .unwrap_or(0.0);

            info!(
                "[TradeUpdate] FILL: {} {} @ ${:.4} ({})",
                client_id,
                fill_qty,
                fill_price,
                order.symbol.as_deref().unwrap_or("?"),
            );

            Some(OrderEvent::Filled(OrderFilled {
                client_order_id: client_id,
                fill_price,
                fill_quantity: fill_qty,
                commission: 0.0, // Alpaca is commission-free
            }))
        }

        // ── Partial fill ───────────────────────────────────────────────
        "partial_fill" => {
            let fill_price = data
                .price
                .as_deref()
                .and_then(|p| p.parse::<f64>().ok())
                .unwrap_or(0.0);

            let fill_qty = data
                .qty
                .as_deref()
                .and_then(|q| q.parse::<f64>().ok())
                .unwrap_or(0.0);

            info!(
                "[TradeUpdate] PARTIAL FILL: {} {} @ ${:.4} ({})",
                client_id,
                fill_qty,
                fill_price,
                order.symbol.as_deref().unwrap_or("?"),
            );

            Some(OrderEvent::Filled(OrderFilled {
                client_order_id: client_id,
                fill_price,
                fill_quantity: fill_qty,
                commission: 0.0,
            }))
        }

        // ── Cancelled ──────────────────────────────────────────────────
        "canceled" => {
            info!(
                "[TradeUpdate] CANCELLED: {} ({})",
                client_id,
                order.symbol.as_deref().unwrap_or("?"),
            );
            Some(OrderEvent::Cancelled(OrderCancelled {
                client_order_id: client_id,
            }))
        }

        // ── Rejected ───────────────────────────────────────────────────
        "rejected" => {
            let reason = order
                .status
                .as_deref()
                .unwrap_or("rejected by exchange")
                .to_string();

            warn!(
                "[TradeUpdate] REJECTED: {} — {} ({})",
                client_id,
                reason,
                order.symbol.as_deref().unwrap_or("?"),
            );

            Some(OrderEvent::Rejected(OrderRejected {
                client_order_id: client_id,
                reason: CompactString::from(reason),
            }))
        }

        // ── Expired ────────────────────────────────────────────────────
        "expired" => {
            info!(
                "[TradeUpdate] EXPIRED: {} ({})",
                client_id,
                order.symbol.as_deref().unwrap_or("?"),
            );
            Some(OrderEvent::Cancelled(OrderCancelled {
                client_order_id: client_id,
            }))
        }

        // ── Replaced (modify order) ────────────────────────────────────
        "replaced" => {
            info!(
                "[TradeUpdate] REPLACED: {} → new venue_id={} ({})",
                client_id,
                venue_id,
                order.symbol.as_deref().unwrap_or("?"),
            );
            Some(OrderEvent::Accepted(OrderAccepted {
                client_order_id: client_id,
                venue_order_id: venue_id,
            }))
        }

        // ── Other lifecycle events (logged, not emitted) ───────────────
        "pending_new"
        | "pending_cancel"
        | "pending_replace"
        | "calculated"
        | "suspended"
        | "order_replace_rejected"
        | "order_cancel_rejected"
        | "stopped" => {
            debug!(
                "[TradeUpdate] {}: {} ({})",
                data.event,
                client_id,
                order.symbol.as_deref().unwrap_or("?"),
            );
            None
        }

        other => {
            warn!("[TradeUpdate] Unknown event type: {}", other);
            None
        }
    }
}
