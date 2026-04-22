#![forbid(unsafe_code)]
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::{Html, Response},
    routing::get,
    Router,
};
use std::net::SocketAddr;
use tokio::sync::broadcast;

pub async fn serve(tx: broadcast::Sender<String>) {
    let app = Router::new().route("/", get(index)).route(
        "/ws",
        get(move |ws: WebSocketUpgrade| handle_ws(ws, tx.clone())),
    );

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind web dashboard to port 8080");
    tracing::info!("web-dashboard listening on {}", addr);
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("Web dashboard server crashed: {:?}", e);
    }
}

async fn index() -> Html<&'static str> {
    Html(include_str!("static/index.html"))
}

async fn handle_ws(ws: WebSocketUpgrade, tx: broadcast::Sender<String>) -> Response {
    ws.on_upgrade(move |mut socket: WebSocket| async move {
        let mut rx = tx.subscribe();
        loop {
            tokio::select! {
                msg = socket.recv() => {
                    if msg.is_none() { break; }
                }
                evt = rx.recv() => {
                    match evt {
                        Ok(payload) => {
                            if socket.send(Message::Text(payload)).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    })
}
