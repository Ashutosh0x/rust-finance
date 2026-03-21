use crate::source::{MarketDataSource, DataType, Subscription, IngestionError, MarketStream};
use async_trait::async_trait;
use common::time::SequenceGenerator;
use std::sync::Arc;

#[derive(Clone)]
pub struct MockSource { seq_gen: Arc<SequenceGenerator> }

impl MockSource { 
    pub fn new(seq_gen: Arc<SequenceGenerator>) -> Self { Self { seq_gen } } 
}

#[async_trait]
impl MarketDataSource for MockSource {
    fn name(&self) -> &str { "Mock" }
    fn supported_data_types(&self) -> &[DataType] { &[DataType::Trades, DataType::Quotes, DataType::OrderBookL1] }
    async fn connect(&self, _sub: &Subscription) -> Result<MarketStream, IngestionError> {
        let stream = futures::stream::empty::<Result<common::events::Envelope<common::events::MarketEvent>, IngestionError>>();
        Ok(Box::pin(stream))
    }
    async fn is_healthy(&self) -> bool { true }
}
