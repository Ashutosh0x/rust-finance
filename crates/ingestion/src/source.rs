use async_trait::async_trait;
use common::events::{Envelope, MarketEvent};
use std::pin::Pin;
use futures::Stream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Trades,
    Quotes,
    OrderBookL1,
    OrderBookL2,
    Bars1m,
}

#[derive(Debug, Clone)]
pub struct Subscription {
    pub symbols: Vec<String>,
    pub data_types: Vec<DataType>,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestionError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Stream closed")]
    StreamClosed,
    #[error("Deserialize error: {0}")]
    Deserialize(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type MarketStream = Pin<Box<dyn Stream<Item = Result<Envelope<MarketEvent>, IngestionError>> + Send>>;

#[async_trait]
pub trait MarketDataSource: Send + Sync {
    fn name(&self) -> &str;
    fn supported_data_types(&self) -> &[DataType];
    async fn connect(&self, subscription: &Subscription) -> Result<MarketStream, IngestionError>;
    async fn is_healthy(&self) -> bool;
}
