// crates/web/src/server.rs
//
// Axum web server exposing real-time portfolio data, AI signals,
// and system metrics to the frontend dashboard.

use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, error};
use serde::Serialize;
use common::events::BotEvent; // assuming BotEvent exists in common

#[derive(Clone)]
pub struct AppState {
    /// Broadcast channel receiving events from the Daemon's EventBus.
    pub tx_events: broadcast::Sender<BotEvent>,
}

pub async fn start_server(port: u16, state: AppState) {
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/ws", get(ws_handler))
        .with_state(Arc::new(state));

    let addr = format!("0.0.0.0:{}", port);
    info!("Starting Web Dashboard server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app)
        .await
        .expect("Axum server failed");
}

async fn health_check() -> &'static str {
    "OK"
}

// ── WebSocket Handler ─────────────────────────────────────────────────────────

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.tx_events.subscribe();
    info!("New WebSocket dashboard client connected");

    loop {
        tokio::select! {
            // Forward events from the daemon bus down to the JS client
            Ok(event) = rx.recv() => {
                if let Ok(json) = serde_json::to_string(&event) {
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        info!("Client disconnected");
                        break;
                    }
                }
            }
            
            // Listen for client disconnects / pings
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        info!("Client disconnected");
                        break;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {} // Ignore other messages (e.g. text/binary from client)
                }
            }
        }
    }
}
