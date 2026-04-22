//! Finnhub REST API client — covers the top free-tier endpoints for
//! quote, fundamentals, news, sentiment, calendars, and technicals.
//!
//! API docs: <https://finnhub.io/docs/api>
//! Base URL:  `https://finnhub.io/api/v1`

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

// ═══════════════════════════════════════════════════════════════════════════════
// ─── Response Models ────────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

// ── Quote ───────────────────────────────────────────────────────────────────

/// Real-time quote for a symbol — `/quote`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinnhubQuote {
    /// Current price
    #[serde(rename = "c")]
    pub current: f64,
    /// Change
    #[serde(rename = "d")]
    pub change: Option<f64>,
    /// Percent change
    #[serde(rename = "dp")]
    pub change_percent: Option<f64>,
    /// High price of the day
    #[serde(rename = "h")]
    pub high: f64,
    /// Low price of the day
    #[serde(rename = "l")]
    pub low: f64,
    /// Open price of the day
    #[serde(rename = "o")]
    pub open: f64,
    /// Previous close price
    #[serde(rename = "pc")]
    pub prev_close: f64,
    /// Timestamp
    #[serde(rename = "t")]
    pub timestamp: i64,
}

// ── Company News ────────────────────────────────────────────────────────────

/// Company-specific news article — `/company-news`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyNews {
    pub category: Option<String>,
    pub datetime: i64,
    pub headline: String,
    pub id: i64,
    pub image: Option<String>,
    pub related: Option<String>,
    pub source: String,
    pub summary: Option<String>,
    pub url: String,
}

// ── News Sentiment ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsSentiment {
    pub buzz: Option<BuzzStats>,
    #[serde(rename = "companyNewsScore")]
    pub company_news_score: Option<f64>,
    #[serde(rename = "sectorAverageBullishPercent")]
    pub sector_avg_bullish: Option<f64>,
    #[serde(rename = "sectorAverageNewsScore")]
    pub sector_avg_news_score: Option<f64>,
    pub sentiment: Option<SentimentScores>,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuzzStats {
    #[serde(rename = "articlesInLastWeek")]
    pub articles_last_week: Option<i64>,
    pub buzz: Option<f64>,
    #[serde(rename = "weeklyAverage")]
    pub weekly_average: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentScores {
    #[serde(rename = "bearishPercent")]
    pub bearish_percent: Option<f64>,
    #[serde(rename = "bullishPercent")]
    pub bullish_percent: Option<f64>,
}

// ── Earnings Calendar ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarningsCalendarResponse {
    #[serde(rename = "earningsCalendar")]
    pub earnings_calendar: Vec<EarningsRelease>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarningsRelease {
    pub date: Option<String>,
    #[serde(rename = "epsActual")]
    pub eps_actual: Option<f64>,
    #[serde(rename = "epsEstimate")]
    pub eps_estimate: Option<f64>,
    /// "bmo" (before market open), "amc" (after market close), "dmh" (during market hours)
    pub hour: Option<String>,
    pub quarter: Option<i64>,
    #[serde(rename = "revenueActual")]
    pub revenue_actual: Option<f64>,
    #[serde(rename = "revenueEstimate")]
    pub revenue_estimate: Option<f64>,
    pub symbol: Option<String>,
    pub year: Option<i64>,
}

// ── Economic Calendar ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicCalendarResponse {
    #[serde(rename = "economicCalendar")]
    pub economic_calendar: Vec<EconomicEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicEvent {
    pub actual: Option<f64>,
    pub country: Option<String>,
    pub estimate: Option<f64>,
    pub event: Option<String>,
    pub impact: Option<String>,
    pub prev: Option<f64>,
    pub time: Option<String>,
    pub unit: Option<String>,
}

// ── Recommendation Trends ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationTrend {
    pub buy: Option<i64>,
    pub hold: Option<i64>,
    pub period: Option<String>,
    pub sell: Option<i64>,
    #[serde(rename = "strongBuy")]
    pub strong_buy: Option<i64>,
    #[serde(rename = "strongSell")]
    pub strong_sell: Option<i64>,
    pub symbol: Option<String>,
}

