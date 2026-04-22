// crates/polymarket/src/data.rs
// Data API client — positions, trades, activity, leaderboard
// Base URL: https://data-api.polymarket.com
// All endpoints are public (no authentication required)

use reqwest::Client;
use serde::{Deserialize, Serialize};

// ─── Response Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPosition {
    pub market: Option<String>,
    #[serde(rename = "outcomeIndex")]
    pub outcome_index: Option<String>,
    pub size: Option<String>,
    #[serde(rename = "avgPrice")]
    pub avg_price: Option<String>,
    #[serde(rename = "curPrice")]
    pub cur_price: Option<f64>,
    #[serde(rename = "cashPnl")]
    pub cash_pnl: Option<f64>,
    #[serde(rename = "percentPnl")]
    pub percent_pnl: Option<f64>,
    pub asset: Option<String>,
    #[serde(rename = "proxyWallet")]
    pub proxy_wallet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTrade {
    pub id: Option<String>,
    pub market: Option<String>,
    pub asset: Option<String>,
    pub side: Option<String>,
    pub price: Option<String>,
    pub size: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(rename = "matchedAt")]
    pub matched_at: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub address: Option<String>,
    pub username: Option<String>,
    #[serde(rename = "profileImage")]
    pub profile_image: Option<String>,
    pub bio: Option<String>,
    #[serde(rename = "totalVolume")]
    pub total_volume: Option<f64>,
    #[serde(rename = "positionsValue")]
    pub positions_value: Option<f64>,
    #[serde(rename = "marketsTraded")]
    pub markets_traded: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub rank: Option<i64>,
    pub address: Option<String>,
    pub username: Option<String>,
    pub volume: Option<f64>,
    #[serde(rename = "pnl")]
    pub pnl: Option<f64>,
    #[serde(rename = "marketsTraded")]
    pub markets_traded: Option<i64>,
}

// ─── Client ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DataClient {
    http: Client,
    base_url: String,
}

impl DataClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.to_string(),
        }
    }

    // ─── Profile ─────────────────────────────────────────────────

    /// GET /profile/{address} — get public profile by wallet address
    pub async fn get_profile(&self, address: &str) -> anyhow::Result<UserProfile> {
        let resp = self
            .http
            .get(format!("{}/profile/{}", self.base_url, address))
            .send()
            .await?
            .json::<UserProfile>()
            .await?;
        Ok(resp)
    }

    // ─── Positions ───────────────────────────────────────────────

    /// GET /positions?user={address} — get current positions for a user
    pub async fn get_positions(&self, address: &str) -> anyhow::Result<Vec<UserPosition>> {
        let resp = self
            .http
            .get(format!("{}/positions", self.base_url))
            .query(&[("user", address)])
            .send()
            .await?
            .json::<Vec<UserPosition>>()
            .await?;
        Ok(resp)
    }

    /// GET /positions?user={address}&closed=true — get closed positions
    pub async fn get_closed_positions(&self, address: &str) -> anyhow::Result<Vec<UserPosition>> {
        let resp = self
            .http
            .get(format!("{}/positions", self.base_url))
            .query(&[("user", address), ("closed", "true")])
            .send()
            .await?
            .json::<Vec<UserPosition>>()
            .await?;
        Ok(resp)
    }

    // ─── Trades ──────────────────────────────────────────────────

    /// GET /trades?user={address} — get trades for a user
    pub async fn get_user_trades(
        &self,
        address: &str,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<UserTrade>> {
        let mut params: Vec<(&str, String)> = vec![("user", address.to_string())];
        if let Some(l) = limit {
            params.push(("limit", l.to_string()));
        }

        let resp = self
            .http
            .get(format!("{}/trades", self.base_url))
            .query(&params)
            .send()
            .await?
            .json::<Vec<UserTrade>>()
            .await?;
        Ok(resp)
    }

    /// GET /trades?market={market_id} — get trades for a market
    pub async fn get_market_trades(
        &self,
        market_id: &str,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<UserTrade>> {
        let mut params: Vec<(&str, String)> = vec![("market", market_id.to_string())];
        if let Some(l) = limit {
            params.push(("limit", l.to_string()));
        }

        let resp = self
            .http
            .get(format!("{}/trades", self.base_url))
            .query(&params)
            .send()
            .await?
            .json::<Vec<UserTrade>>()
            .await?;
        Ok(resp)
    }

    // ─── Leaderboard ─────────────────────────────────────────────

    /// GET /leaderboard — get trader leaderboard rankings
    pub async fn get_leaderboard(
        &self,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<LeaderboardEntry>> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(l) = limit {
            params.push(("limit", l.to_string()));
        }

        let resp = self
            .http
            .get(format!("{}/leaderboard", self.base_url))
            .query(&params)
            .send()
            .await?
            .json::<Vec<LeaderboardEntry>>()
            .await?;
        Ok(resp)
    }

    // ─── Open Interest ───────────────────────────────────────────

    /// GET /open-interest?market={market_id} — get open interest for a market
    pub async fn get_open_interest(&self, market_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/open-interest", self.base_url))
            .query(&[("market", market_id)])
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }
}
