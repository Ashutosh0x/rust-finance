// crates/ingestion/src/news.rs
// Unified news feed fetcher — polls Finnhub + Alpaca news APIs
// Emits BotEvent::Feed(headline) into the event bus
//
// Enhancements over v1:
//   • Per-symbol Finnhub company news via /company-news
//   • Alpaca news pagination via page_token
//   • include_content + sort params for Alpaca
//   • Dedup by headline to prevent duplicates across sources

use chrono::Utc;
use common::events::BotEvent;
use serde::Deserialize;
use std::collections::HashSet;
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
    /// Symbols to fetch per-company news for (Finnhub company-news)
    pub watch_symbols: Vec<String>,
    /// Include full article content from Alpaca (default: false)
    pub include_content: bool,
    /// Number of Alpaca pages to fetch (for pagination, default: 1)
    pub alpaca_pages: usize,
}

impl Default for NewsFeedConfig {
    fn default() -> Self {
        Self {
            finnhub_key: None,
            alpaca_key: None,
            alpaca_secret: None,
            poll_interval: Duration::from_secs(60),
            max_headlines: 20,
            watch_symbols: Vec::new(),
            include_content: false,
            alpaca_pages: 1,
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
    /// Full article content (only if include_content is set)
    pub content: Option<String>,
    /// Related symbol(s)
    pub symbols: Vec<String>,
}

// ─── Finnhub Response Models ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FinnhubNewsItem {
    category: Option<String>,
    datetime: i64,
    headline: String,
    source: String,
    url: String,
    summary: Option<String>,
    related: Option<String>,
}

// ─── Alpaca Response Model ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AlpacaNewsItem {
    headline: String,
    source: String,
    url: String,
    created_at: String,
    summary: Option<String>,
    content: Option<String>,
    #[serde(default)]
    symbols: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AlpacaNewsResponse {
    news: Vec<AlpacaNewsItem>,
    next_page_token: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// ─── Finnhub Fetchers ───────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

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
                        content: None,
                        symbols: item
                            .related
                            .map(|r| r.split(',').map(|s| s.trim().to_string()).collect())
                            .unwrap_or_default(),
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

/// Fetches company-specific news from Finnhub for a single symbol.
///
/// Uses `/company-news` endpoint with a 7-day lookback window.
async fn fetch_finnhub_company_news(
    client: &reqwest::Client,
    api_key: &str,
    symbol: &str,
    max: usize,
) -> Vec<NewsHeadline> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let week_ago = (Utc::now() - chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();

    let url = format!(
        "https://finnhub.io/api/v1/company-news?symbol={}&from={}&to={}&token={}",
        symbol, week_ago, today, api_key
    );

    match client.get(&url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                warn!(status = %resp.status(), symbol = symbol, "Finnhub company news error");
                return Vec::new();
            }
            match resp.json::<Vec<FinnhubNewsItem>>().await {
                Ok(items) => items
                    .into_iter()
                    .take(max)
                    .map(|item| NewsHeadline {
                        source: format!("Finnhub/{}", item.source),
                        headline: item.headline,
                        summary: item.summary,
                        url: Some(item.url),
                        timestamp: item.datetime,
                        content: None,
                        symbols: vec![symbol.to_string()],
                    })
                    .collect(),
                Err(e) => {
                    error!(error = %e, symbol = symbol, "Failed to parse Finnhub company news");
                    Vec::new()
                }
            }
        }
        Err(e) => {
            error!(error = %e, symbol = symbol, "Finnhub company news request failed");
            Vec::new()
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ─── Alpaca Fetcher ─────────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

/// Fetches news from Alpaca news API with pagination support.
async fn fetch_alpaca_news(
    client: &reqwest::Client,
    api_key: &str,
    api_secret: &str,
    max: usize,
    include_content: bool,
    symbols: Option<&[String]>,
    max_pages: usize,
) -> Vec<NewsHeadline> {
    let mut all_headlines: Vec<NewsHeadline> = Vec::new();
    let mut page_token: Option<String> = None;

    for page in 0..max_pages {
        let mut url = format!(
            "https://data.alpaca.markets/v1beta1/news?limit={}&sort=desc",
            max
        );

        if include_content {
            url.push_str("&include_content=true");
        }

        if let Some(syms) = symbols {
            if !syms.is_empty() {
                url.push_str(&format!("&symbols={}", syms.join(",")));
            }
        }

        if let Some(ref token) = page_token {
            url.push_str(&format!("&page_token={}", token));
        }

        match client
            .get(&url)
            .header("APCA-API-KEY-ID", api_key)
            .header("APCA-API-SECRET-KEY", api_secret)
            .send()
            .await
        {
            Ok(resp) => {
                if !resp.status().is_success() {
                    warn!(status = %resp.status(), page = page, "Alpaca news API error");
                    break;
                }
                match resp.json::<AlpacaNewsResponse>().await {
                    Ok(data) => {
                        let count = data.news.len();
                        for item in data.news {
                            all_headlines.push(NewsHeadline {
                                source: item.source,
                                headline: item.headline,
                                summary: item.summary,
                                url: Some(item.url),
                                timestamp: Utc::now().timestamp(),
                                content: item.content,
                                symbols: item.symbols,
                            });
                        }

                        debug!(page = page, count = count, "Alpaca news page fetched");

                        // If there's a next page token and we haven't hit the limit, continue
                        if let Some(next_token) = data.next_page_token {
                            page_token = Some(next_token);
                        } else {
                            break; // No more pages
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to parse Alpaca news");
                        break;
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "Alpaca news request failed");
                break;
            }
        }
    }

    all_headlines
}

// ═══════════════════════════════════════════════════════════════════════════════
// ─── News Feed Runner ───────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

/// News feed runner. Polls Finnhub + Alpaca on an interval
/// and broadcasts headlines as BotEvent::Feed.
///
/// Deduplicates headlines by title to prevent duplicates across sources.
pub async fn run_news_feed(config: NewsFeedConfig, event_tx: broadcast::Sender<BotEvent>) {
    info!(
        interval = ?config.poll_interval,
        finnhub = config.finnhub_key.is_some(),
        alpaca = config.alpaca_key.is_some(),
        symbols = config.watch_symbols.len(),
        "News feed started"
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let mut interval = tokio::time::interval(config.poll_interval);
    let mut seen_headlines: HashSet<String> = HashSet::new();

    loop {
        interval.tick().await;

        let mut headlines: Vec<NewsHeadline> = Vec::new();

        // ── Fetch general Finnhub news ──────────────────────────────────
        if let Some(ref key) = config.finnhub_key {
            let fh = fetch_finnhub_news(&client, key, config.max_headlines / 2).await;
            debug!(count = fh.len(), "Finnhub headlines fetched");
            headlines.extend(fh);

            // ── Fetch per-symbol Finnhub company news ──────────────────
            let per_symbol_max = if config.watch_symbols.is_empty() {
                0
            } else {
                (config.max_headlines / 4).max(3) / config.watch_symbols.len().max(1)
            };

            for symbol in &config.watch_symbols {
                let company =
                    fetch_finnhub_company_news(&client, key, symbol, per_symbol_max.max(2)).await;
                debug!(symbol = %symbol, count = company.len(), "Finnhub company news fetched");
                headlines.extend(company);
            }
        }

        // ── Fetch Alpaca news (with pagination + optional symbols) ──────
        if let (Some(ref key), Some(ref secret)) = (&config.alpaca_key, &config.alpaca_secret) {
            let symbols_ref: Option<&[String]> = if config.watch_symbols.is_empty() {
                None
            } else {
                Some(&config.watch_symbols)
            };

            let alp = fetch_alpaca_news(
                &client,
                key,
                secret,
                config.max_headlines / 2,
                config.include_content,
                symbols_ref,
                config.alpaca_pages,
            )
            .await;
            debug!(count = alp.len(), "Alpaca headlines fetched");
            headlines.extend(alp);
        }

        // ── Sort by timestamp descending (newest first) ────────────────
        headlines.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        // ── Dedup + broadcast ──────────────────────────────────────────
        let mut broadcast_count = 0;
        for hl in headlines.iter().take(config.max_headlines) {
            // Skip if we've seen this headline before
            if seen_headlines.contains(&hl.headline) {
                continue;
            }
            seen_headlines.insert(hl.headline.clone());

            let symbols_str = if hl.symbols.is_empty() {
                String::new()
            } else {
                format!(" [{}]", hl.symbols.join(","))
            };

            let feed_text = format!("[{}]{} {}", hl.source, symbols_str, hl.headline);
            let _ = event_tx.send(BotEvent::Feed(feed_text));
            broadcast_count += 1;
        }

        if broadcast_count > 0 {
            info!(count = broadcast_count, "News headlines broadcast");
        }

        // ── Prune seen set if it grows too large ───────────────────────
        if seen_headlines.len() > 5000 {
            seen_headlines.clear();
            debug!("Pruned seen-headlines cache");
        }
    }
}