// ── Price Target ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceTarget {
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
    #[serde(rename = "numberAnalysts")]
    pub number_analysts: Option<i64>,
    pub symbol: Option<String>,
    #[serde(rename = "targetHigh")]
    pub target_high: Option<f64>,
    #[serde(rename = "targetLow")]
    pub target_low: Option<f64>,
    #[serde(rename = "targetMean")]
    pub target_mean: Option<f64>,
    #[serde(rename = "targetMedian")]
    pub target_median: Option<f64>,
}

// ── Company Profile ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyProfile {
    pub country: Option<String>,
    pub currency: Option<String>,
    pub exchange: Option<String>,
    #[serde(rename = "finnhubIndustry")]
    pub finnhub_industry: Option<String>,
    pub ipo: Option<String>,
    pub logo: Option<String>,
    #[serde(rename = "marketCapitalization")]
    pub market_cap: Option<f64>,
    pub name: Option<String>,
    pub phone: Option<String>,
    #[serde(rename = "shareOutstanding")]
    pub shares_outstanding: Option<f64>,
    pub ticker: Option<String>,
    pub weburl: Option<String>,
}

// ── Basic Financials ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicFinancials {
    pub metric: Option<serde_json::Value>,
    #[serde(rename = "metricType")]
    pub metric_type: Option<String>,
    pub symbol: Option<String>,
}

// ── Insider Transactions ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsiderTransactionsResponse {
    pub data: Vec<InsiderTransaction>,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsiderTransaction {
    pub change: Option<i64>,
    #[serde(rename = "filingDate")]
    pub filing_date: Option<String>,
    pub name: Option<String>,
    pub share: Option<i64>,
    pub symbol: Option<String>,
    #[serde(rename = "transactionCode")]
    pub transaction_code: Option<String>,
    #[serde(rename = "transactionDate")]
    pub transaction_date: Option<String>,
    #[serde(rename = "transactionPrice")]
    pub transaction_price: Option<f64>,
}

// ── Insider Sentiment ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsiderSentimentResponse {
    pub data: Vec<InsiderSentimentData>,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsiderSentimentData {
    pub change: Option<i64>,
    pub month: Option<i64>,
    /// Monthly share purchase ratio
    pub mspr: Option<f64>,
    pub symbol: Option<String>,
    pub year: Option<i64>,
}

// ── Technical Indicator ─────────────────────────────────────────────────────

/// Response from `/indicator` — technical indicator values (SMA, EMA, RSI, MACD, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechnicalIndicator {
    /// Close prices
    #[serde(rename = "c", default)]
    pub close: Vec<f64>,
    /// High prices
    #[serde(rename = "h", default)]
    pub high: Vec<f64>,
    /// Low prices
    #[serde(rename = "l", default)]
    pub low: Vec<f64>,
    /// Open prices
    #[serde(rename = "o", default)]
    pub open: Vec<f64>,
    /// Status
    #[serde(rename = "s")]
    pub status: Option<String>,
    /// Timestamps
    #[serde(rename = "t", default)]
    pub timestamps: Vec<i64>,
    /// Volume
    #[serde(rename = "v", default)]
    pub volume: Vec<f64>,
    /// Indicator values (dynamic — depends on indicator type)
    #[serde(flatten)]
    pub indicator_values: serde_json::Value,
}

// ── Support & Resistance ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportResistance {
    pub levels: Vec<f64>,
}

// ── Stock Candles ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockCandles {
    /// Close prices
    #[serde(rename = "c", default)]
    pub close: Vec<f64>,
    /// High prices
    #[serde(rename = "h", default)]
    pub high: Vec<f64>,
    /// Low prices
    #[serde(rename = "l", default)]
    pub low: Vec<f64>,
    /// Open prices
    #[serde(rename = "o", default)]
    pub open: Vec<f64>,
    /// Status
    #[serde(rename = "s")]
    pub status: Option<String>,
    /// Timestamps
    #[serde(rename = "t", default)]
    pub timestamps: Vec<i64>,
    /// Volume
    #[serde(rename = "v", default)]
    pub volume: Vec<f64>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// ─── Client ─────────────────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

pub struct FinnhubClient {
    client: Client,
    api_key: String,
    base_url: String,
}

