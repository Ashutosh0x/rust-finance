use crate::source::{DataType, IngestionError, MarketDataSource, MarketStream, Subscription};
use async_trait::async_trait;
use common::time::SequenceGenerator;
use std::sync::Arc;

#[derive(Clone)]
#[allow(dead_code)]
pub struct AlpacaSource {
    seq_gen: Arc<SequenceGenerator>,
}

impl AlpacaSource {
    pub fn from_env(_seq_gen: Arc<SequenceGenerator>) -> Result<Self, IngestionError> {
        let key = std::env::var("ALPACA_API_KEY")
            .map_err(|_| IngestionError::ConnectionFailed("ALPACA_API_KEY not set".into()))?;
        let secret = std::env::var("ALPACA_SECRET_KEY")
            .map_err(|_| IngestionError::ConnectionFailed("ALPACA_SECRET_KEY not set".into()))?;
        if key.trim().is_empty() || secret.trim().is_empty() {
            return Err(IngestionError::ConnectionFailed(
                "Alpaca credentials cannot be empty".into(),
            ));
        }

        Err(IngestionError::ConnectionFailed(
            "AlpacaSource is not implemented; use alpaca_ws or Finnhub for equities".into(),
        ))
    }
}

#[async_trait]
impl MarketDataSource for AlpacaSource {
    fn name(&self) -> &str {
        "Alpaca"
    }
    fn supported_data_types(&self) -> &[DataType] {
        &[DataType::Trades, DataType::Quotes]
    }
    async fn connect(&self, _sub: &Subscription) -> Result<MarketStream, IngestionError> {
        Err(IngestionError::ConnectionFailed(
            "AlpacaSource is not implemented; use Finnhub/Binance or alpaca_ws instead".into(),
        ))
    }
    async fn is_healthy(&self) -> bool {
        false
    }
}
