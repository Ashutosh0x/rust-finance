// crates/ingestion/src/news.rs
// Unified news feed fetcher — polls Finnhub + Alpaca news APIs
// Emits BotEvent::Feed(headline) into the event bus

use chrono::Utc;
use common::events::BotEvent;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Configuration for the news feed
#[derive(Debug, Clone)]
pub struct NewsFeedConfig {
    /// Finnhub API key
    pub finnhub_key: Option<String>,
    /// Alpaca API key + secret
    pub alpaca_key: Option<String>,
    pub alpaca_secret: Option<String>,
    /// Polling interval (default: 60 seconds)
    pub poll_interval: Duration,
    /// Maximum headlines per poll
    pub max_headlines: usize,
}

impl Default for NewsFeedConfig {
    fn default() -> Self {
        Self {
            finnhub_key: None,
            alpaca_key: None,
            alpaca_secret: None,
            poll_interval: Duration::from_secs(60),
            max_headlines: 20,
        }
    }
}

/// A single news headline
#[derive(Debug, Clone, Deserialize)]
pub struct NewsHeadline {
    pub source: String,
    pub headline: String,
    pub summary: Option<String>,
    pub url: Option<String>,
    pub timestamp: i64,
}

// ─── Finnhub Response Model ─────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FinnhubNewsItem {
    category: Option<String>,
    datetime: i64,
    headline: String,
    source: String,
    url: String,
    summary: Option<String>,
}

// ─── Alpaca Response Model ──────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AlpacaNewsItem {
    headline: String,
    source: String,
    url: String,
    created_at: String,
    summary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlpacaNewsResponse {
    news: Vec<AlpacaNewsItem>,
}

/// Fetches news from Finnhub general news API
async fn fetch_finnhub_news(
    client: &reqwest::Client,
    api_key: &str,
    max: usize,
) -> Vec<NewsHeadline> {
    let url = format!(
        "https://finnhub.io/api/v1/news?category=general&token={}",
        api_key
    );

    match client.get(&url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                warn!(status = %resp.status(), "Finnhub news API error");
                return Vec::new();
            }
            match resp.json::<Vec<FinnhubNewsItem>>().await {
                Ok(items) => items
                    .into_iter()
                    .take(max)
                    .map(|item| NewsHeadline {
                        source: item.source,
                        headline: item.headline,
                        summary: item.summary,
                        url: Some(item.url),
                        timestamp: item.datetime,
                    })
                    .collect(),
                Err(e) => {
                    error!(error = %e, "Failed to parse Finnhub news");
                    Vec::new()
                }
            }
        }
        Err(e) => {
            error!(error = %e, "Finnhub news request failed");
            Vec::new()
        }
    }
}

/// Fetches news from Alpaca news API
async fn fetch_alpaca_news(
    client: &reqwest::Client,
    api_key: &str,
    api_secret: &str,
    max: usize,
) -> Vec<NewsHeadline> {
    let url = format!(
        "https://data.alpaca.markets/v1beta1/news?limit={}",
        max
    );

    match client
        .get(&url)
        .header("APCA-API-KEY-ID", api_key)
        .header("APCA-API-SECRET-KEY", api_secret)
        .send()
        .await
    {
        Ok(resp) => {
            if !resp.status().is_success() {
                warn!(status = %resp.status(), "Alpaca news API error");
                return Vec::new();
            }
            match resp.json::<AlpacaNewsResponse>().await {
                Ok(data) => data
                    .news
                    .into_iter()
                    .take(max)
                    .map(|item| NewsHeadline {
                        source: item.source,
                        headline: item.headline,
                        summary: item.summary,
                        url: Some(item.url),
                        timestamp: Utc::now().timestamp(),
                    })
                    .collect(),
                Err(e) => {
                    error!(error = %e, "Failed to parse Alpaca news");
                    Vec::new()
                }
            }
        }
        Err(e) => {
            error!(error = %e, "Alpaca news request failed");
            Vec::new()
        }
    }
}

/// News feed runner. Polls Finnhub + Alpaca on an interval
/// and broadcasts headlines as BotEvent::Feed.
pub async fn run_news_feed(
    config: NewsFeedConfig,
    event_tx: broadcast::Sender<BotEvent>,
) {
    info!(
        interval = ?config.poll_interval,
        finnhub = config.finnhub_key.is_some(),
        alpaca = config.alpaca_key.is_some(),
        "News feed started"
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let mut interval = tokio::time::interval(config.poll_interval);

    loop {
        interval.tick().await;

        let mut headlines: Vec<NewsHeadline> = Vec::new();

        // Fetch from Finnhub
        if let Some(ref key) = config.finnhub_key {
            let fh = fetch_finnhub_news(&client, key, config.max_headlines / 2).await;
            debug!(count = fh.len(), "Finnhub headlines fetched");
            headlines.extend(fh);
        }

        // Fetch from Alpaca
        if let (Some(ref key), Some(ref secret)) = (&config.alpaca_key, &config.alpaca_secret) {
            let alp = fetch_alpaca_news(&client, key, secret, config.max_headlines / 2).await;
            debug!(count = alp.len(), "Alpaca headlines fetched");
            headlines.extend(alp);
        }

        // Sort by timestamp descending (newest first)
        headlines.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        // Broadcast each headline as a BotEvent::Feed
        for hl in headlines.iter().take(config.max_headlines) {
            let feed_text = format!("[{}] {}", hl.source, hl.headline);
            let _ = event_tx.send(BotEvent::Feed(feed_text));
        }

        if !headlines.is_empty() {
            info!(count = headlines.len(), "News headlines broadcast");
        }
    }
}
