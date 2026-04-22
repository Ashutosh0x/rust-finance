use crate::clob::{ClobClient, OrderType, Side};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use tokio::time::{interval, Duration};
use tracing::{info, warn};

#[derive(Debug, Deserialize)]
pub struct TargetPosition {
    pub asset_id: String,
    pub market: String,
    pub side: String,
    pub size: String,
    pub price: String,
    pub outcome: String,
}

#[derive(Debug, Deserialize)]
pub struct TargetActivity {
    #[serde(rename = "type")]
    pub activity_type: String, // "trade", "split", "merge", etc.
    pub condition_id: Option<String>,
    pub asset_id: Option<String>,
    pub side: Option<String>,
    pub size: Option<String>,
    pub price: Option<String>,
    pub timestamp: Option<String>,
}

#[allow(dead_code)]
pub struct CopyTrader {
    data_api_url: String,
    http: Client,
    clob: ClobClient,
    target_addresses: Vec<String>,
    copy_size_pct: Decimal,
    /// Track what we've already copied to avoid duplicates
    seen_trades: HashMap<String, bool>,
    dry_run: bool,
}

impl CopyTrader {
    pub fn new(
        data_api_url: &str,
        clob: ClobClient,
        target_addresses: Vec<String>,
        copy_size_pct: f64,
        dry_run: bool,
    ) -> Self {
        Self {
            data_api_url: data_api_url.to_string(),
            http: Client::new(),
            clob,
            target_addresses,
            copy_size_pct: Decimal::try_from(copy_size_pct).unwrap_or(Decimal::TEN),
            seen_trades: HashMap::new(),
            dry_run,
        }
    }

    /// Poll target wallet activity and copy new trades
    pub async fn run_polling_loop(&mut self, poll_interval_secs: u64) {
        let mut tick = interval(Duration::from_secs(poll_interval_secs));

        loop {
            tick.tick().await;

            for address in &self.target_addresses.clone() {
                match self.fetch_recent_activity(address).await {
                    Ok(activities) => {
                        for activity in activities {
                            if activity.activity_type == "trade" {
                                self.maybe_copy_trade(&activity).await;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Error fetching activity for {}: {}", address, e);
                    }
                }
            }
        }
    }

    /// Fetch recent activity from the Data API
    async fn fetch_recent_activity(&self, address: &str) -> anyhow::Result<Vec<TargetActivity>> {
        // Data API: GET /activity?user=<address>&type=trade
        let url = format!("{}/activity", self.data_api_url);
        let activities: Vec<TargetActivity> = self
            .http
            .get(&url)
            .query(&[("user", address), ("type", "trade"), ("limit", "20")])
            .send()
            .await?
            .json()
            .await?;
        Ok(activities)
    }

    /// Fetch current positions of a target wallet
    pub async fn fetch_target_positions(
        &self,
        address: &str,
    ) -> anyhow::Result<Vec<TargetPosition>> {
        let url = format!("{}/positions", self.data_api_url);
        let positions: Vec<TargetPosition> = self
            .http
            .get(&url)
            .query(&[("user", address)])
            .send()
            .await?
            .json()
            .await?;
        Ok(positions)
    }

    /// Copy a trade with scaled size
    async fn maybe_copy_trade(&mut self, activity: &TargetActivity) {
        let trade_key = format!(
            "{}-{}-{}",
            activity.asset_id.as_deref().unwrap_or(""),
            activity.timestamp.as_deref().unwrap_or(""),
            activity.side.as_deref().unwrap_or(""),
        );

        if self.seen_trades.contains_key(&trade_key) {
            return;
        }
        self.seen_trades.insert(trade_key, true);

        let Some(asset_id) = &activity.asset_id else {
            return;
        };
        let Some(side_str) = &activity.side else {
            return;
        };
        let Some(size_str) = &activity.size else {
            return;
        };
        let Some(price_str) = &activity.price else {
            return;
        };

        let size: Decimal = match size_str.parse() {
            Ok(s) => s,
            Err(_) => return,
        };
        let price: Decimal = match price_str.parse() {
            Ok(p) => p,
            Err(_) => return,
        };

        let side = match side_str.to_uppercase().as_str() {
            "BUY" => Side::Buy,
            "SELL" => Side::Sell,
            _ => return,
        };

        // Scale size by copy percentage
        let scaled_size = size * self.copy_size_pct / Decimal::ONE_HUNDRED;

        info!(
            "COPY TRADE: {:?} {} @ {} (scaled from {})",
            side, scaled_size, price, size
        );

        match self
            .clob
            .place_order(&asset_id, side, price, scaled_size, OrderType::GTC, false)
            .await
        {
            Ok(resp) => {
                if resp.success.unwrap_or(false) {
                    info!("Copy trade placed: {:?}", resp.order_id);
                } else {
                    warn!("Copy trade failed: {:?}", resp.error_msg);
                }
            }
            Err(e) => {
                warn!("Error placing copy trade: {}", e);
            }
        }
    }
}