impl FinnhubClient {
    /// Create from `FINNHUB_API_KEY` environment variable.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("FINNHUB_API_KEY").context("FINNHUB_API_KEY not set")?;
        Ok(Self::new(api_key))
    }

    /// Create with explicit API key.
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://finnhub.io/api/v1".to_string(),
        }
    }

    /// Internal helper — GET with automatic `token` parameter injection.
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T> {
        let owned: Vec<(String, String)> = params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        self.get_owned(path, owned).await
    }

    /// Internal helper using fully owned param pairs — used when keys are
    /// dynamically constructed (e.g., technical indicator fields).
    async fn get_owned<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        mut params: Vec<(String, String)>,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        params.insert(0, ("token".to_string(), self.api_key.clone()));

        debug!(url = %url, "Finnhub REST request");

        let resp = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await
            .context("Finnhub request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::error!(
                url = %url,
                status = %status,
                body = %body,
                "Finnhub API error — response body logged for diagnosis"
            );
            anyhow::bail!("Finnhub HTTP {} for {}: {}", status, url, body);
        }

        resp.json::<T>()
            .await
            .context("Failed to parse Finnhub response")
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Quote ──────────────────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /quote — real-time price, change, high/low/open/prev close.
    ///
    /// ```ignore
    /// let q = client.get_quote("AAPL").await?;
    /// println!("AAPL: ${} ({:+.2}%)", q.current, q.change_percent.unwrap_or(0.0));
    /// ```
    pub async fn get_quote(&self, symbol: &str) -> Result<FinnhubQuote> {
        self.get("/quote", &[("symbol", symbol.to_string())]).await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Company News ───────────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /company-news — news for a specific symbol within a date range.
    ///
    /// Free tier: 1 year of history. Dates in `YYYY-MM-DD` format.
    pub async fn get_company_news(
        &self,
        symbol: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<CompanyNews>> {
        self.get(
            "/company-news",
            &[
                ("symbol", symbol.to_string()),
                ("from", from.to_string()),
                ("to", to.to_string()),
            ],
        )
        .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── News Sentiment ─────────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /news-sentiment — aggregated bullish/bearish sentiment for a symbol.
    pub async fn get_news_sentiment(&self, symbol: &str) -> Result<NewsSentiment> {
        self.get("/news-sentiment", &[("symbol", symbol.to_string())])
            .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Earnings Calendar ──────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /calendar/earnings — upcoming and historical earnings releases.
    ///
    /// Pass `symbol` to filter for a specific company, or None for all.
    pub async fn get_earnings_calendar(
        &self,
        from: Option<&str>,
        to: Option<&str>,
        symbol: Option<&str>,
    ) -> Result<EarningsCalendarResponse> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(f) = from {
            params.push(("from", f.to_string()));
        }
        if let Some(t) = to {
            params.push(("to", t.to_string()));
        }
        if let Some(s) = symbol {
            params.push(("symbol", s.to_string()));
        }
        self.get("/calendar/earnings", &params).await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Economic Calendar ──────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /calendar/economic — macro events (CPI, NFP, FOMC, etc.)
    pub async fn get_economic_calendar(
        &self,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<EconomicCalendarResponse> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(f) = from {
            params.push(("from", f.to_string()));
        }
        if let Some(t) = to {
            params.push(("to", t.to_string()));
        }
        self.get("/calendar/economic", &params).await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Recommendations ────────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /stock/recommendation — analyst buy/hold/sell consensus history.
    pub async fn get_recommendations(&self, symbol: &str) -> Result<Vec<RecommendationTrend>> {
        self.get("/stock/recommendation", &[("symbol", symbol.to_string())])
            .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Price Target ───────────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /stock/price-target — analyst price target consensus.
    pub async fn get_price_target(&self, symbol: &str) -> Result<PriceTarget> {
        self.get("/stock/price-target", &[("symbol", symbol.to_string())])
            .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Company Profile ────────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /stock/profile2 — company info (name, industry, market cap, etc.)
    pub async fn get_company_profile(&self, symbol: &str) -> Result<CompanyProfile> {
        self.get("/stock/profile2", &[("symbol", symbol.to_string())])
            .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Basic Financials ───────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /stock/metric — key financial ratios (P/E, P/B, EPS, dividend yield, etc.)
    ///
    /// `metric_type` is typically `"all"`.
    pub async fn get_basic_financials(
        &self,
        symbol: &str,
        metric_type: &str,
    ) -> Result<BasicFinancials> {
        self.get(
            "/stock/metric",
            &[
                ("symbol", symbol.to_string()),
                ("metric", metric_type.to_string()),
            ],
        )
        .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Peers ──────────────────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /stock/peers — list of similar companies by sub-industry.
    pub async fn get_peers(&self, symbol: &str) -> Result<Vec<String>> {
        self.get("/stock/peers", &[("symbol", symbol.to_string())])
            .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Insider Transactions ───────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /stock/insider-transactions — insider buying/selling activity.
    pub async fn get_insider_transactions(
        &self,
        symbol: &str,
    ) -> Result<InsiderTransactionsResponse> {
        self.get(
            "/stock/insider-transactions",
            &[("symbol", symbol.to_string())],
        )
        .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Insider Sentiment ──────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /stock/insider-sentiment — monthly insider sentiment (MSPR).
    pub async fn get_insider_sentiment(
        &self,
        symbol: &str,
        from: &str,
        to: &str,
    ) -> Result<InsiderSentimentResponse> {
        self.get(
            "/stock/insider-sentiment",
            &[
                ("symbol", symbol.to_string()),
                ("from", from.to_string()),
                ("to", to.to_string()),
            ],
        )
        .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Technical Indicators ───────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /indicator — compute technical indicators (SMA, EMA, RSI, MACD, Bollinger, etc.)
    ///
    /// # Parameters
    /// - `symbol`: Stock symbol (e.g., "AAPL")
    /// - `resolution`: Candle resolution: 1, 5, 15, 30, 60, D, W, M
    /// - `from`: UNIX timestamp start
    /// - `to`: UNIX timestamp end
    /// - `indicator`: Indicator name (e.g., "sma", "ema", "rsi", "macd", "bbands")
    /// - `indicator_fields`: Indicator-specific params as JSON object
    ///   - SMA/EMA: `{"timeperiod": 14}`
    ///   - RSI: `{"timeperiod": 14}`
    ///   - MACD: `{"fastperiod": 12, "slowperiod": 26, "signalperiod": 9}`
    ///   - Bollinger: `{"timeperiod": 20, "nbdevup": 2, "nbdevdn": 2}`
    pub async fn get_technical_indicator(
        &self,
        symbol: &str,
        resolution: &str,
        from: i64,
        to: i64,
        indicator: &str,
        indicator_fields: Option<&serde_json::Value>,
    ) -> Result<TechnicalIndicator> {
        let mut params: Vec<(String, String)> = vec![
            ("symbol".into(), symbol.to_string()),
            ("resolution".into(), resolution.to_string()),
            ("from".into(), from.to_string()),
            ("to".into(), to.to_string()),
            ("indicator".into(), indicator.to_string()),
        ];

        if let Some(fields) = indicator_fields {
            if let Some(obj) = fields.as_object() {
                for (k, v) in obj {
                    let val = match v {
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    params.push((k.clone(), val));
                }
            }
        }

        self.get_owned("/indicator", params).await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Support & Resistance ───────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /scan/support-resistance — key support and resistance levels.
    pub async fn get_support_resistance(
        &self,
        symbol: &str,
        resolution: &str,
    ) -> Result<SupportResistance> {
        self.get(
            "/scan/support-resistance",
            &[
                ("symbol", symbol.to_string()),
                ("resolution", resolution.to_string()),
            ],
        )
        .await
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ─── Stock Candles ──────────────────────────────────────────────────
    // ═════════════════════════════════════════════════════════════════════════

    /// GET /stock/candle — historical OHLCV data.
    ///
    /// `resolution`: 1, 5, 15, 30, 60, D, W, M
    /// `from`/`to`: UNIX timestamps (seconds, not millis)
    ///
    /// **Note**: Finnhub returns HTTP 200 with `s: "no_data"` when the symbol
    /// isn't found or no data exists in the date range. This method detects
    /// that case and logs a warning.
    pub async fn get_stock_candles(
        &self,
        symbol: &str,
        resolution: &str,
        from: i64,
        to: i64,
    ) -> Result<StockCandles> {
        let candles: StockCandles = self
            .get(
                "/stock/candle",
                &[
                    ("symbol", symbol.to_string()),
                    ("resolution", resolution.to_string()),
                    ("from", from.to_string()),
                    ("to", to.to_string()),
                ],
            )
            .await?;

        // Finnhub returns s:"no_data" with HTTP 200 when symbol or date range is invalid
        if candles.status.as_deref() == Some("no_data") {
            tracing::warn!(
                symbol = %symbol,
                from = from,
                to = to,
                resolution = %resolution,
                "Finnhub candle response: no_data (check symbol or date range)"
            );
        }

        Ok(candles)
    }
}
