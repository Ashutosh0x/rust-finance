use anyhow::{Context, Result};
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use tracing::info;

// ═══════════════════════════════════════════════════════════════════
// ─── Config ─────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct AlpacaConfig {
    pub key_id: String,
    pub secret_key: String,
    /// Trading API base URL
    /// Paper: "https://paper-api.alpaca.markets"
    /// Live:  "https://api.alpaca.markets"
    pub base_url: String,
    /// Market Data API base URL
    /// Default: "https://data.alpaca.markets"
    pub data_url: String,
}

impl AlpacaConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            key_id: std::env::var("ALPACA_API_KEY").unwrap_or_default(),
            secret_key: std::env::var("ALPACA_SECRET_KEY").unwrap_or_default(),
            base_url: std::env::var("ALPACA_BASE_URL")
                .unwrap_or_else(|_| "https://paper-api.alpaca.markets".to_string()),
            data_url: std::env::var("ALPACA_DATA_URL")
                .unwrap_or_else(|_| "https://data.alpaca.markets".to_string()),
        })
    }
}

// ═══════════════════════════════════════════════════════════════════
// ─── Trading API Models ─────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Account {
    pub id: String,
    pub status: String,
    pub currency: String,
    pub buying_power: String,
    pub regt_buying_power: String,
    pub daytrading_buying_power: String,
    pub non_marginable_buying_power: String,
    pub cash: String,
    pub portfolio_value: String,
    #[serde(default)]
    pub pattern_day_trader: bool,
    #[serde(default)]
    pub trading_blocked: bool,
    #[serde(default)]
    pub transfers_blocked: bool,
    #[serde(default)]
    pub account_blocked: bool,
    #[serde(default)]
    pub shorting_enabled: bool,
    pub long_market_value: Option<String>,
    pub short_market_value: Option<String>,
    pub equity: Option<String>,
    pub last_equity: Option<String>,
    pub multiplier: Option<String>,
    pub initial_margin: Option<String>,
    pub maintenance_margin: Option<String>,
    pub last_maintenance_margin: Option<String>,
    pub sma: Option<String>,
    pub daytrade_count: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Position {
    pub asset_id: String,
    pub symbol: String,
    pub exchange: Option<String>,
    pub asset_class: Option<String>,
    pub asset_marginable: Option<bool>,
    pub qty: String,
    pub avg_entry_price: String,
    pub side: Option<String>,
    pub market_value: Option<String>,
    pub cost_basis: Option<String>,
    pub current_price: String,
    pub lastday_price: Option<String>,
    pub change_today: Option<String>,
    pub unrealized_pl: String,
    pub unrealized_plpc: String,
    pub unrealized_intraday_pl: Option<String>,
    pub unrealized_intraday_plpc: Option<String>,
    pub qty_available: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct OrderRequest {
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notional: Option<f64>,
    pub side: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub time_in_force: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trail_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trail_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_hours: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit: Option<TakeProfitParams>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss: Option<StopLossParams>,
}

#[derive(Serialize, Debug, Clone)]
pub struct TakeProfitParams {
    pub limit_price: f64,
}

#[derive(Serialize, Debug, Clone)]
pub struct StopLossParams {
    pub stop_price: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Order {
    pub id: String,
    pub client_order_id: Option<String>,
    pub symbol: String,
    pub asset_id: Option<String>,
    pub qty: Option<String>,
    pub notional: Option<String>,
    pub filled_qty: Option<String>,
    pub filled_avg_price: Option<String>,
    pub status: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub submitted_at: Option<String>,
    pub filled_at: Option<String>,
    pub expired_at: Option<String>,
    pub canceled_at: Option<String>,
    #[serde(rename = "type")]
    pub order_type: Option<String>,
    pub side: Option<String>,
    pub time_in_force: Option<String>,
    pub limit_price: Option<String>,
    pub stop_price: Option<String>,
    pub trail_price: Option<String>,
    pub trail_percent: Option<String>,
    pub extended_hours: Option<bool>,
    pub order_class: Option<String>,
    pub legs: Option<Vec<Order>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Asset {
    pub id: String,
    pub class: Option<String>,
    pub exchange: Option<String>,
    pub symbol: String,
    pub name: Option<String>,
    pub status: Option<String>,
    pub tradable: Option<bool>,
    pub marginable: Option<bool>,
    pub shortable: Option<bool>,
    pub easy_to_borrow: Option<bool>,
    pub fractionable: Option<bool>,
    pub min_order_size: Option<String>,
    pub min_trade_increment: Option<String>,
    pub price_increment: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Clock {
    pub timestamp: String,
    pub is_open: bool,
    pub next_open: String,
    pub next_close: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Calendar {
    pub date: String,
    pub open: String,
    pub close: String,
    pub session_open: Option<String>,
    pub session_close: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PortfolioHistory {
    pub timestamp: Vec<i64>,
    pub equity: Vec<f64>,
    pub profit_loss: Vec<f64>,
    pub profit_loss_pct: Vec<f64>,
    pub base_value: f64,
    pub timeframe: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AccountActivity {
    pub id: String,
    pub activity_type: String,
    pub symbol: Option<String>,
    pub side: Option<String>,
    pub qty: Option<String>,
    pub price: Option<String>,
    pub cum_qty: Option<String>,
    pub leaves_qty: Option<String>,
    pub order_id: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    pub net_amount: Option<String>,
    pub per_share_amount: Option<String>,
    pub transaction_time: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
// ─── Market Data Models (Historical) ────────────────────────────
// ═══════════════════════════════════════════════════════════════════

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Bar {
    #[serde(rename = "t")]
    pub timestamp: String,
    #[serde(rename = "o")]
    pub open: f64,
    #[serde(rename = "h")]
    pub high: f64,
    #[serde(rename = "l")]
    pub low: f64,
    #[serde(rename = "c")]
    pub close: f64,
    #[serde(rename = "v")]
    pub volume: f64,
    #[serde(rename = "n")]
    pub trade_count: Option<u64>,
    #[serde(rename = "vw")]
    pub vwap: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Trade {
    #[serde(rename = "t")]
    pub timestamp: String,
    #[serde(rename = "p")]
    pub price: f64,
    #[serde(rename = "s")]
    pub size: f64,
    #[serde(rename = "x")]
    pub exchange: Option<String>,
    #[serde(rename = "i")]
    pub id: Option<u64>,
    #[serde(rename = "c")]
    pub conditions: Option<Vec<String>>,
    #[serde(rename = "z")]
    pub tape: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Quote {
    #[serde(rename = "t")]
    pub timestamp: String,
    #[serde(rename = "bp")]
    pub bid_price: f64,
    #[serde(rename = "bs")]
    pub bid_size: f64,
    #[serde(rename = "bx")]
    pub bid_exchange: Option<String>,
    #[serde(rename = "ap")]
    pub ask_price: f64,
    #[serde(rename = "as")]
    pub ask_size: f64,
    #[serde(rename = "ax")]
    pub ask_exchange: Option<String>,
    #[serde(rename = "c")]
    pub conditions: Option<Vec<String>>,
    #[serde(rename = "z")]
    pub tape: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Snapshot {
    #[serde(rename = "latestTrade")]
    pub latest_trade: Option<Trade>,
    #[serde(rename = "latestQuote")]
    pub latest_quote: Option<Quote>,
    #[serde(rename = "minuteBar")]
    pub minute_bar: Option<Bar>,
    #[serde(rename = "dailyBar")]
    pub daily_bar: Option<Bar>,
    #[serde(rename = "prevDailyBar")]
    pub prev_daily_bar: Option<Bar>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BarsResponse {
    pub bars: Vec<Bar>,
    pub symbol: Option<String>,
    pub next_page_token: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MultiBarsResponse {
    pub bars: std::collections::HashMap<String, Vec<Bar>>,
    pub next_page_token: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TradesResponse {
    pub trades: Vec<Trade>,
    pub symbol: Option<String>,
    pub next_page_token: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct QuotesResponse {
    pub quotes: Vec<Quote>,
    pub symbol: Option<String>,
    pub next_page_token: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
// ─── Client ─────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════

pub struct AlpacaRestClient {
    client: Client,
    base_url: String,
    data_url: String,
}

impl AlpacaRestClient {
    pub fn new(config: AlpacaConfig) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            "APCA-API-KEY-ID",
            header::HeaderValue::from_str(&config.key_id)?,
        );
        headers.insert(
            "APCA-API-SECRET-KEY",
            header::HeaderValue::from_str(&config.secret_key)?,
        );

        let client = Client::builder()
            .default_headers(headers)
            .build()
            .context("Failed to build Alpaca reqwest Client")?;

        Ok(Self {
            client,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            data_url: config.data_url.trim_end_matches('/').to_string(),
        })
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Account ────────────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /v2/account
    pub async fn get_account(&self) -> Result<Account> {
        let url = format!("{}/v2/account", self.base_url);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/account/portfolio/history
    pub async fn get_portfolio_history(
        &self,
        period: Option<&str>,
        timeframe: Option<&str>,
    ) -> Result<PortfolioHistory> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(p) = period {
            params.push(("period", p.to_string()));
        }
        if let Some(tf) = timeframe {
            params.push(("timeframe", tf.to_string()));
        }

        let url = format!("{}/v2/account/portfolio/history", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/account/activities
    pub async fn get_activities(
        &self,
        activity_type: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<AccountActivity>> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(t) = activity_type {
            params.push(("activity_type", t.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit", l.to_string()));
        }

        let url = format!("{}/v2/account/activities", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Orders ─────────────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// POST /v2/orders — submit order
    pub async fn place_order(&self, req: &OrderRequest) -> Result<Order> {
        let url = format!("{}/v2/orders", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(req)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/orders — list orders
    pub async fn list_orders(
        &self,
        status: Option<&str>,
        limit: Option<u32>,
        after: Option<&str>,
        until: Option<&str>,
        direction: Option<&str>,
        symbols: Option<&[&str]>,
    ) -> Result<Vec<Order>> {
        let mut params: Vec<(String, String)> = Vec::new();
        if let Some(s) = status {
            params.push(("status".into(), s.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit".into(), l.to_string()));
        }
        if let Some(a) = after {
            params.push(("after".into(), a.to_string()));
        }
        if let Some(u) = until {
            params.push(("until".into(), u.to_string()));
        }
        if let Some(d) = direction {
            params.push(("direction".into(), d.to_string()));
        }
        if let Some(syms) = symbols {
            params.push(("symbols".into(), syms.join(",")));
        }

        let url = format!("{}/v2/orders", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/orders/{order_id}
    pub async fn get_order(&self, order_id: &str) -> Result<Order> {
        let url = format!("{}/v2/orders/{}", self.base_url, order_id);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// DELETE /v2/orders/{order_id} — cancel specific order
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        let url = format!("{}/v2/orders/{}", self.base_url, order_id);
        self.client.delete(&url).send().await?.error_for_status()?;
        info!("Cancelled order: {}", order_id);
        Ok(())
    }

    /// DELETE /v2/orders — cancel all orders
    pub async fn cancel_all_orders(&self) -> Result<()> {
        let url = format!("{}/v2/orders", self.base_url);
        self.client.delete(&url).send().await?.error_for_status()?;
        info!("Cancelled all open orders");
        Ok(())
    }

    /// PATCH /v2/orders/{order_id} — replace/modify order
    pub async fn replace_order(
        &self,
        order_id: &str,
        qty: Option<f64>,
        limit_price: Option<f64>,
        stop_price: Option<f64>,
        time_in_force: Option<&str>,
    ) -> Result<Order> {
        let mut body = serde_json::Map::new();
        if let Some(q) = qty {
            body.insert("qty".into(), serde_json::json!(q));
        }
        if let Some(lp) = limit_price {
            body.insert("limit_price".into(), serde_json::json!(lp));
        }
        if let Some(sp) = stop_price {
            body.insert("stop_price".into(), serde_json::json!(sp));
        }
        if let Some(tif) = time_in_force {
            body.insert("time_in_force".into(), serde_json::json!(tif));
        }

        let url = format!("{}/v2/orders/{}", self.base_url, order_id);
        let resp = self
            .client
            .patch(&url)
            .json(&serde_json::Value::Object(body))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Positions ──────────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /v2/positions — all open positions
    pub async fn get_positions(&self) -> Result<Vec<Position>> {
        let url = format!("{}/v2/positions", self.base_url);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/positions/{symbol}
    pub async fn get_position(&self, symbol: &str) -> Result<Position> {
        let url = format!("{}/v2/positions/{}", self.base_url, symbol);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// DELETE /v2/positions/{symbol} — close position
    pub async fn close_position(&self, symbol: &str, qty: Option<f64>) -> Result<Order> {
        let url = format!("{}/v2/positions/{}", self.base_url, symbol);
        let mut req = self.client.delete(&url);
        if let Some(q) = qty {
            req = req.query(&[("qty", q.to_string())]);
        }
        let resp = req.send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// DELETE /v2/positions — close all positions
    pub async fn close_all_positions(&self) -> Result<Vec<Order>> {
        let url = format!("{}/v2/positions", self.base_url);
        let resp = self.client.delete(&url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Assets ─────────────────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /v2/assets
    pub async fn list_assets(
        &self,
        status: Option<&str>,
        asset_class: Option<&str>,
        exchange: Option<&str>,
    ) -> Result<Vec<Asset>> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(s) = status {
            params.push(("status", s.to_string()));
        }
        if let Some(c) = asset_class {
            params.push(("asset_class", c.to_string()));
        }
        if let Some(e) = exchange {
            params.push(("exchange", e.to_string()));
        }

        let url = format!("{}/v2/assets", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/assets/{symbol_or_id}
    pub async fn get_asset(&self, symbol: &str) -> Result<Asset> {
        let url = format!("{}/v2/assets/{}", self.base_url, symbol);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Clock & Calendar ───────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /v2/clock — market status
    pub async fn get_clock(&self) -> Result<Clock> {
        let url = format!("{}/v2/clock", self.base_url);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/calendar — trading calendar
    pub async fn get_calendar(
        &self,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<Vec<Calendar>> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(s) = start {
            params.push(("start", s.to_string()));
        }
        if let Some(e) = end {
            params.push(("end", e.to_string()));
        }

        let url = format!("{}/v2/calendar", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Market Data — Historical Stocks ────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /v2/stocks/{symbol}/bars — historical stock bars
    pub async fn get_stock_bars(
        &self,
        symbol: &str,
        timeframe: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: Option<u32>,
        feed: Option<&str>,
    ) -> Result<BarsResponse> {
        let mut params: Vec<(&str, String)> = vec![("timeframe", timeframe.to_string())];
        if let Some(s) = start {
            params.push(("start", s.to_string()));
        }
        if let Some(e) = end {
            params.push(("end", e.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit", l.to_string()));
        }
        if let Some(f) = feed {
            params.push(("feed", f.to_string()));
        }

        let url = format!("{}/v2/stocks/{}/bars", self.data_url, symbol);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/stocks/bars — multi-symbol bars
    pub async fn get_multi_stock_bars(
        &self,
        symbols: &[&str],
        timeframe: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: Option<u32>,
        feed: Option<&str>,
    ) -> Result<MultiBarsResponse> {
        let mut params: Vec<(String, String)> = vec![
            ("symbols".into(), symbols.join(",")),
            ("timeframe".into(), timeframe.to_string()),
        ];
        if let Some(s) = start {
            params.push(("start".into(), s.to_string()));
        }
        if let Some(e) = end {
            params.push(("end".into(), e.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit".into(), l.to_string()));
        }
        if let Some(f) = feed {
            params.push(("feed".into(), f.to_string()));
        }

        let url = format!("{}/v2/stocks/bars", self.data_url);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/stocks/{symbol}/trades — historical trades
    pub async fn get_stock_trades(
        &self,
        symbol: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: Option<u32>,
        feed: Option<&str>,
    ) -> Result<TradesResponse> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(s) = start {
            params.push(("start", s.to_string()));
        }
        if let Some(e) = end {
            params.push(("end", e.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit", l.to_string()));
        }
        if let Some(f) = feed {
            params.push(("feed", f.to_string()));
        }

        let url = format!("{}/v2/stocks/{}/trades", self.data_url, symbol);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/stocks/{symbol}/quotes — historical quotes
    pub async fn get_stock_quotes(
        &self,
        symbol: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: Option<u32>,
        feed: Option<&str>,
    ) -> Result<QuotesResponse> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(s) = start {
            params.push(("start", s.to_string()));
        }
        if let Some(e) = end {
            params.push(("end", e.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit", l.to_string()));
        }
        if let Some(f) = feed {
            params.push(("feed", f.to_string()));
        }

        let url = format!("{}/v2/stocks/{}/quotes", self.data_url, symbol);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/stocks/{symbol}/snapshot — latest snapshot
    pub async fn get_stock_snapshot(&self, symbol: &str, feed: Option<&str>) -> Result<Snapshot> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(f) = feed {
            params.push(("feed", f.to_string()));
        }

        let url = format!("{}/v2/stocks/{}/snapshot", self.data_url, symbol);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v2/stocks/snapshots — multi-symbol snapshots
    pub async fn get_stock_snapshots(
        &self,
        symbols: &[&str],
        feed: Option<&str>,
    ) -> Result<std::collections::HashMap<String, Snapshot>> {
        let mut params: Vec<(String, String)> = vec![("symbols".into(), symbols.join(","))];
        if let Some(f) = feed {
            params.push(("feed".into(), f.to_string()));
        }

        let url = format!("{}/v2/stocks/snapshots", self.data_url);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Market Data — Crypto ───────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /v1beta3/crypto/us/bars — crypto bars
    pub async fn get_crypto_bars(
        &self,
        symbol: &str,
        timeframe: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: Option<u32>,
    ) -> Result<BarsResponse> {
        let mut params: Vec<(&str, String)> = vec![
            ("symbols", symbol.to_string()),
            ("timeframe", timeframe.to_string()),
        ];
        if let Some(s) = start {
            params.push(("start", s.to_string()));
        }
        if let Some(e) = end {
            params.push(("end", e.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit", l.to_string()));
        }

        let url = format!("{}/v1beta3/crypto/us/bars", self.data_url);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v1beta3/crypto/us/latest/bars — latest crypto bar
    pub async fn get_latest_crypto_bar(&self, symbol: &str) -> Result<serde_json::Value> {
        let url = format!("{}/v1beta3/crypto/us/latest/bars", self.data_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("symbols", symbol)])
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// GET /v1beta3/crypto/us/snapshots — crypto snapshots
    pub async fn get_crypto_snapshots(&self, symbols: &[&str]) -> Result<serde_json::Value> {
        let url = format!("{}/v1beta3/crypto/us/snapshots", self.data_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("symbols", symbols.join(","))])
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════════════════════════════
    // ─── Market Data — News ─────────────────────────────────────
    // ═══════════════════════════════════════════════════════════════

    /// GET /v1beta1/news — historical news
    pub async fn get_news(
        &self,
        symbols: Option<&[&str]>,
        start: Option<&str>,
        end: Option<&str>,
        limit: Option<u32>,
    ) -> Result<serde_json::Value> {
        let mut params: Vec<(String, String)> = Vec::new();
        if let Some(syms) = symbols {
            params.push(("symbols".into(), syms.join(",")));
        }
        if let Some(s) = start {
            params.push(("start".into(), s.to_string()));
        }
        if let Some(e) = end {
            params.push(("end".into(), e.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit".into(), l.to_string()));
        }

        let url = format!("{}/v1beta1/news", self.data_url);
        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }
}
