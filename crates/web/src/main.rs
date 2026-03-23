#![forbid(unsafe_code)]
use axum::{routing::get, Router};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let app = Router::new().route("/", get(|| async { "RL Trading Bot Web API" }));
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Web server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
