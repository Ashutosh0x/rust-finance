use crate::source::{MarketDataSource, DataType, Subscription, IngestionError, MarketStream};
use async_trait::async_trait;
use common::time::SequenceGenerator;
use std::sync::Arc;

#[derive(Clone)]
pub struct AlpacaSource { seq_gen: Arc<SequenceGenerator> }

impl AlpacaSource { 
    pub fn from_env(seq_gen: Arc<SequenceGenerator>) -> Result<Self, IngestionError> { 
        Ok(Self { seq_gen }) 
    } 
}

#[async_trait]
impl MarketDataSource for AlpacaSource {
    fn name(&self) -> &str { "Alpaca" }
    fn supported_data_types(&self) -> &[DataType] { &[DataType::Trades, DataType::Quotes] }
    async fn connect(&self, _sub: &Subscription) -> Result<MarketStream, IngestionError> {
        let stream = futures::stream::empty::<Result<common::events::Envelope<common::events::MarketEvent>, IngestionError>>();
        Ok(Box::pin(stream))
    }
    async fn is_healthy(&self) -> bool { true }
}
