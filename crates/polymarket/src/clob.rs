// crates/polymarket/src/clob.rs

use crate::auth::{ApiCredentials, L1Auth, L2Auth};
use crate::config::PolymarketConfig;
use crate::signing::{create_wallet, sign_order, Order};
use ethers_signers::Signer;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn as_str(&self) -> &str {
        match self {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        }
    }

    /// Polymarket uses 0 for BUY, 1 for SELL in the order struct
    pub fn as_u8(&self) -> u8 {
        match self {
            Side::Buy => 0,
            Side::Sell => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderType {
    /// Good-Til-Cancelled (limit order, rests on book)
    GTC,
    /// Fill-Or-Kill (market order, fills immediately or cancels)
    FOK,
    /// Good-Til-Date (limit order with expiration)
    GTD,
}

impl OrderType {
    pub fn as_str(&self) -> &str {
        match self {
            OrderType::GTC => "GTC",
            OrderType::FOK => "FOK",
            OrderType::GTD => "GTD",
        }
    }
}

// ─── API Response Types ───

#[derive(Debug, Deserialize)]
pub struct OrderBookResponse {
    pub market: Option<String>,
    pub asset_id: Option<String>,
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub hash: Option<String>,
    pub timestamp: Option<String>,
    pub min_order_size: Option<String>,
    pub tick_size: Option<String>,
    pub neg_risk: Option<bool>,
    pub last_trade_price: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BookLevel {
    pub price: String,
    pub size: String,
}

#[derive(Debug, Deserialize)]
pub struct MidpointResponse {
    pub mid: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PriceResponse {
    pub price: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PostOrderResponse {
    pub success: Option<bool>,
    #[serde(rename = "orderID")]
    pub order_id: Option<String>,
    pub error_msg: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenOrder {
    pub id: String,
    pub asset_id: String,
    pub market: Option<String>,
    pub side: String,
    pub price: String,
    pub original_size: String,
    pub size_matched: String,
    pub status: String,
    #[serde(rename = "type")]
    pub order_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct PostOrderBody {
    pub order: Order,
    pub signature: String,
    #[serde(rename = "orderType")]
    pub order_type: String,
    pub owner: String,
}

// ─── Main Client ───

pub struct ClobClient {
    http: Client,
    host: String,
    wallet: ethers_signers::LocalWallet,
    l2_auth: L2Auth,
    funder_address: String,
    signature_type: u8,
    dry_run: bool,
}

impl ClobClient {
    /// Initialize the CLOB client with full L1→L2 auth flow
    pub async fn new(config: &PolymarketConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let wallet = create_wallet(&config.private_key)
            .map_err(|e| format!("Failed to create wallet: {}", e))?;

        // Check if we have stored L2 credentials
        let credentials = if let (Ok(key), Ok(secret), Ok(pass)) = (
            std::env::var("POLYMARKET_API_KEY"),
            std::env::var("POLYMARKET_API_SECRET"),
            std::env::var("POLYMARKET_API_PASSPHRASE"),
        ) {
            info!("Using stored L2 API credentials");
            ApiCredentials {
                api_key: key,
                api_secret: secret,
                api_passphrase: pass,
            }
        } else {
            // Derive via L1 auth
            info!("Deriving L2 API credentials via L1 auth...");
            let l1 = L1Auth::new(wallet.clone());
            let creds = l1.derive_api_credentials(&config.clob_host).await?;
            info!(
                "Derived L2 API credentials; store them in a secret manager or .env outside logs"
            );
            creds
        };

        let l2_auth = L2Auth::new(credentials);

        Ok(Self {
            http: Client::new(),
            host: config.clob_host.clone(),
            wallet,
            l2_auth,
            funder_address: config.funder_address.clone(),
            signature_type: config.signature_type,
            dry_run: config.dry_run,
        })
    }

    // ─── PUBLIC (Unauthenticated) Endpoints ───

    /// GET /book — fetch orderbook for a token
    #[instrument(skip(self))]
    pub async fn get_orderbook(&self, token_id: &str) -> Result<OrderBookResponse, reqwest::Error> {
        self.http
            .get(format!("{}/book", self.host))
            .query(&[("token_id", token_id)])
            .send()
            .await?
            .json()
            .await
    }

    /// POST /books — batch fetch orderbooks for multiple tokens
    pub async fn get_orderbooks(
        &self,
        token_ids: &[&str],
    ) -> Result<Vec<OrderBookResponse>, Box<dyn std::error::Error>> {
        let body: Vec<serde_json::Value> = token_ids
            .iter()
            .map(|id| serde_json::json!({"token_id": id}))
            .collect();
        let resp = self
            .http
            .post(format!("{}/books", self.host))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        Ok(resp)
    }

    /// GET /midpoint — fetch midpoint price
    pub async fn get_midpoint(
        &self,
        token_id: &str,
    ) -> Result<Option<Decimal>, Box<dyn std::error::Error>> {
        let resp: MidpointResponse = self
            .http
            .get(format!("{}/midpoint", self.host))
            .query(&[("token_id", token_id)])
            .send()
            .await?
            .json()
            .await?;

        Ok(resp.mid.and_then(|m| m.parse::<Decimal>().ok()))
    }

    /// GET /price — get best price for a side (BUY or SELL)
    pub async fn get_price(
        &self,
        token_id: &str,
        side: Side,
    ) -> Result<Option<Decimal>, Box<dyn std::error::Error>> {
        let resp: PriceResponse = self
            .http
            .get(format!("{}/price", self.host))
            .query(&[("token_id", token_id), ("side", side.as_str())])
            .send()
            .await?
            .json()
            .await?;

        Ok(resp.price.and_then(|p| p.parse::<Decimal>().ok()))
    }

    /// GET /spread — get bid-ask spread for a token
    pub async fn get_spread(
        &self,
        token_id: &str,
    ) -> Result<Option<Decimal>, Box<dyn std::error::Error>> {
        let resp: serde_json::Value = self
            .http
            .get(format!("{}/spread", self.host))
            .query(&[("token_id", token_id)])
            .send()
            .await?
            .json()
            .await?;
        Ok(resp
            .get("spread")
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse::<Decimal>().ok()))
    }

    /// GET /last-trade-price — get last trade price and side
    pub async fn get_last_trade_price(
        &self,
        token_id: &str,
    ) -> Result<Option<Decimal>, Box<dyn std::error::Error>> {
        let resp: serde_json::Value = self
            .http
            .get(format!("{}/last-trade-price", self.host))
            .query(&[("token_id", token_id)])
            .send()
            .await?
            .json()
            .await?;
        Ok(resp
            .get("price")
            .and_then(|p| p.as_str())
            .and_then(|p| p.parse::<Decimal>().ok()))
    }

    /// GET /tick-size — get minimum price increment for a token
    pub async fn get_tick_size(
        &self,
        token_id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let resp: serde_json::Value = self
            .http
            .get(format!("{}/tick-size", self.host))
            .query(&[("token_id", token_id)])
            .send()
            .await?
            .json()
            .await?;
        Ok(resp
            .get("minimum_tick_size")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string()))
    }

    /// GET /fee-rate — get base fee rate for a token
    pub async fn get_fee_rate(
        &self,
        token_id: &str,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let resp = self
            .http
            .get(format!("{}/fee-rate", self.host))
            .query(&[("token_id", token_id)])
            .send()
            .await?
            .json()
            .await?;
        Ok(resp)
    }

    /// GET /prices-history — get historical prices for a market
    /// interval: "1h" | "6h" | "1d" | "1w" | "1m" | "all" | "max"
    /// fidelity: accuracy in minutes (default 1)
    pub async fn get_prices_history(
        &self,
        token_id: &str,
        interval: Option<&str>,
        fidelity: Option<u32>,
        start_ts: Option<f64>,
        end_ts: Option<f64>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let mut params: Vec<(&str, String)> = vec![("market", token_id.to_string())];
        if let Some(i) = interval {
            params.push(("interval", i.to_string()));
        }
        if let Some(f) = fidelity {
            params.push(("fidelity", f.to_string()));
        }
        if let Some(s) = start_ts {
            params.push(("startTs", s.to_string()));
        }
        if let Some(e) = end_ts {
            params.push(("endTs", e.to_string()));
        }

        let resp = self
            .http
            .get(format!("{}/prices-history", self.host))
            .query(&params)
            .send()
            .await?
            .json()
            .await?;
        Ok(resp)
    }

    /// GET /markets — list all markets
    pub async fn get_markets(&self) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        self.http
            .get(format!("{}/markets", self.host))
            .send()
            .await?
            .json()
            .await
    }

    /// GET /markets/{condition_id} — get specific market
    pub async fn get_market(
        &self,
        condition_id: &str,
    ) -> Result<serde_json::Value, reqwest::Error> {
        self.http
            .get(format!("{}/markets/{}", self.host, condition_id))
            .send()
            .await?
            .json()
            .await
    }

    // ─── AUTHENTICATED Endpoints ───

    /// POST /order — place a signed order
    #[instrument(skip(self))]
    pub async fn place_order(
        &self,
        token_id: &str,
        side: Side,
        price: Decimal,
        size: Decimal,
        order_type: OrderType,
        neg_risk: bool,
    ) -> Result<PostOrderResponse, Box<dyn std::error::Error>> {
        validate_order_inputs(token_id, price, size)?;

        if self.dry_run {
            info!(
                "DRY RUN: {:?} {} @ {} ({}) token={}",
                side,
                size,
                price,
                order_type.as_str(),
                token_id
            );
            return Ok(PostOrderResponse {
                success: Some(true),
                order_id: Some(format!("dry-run-{}", Uuid::new_v4())),
                error_msg: None,
                status: Some("DRY_RUN".to_string()),
            });
        }

        // Calculate maker/taker amounts from price and size
        // For BUY: maker_amount = price * size (USDC you pay)
        //          taker_amount = size (tokens you receive)
        // For SELL: maker_amount = size (tokens you give)
        //           taker_amount = price * size (USDC you receive)
        let (maker_amount, taker_amount) = match side {
            Side::Buy => {
                let usdc = price * size;
                // Convert to 6-decimal USDC units
                let maker = decimal_to_base_units(usdc)?;
                let taker = decimal_to_base_units(size)?;
                (maker, taker)
            }
            Side::Sell => {
                let usdc = price * size;
                let maker = decimal_to_base_units(size)?;
                let taker = decimal_to_base_units(usdc)?;
                (maker, taker)
            }
        };

        let signer_address = format!("{:?}", self.wallet.address());
        let salt = Uuid::new_v4().as_u128().to_string();

        let order = Order {
            salt,
            maker: self.funder_address.clone(),
            signer: signer_address,
            taker: "0x0000000000000000000000000000000000000000".to_string(),
            token_id: token_id.to_string(),
            maker_amount,
            taker_amount,
            expiration: "0".to_string(),
            nonce: "0".to_string(),
            fee_rate_bps: "0".to_string(),
            side: side.as_u8().to_string(),
            signature_type: self.signature_type.to_string(),
        };

        // Sign the order via EIP-712
        let signature = sign_order(&self.wallet, &order, neg_risk)
            .await
            .map_err(|e| format!("Signing failed: {}", e))?;

        let body = PostOrderBody {
            order,
            signature,
            order_type: order_type.as_str().to_string(),
            owner: self.funder_address.clone(),
        };

        let body_json = serde_json::to_string(&body)?;
        let path = "/order";

        // Build L2 auth headers
        let headers = self.l2_auth.build_headers("POST", path, &body_json)?;

        let resp = self
            .http
            .post(format!("{}{}", self.host, path))
            .headers(headers)
            .body(body_json)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            error!("Order placement failed: {} - {}", status, error_body);
            return Ok(PostOrderResponse {
                success: Some(false),
                order_id: None,
                error_msg: Some(format!("{}: {}", status, error_body)),
                status: Some("FAILED".to_string()),
            });
        }

        let response: PostOrderResponse = resp.json().await?;
        info!("Order placed: {:?}", response.order_id);
        Ok(response)
    }

    /// DELETE /order/{orderId} — cancel a specific order
    pub async fn cancel_order(&self, order_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        if self.dry_run {
            info!("DRY RUN: cancel order {}", order_id);
            return Ok(());
        }

        let path = format!("/order/{}", order_id);
        let headers = self.l2_auth.build_headers("DELETE", &path, "")?;

        let resp = self
            .http
            .delete(format!("{}{}", self.host, path))
            .headers(headers)
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!("Cancel failed: {}", body);
        }

        Ok(())
    }

    /// DELETE /cancel-all — cancel all open orders
    pub async fn cancel_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.dry_run {
            info!("DRY RUN: cancel all orders");
            return Ok(());
        }

        let path = "/cancel-all";
        let headers = self.l2_auth.build_headers("DELETE", path, "")?;

        self.http
            .delete(format!("{}{}", self.host, path))
            .headers(headers)
            .send()
            .await?;

        Ok(())
    }

    /// GET /orders — list open orders
    pub async fn get_open_orders(&self) -> Result<Vec<OpenOrder>, Box<dyn std::error::Error>> {
        let path = "/orders";
        let headers = self.l2_auth.build_headers("GET", path, "")?;

        let orders: Vec<OpenOrder> = self
            .http
            .get(format!("{}{}", self.host, path))
            .headers(headers)
            .send()
            .await?
            .json()
            .await?;

        Ok(orders)
    }

    /// GET /balance-allowance — check USDC balance
    pub async fn get_balance(&self) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let path = "/balance-allowance";
        let headers = self.l2_auth.build_headers("GET", path, "")?;

        let balance: serde_json::Value = self
            .http
            .get(format!("{}{}", self.host, path))
            .headers(headers)
            .send()
            .await?
            .json()
            .await?;

        Ok(balance)
    }

    // ─── Convenience Methods ───

    /// Place a GTC limit buy order
    pub async fn limit_buy(
        &self,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        neg_risk: bool,
    ) -> Result<PostOrderResponse, Box<dyn std::error::Error>> {
        self.place_order(token_id, Side::Buy, price, size, OrderType::GTC, neg_risk)
            .await
    }

    /// Place a GTC limit sell order
    pub async fn limit_sell(
        &self,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        neg_risk: bool,
    ) -> Result<PostOrderResponse, Box<dyn std::error::Error>> {
        self.place_order(token_id, Side::Sell, price, size, OrderType::GTC, neg_risk)
            .await
    }

    /// Place a FOK market buy order (fills immediately or cancels)
    pub async fn market_buy(
        &self,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        neg_risk: bool,
    ) -> Result<PostOrderResponse, Box<dyn std::error::Error>> {
        self.place_order(token_id, Side::Buy, price, size, OrderType::FOK, neg_risk)
            .await
    }

    /// Place a FOK market sell order
    pub async fn market_sell(
        &self,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        neg_risk: bool,
    ) -> Result<PostOrderResponse, Box<dyn std::error::Error>> {
        self.place_order(token_id, Side::Sell, price, size, OrderType::FOK, neg_risk)
            .await
    }
}

fn validate_order_inputs(
    token_id: &str,
    price: Decimal,
    size: Decimal,
) -> Result<(), Box<dyn std::error::Error>> {
    if token_id.trim().is_empty() || token_id.len() > 256 {
        return Err("invalid token_id".into());
    }
    if price <= dec!(0) || price > dec!(1) {
        return Err("Polymarket price must be in (0, 1]".into());
    }
    if size <= dec!(0) {
        return Err("order size must be positive".into());
    }
    Ok(())
}

fn decimal_to_base_units(value: Decimal) -> Result<String, Box<dyn std::error::Error>> {
    if value <= dec!(0) {
        return Err("amount must be positive".into());
    }

    let scaled = value * dec!(1_000_000);
    if scaled.fract() != dec!(0) {
        return Err("amount has more than 6 decimal places".into());
    }

    Ok(scaled.trunc().normalize().to_string())
}
