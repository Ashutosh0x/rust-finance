use anyhow::Result;
use redis::AsyncCommands;
use redis::Client;

pub async fn connect_dragonfly(url: &str) -> Result<redis::aio::MultiplexedConnection> {
    let client = Client::open(url)?;
    let conn = client.get_multiplexed_tokio_connection().await?;
    Ok(conn)
}

pub async fn update_price(
    conn: &mut redis::aio::MultiplexedConnection,
    symbol: &str,
    price: f64,
) -> Result<()> {
    let key = format!("price:{}", symbol);
    let _: () = conn.set(key, price).await?;
    Ok(())
}

pub async fn get_price(conn: &mut redis::aio::MultiplexedConnection, symbol: &str) -> Result<f64> {
    let key = format!("price:{}", symbol);
    let val: f64 = conn.get(key).await.unwrap_or(0.0);
    Ok(val)
}

pub async fn update_pnl(conn: &mut redis::aio::MultiplexedConnection, pnl: f64) -> Result<()> {
    let _: () = conn.set("portfolio:pnl", pnl).await?;
    Ok(())
}
