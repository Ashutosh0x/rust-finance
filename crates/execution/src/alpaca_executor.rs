use crate::gateway::{ExecutionGateway, OpenRequest, TimeInForce};
use async_trait::async_trait;
use common::events::{OrderAccepted, OrderEvent, OrderSide, OrderType};
use compact_str::CompactString;
use ingestion::alpaca::{AlpacaConfig, AlpacaRestClient, OrderRequest as AlpacaOrderRequest};
use tracing::{error, info};

pub struct AlpacaExecutor {
    client: AlpacaRestClient,
    paper: bool,
}

impl AlpacaExecutor {
    pub fn from_env(paper: bool) -> Result<Self, anyhow::Error> {
        let base_url = if paper {
            std::env::var("ALPACA_BASE_URL")
                .unwrap_or_else(|_| "https://paper-api.alpaca.markets".to_string())
        } else {
            std::env::var("ALPACA_BASE_URL")
                .unwrap_or_else(|_| "https://api.alpaca.markets".to_string())
        };

        let config = AlpacaConfig {
            key_id: std::env::var("ALPACA_API_KEY")
                .map_err(|_| anyhow::anyhow!("ALPACA_API_KEY not set"))?,
            secret_key: std::env::var("ALPACA_SECRET_KEY")
                .map_err(|_| anyhow::anyhow!("ALPACA_SECRET_KEY not set"))?,
            base_url,
            data_url: std::env::var("ALPACA_DATA_URL")
                .unwrap_or_else(|_| "https://data.alpaca.markets".to_string()),
        };

        let client = AlpacaRestClient::new(config)?;
        info!("AlpacaExecutor initialized (paper={})", paper);

        Ok(Self { client, paper })
    }

    /// Get a reference to the underlying REST client for advanced operations
    pub fn rest_client(&self) -> &AlpacaRestClient {
        &self.client
    }

    pub fn is_paper(&self) -> bool {
        self.paper
    }
}

#[async_trait]
impl ExecutionGateway for AlpacaExecutor {
    fn name(&self) -> &str {
        if self.paper {
            "AlpacaExecutor(paper)"
        } else {
            "AlpacaExecutor(live)"
        }
    }

    async fn submit_order(&self, req: OpenRequest) -> Result<OrderEvent, anyhow::Error> {
        req.validate()?;

        let side = match req.side {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        };

        let order_type = match req.order_type {
            OrderType::Market => "market",
            OrderType::Limit => "limit",
        };

        let time_in_force = match req.time_in_force {
            TimeInForce::DAY => "day",
            TimeInForce::GTC => "gtc",
            TimeInForce::IOC => "ioc",
            TimeInForce::FOK => "fok",
        };

        let alpaca_req = AlpacaOrderRequest {
            symbol: req.symbol.to_string(),
            qty: Some(req.quantity),
            notional: None,
            side: side.to_string(),
            order_type: order_type.to_string(),
            time_in_force: time_in_force.to_string(),
            limit_price: req.limit_price,
            stop_price: None,
            trail_price: None,
            trail_percent: None,
            extended_hours: None,
            client_order_id: Some(req.client_order_id.to_string()),
            order_class: None,
            take_profit: None,
            stop_loss: None,
        };

        info!(
            "[{}] Submitting {} {} {} @ {:?} (tif={})",
            self.name(),
            side,
            req.quantity,
            req.symbol,
            req.limit_price,
            time_in_force,
        );

        match self.client.place_order(&alpaca_req).await {
            Ok(order) => {
                info!(
                    "[{}] Order accepted: id={} status={}",
                    self.name(),
                    order.id,
                    order.status
                );

                // Emit Submitted + Accepted events
                Ok(OrderEvent::Accepted(OrderAccepted {
                    client_order_id: req.client_order_id,
                    venue_order_id: CompactString::from(order.id),
                }))
            }
            Err(e) => {
                error!("[{}] Order rejected: {:?}", self.name(), e);
                Err(anyhow::anyhow!("Alpaca order rejected: {}", e))
            }
        }
    }
}
