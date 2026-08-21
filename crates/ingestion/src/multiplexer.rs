//! Multiplexer: merges multiple MarketDataSource streams into one unified
//! stream. It preserves per-source order, but does not perform global
//! timestamp/sequence sorting across venues.
//!
//! This is the key architectural component that lets the daemon consume
//! Finnhub + Alpaca + Binance + Polymarket through a single Stream.

use crate::source::{IngestionError, MarketDataSource, MarketStream, Subscription};
use futures::stream::{self, SelectAll, StreamExt};
use tracing::{error, info, warn};

/// One registered source and the symbols it is allowed to see.
struct Registered {
    source: Box<dyn MarketDataSource>,
    /// `None` means "every symbol in the subscription", which is the right
    /// default for a source that genuinely serves whatever it is asked for.
    only_symbols: Option<Vec<String>>,
}

/// A collection of data sources that get merged into one stream.
pub struct Multiplexer {
    sources: Vec<Registered>,
}

impl Multiplexer {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    /// Add a data source that receives every symbol in the subscription.
    pub fn add_source(mut self, source: impl MarketDataSource + 'static) -> Self {
        self.sources.push(Registered {
            source: Box::new(source),
            only_symbols: None,
        });
        self
    }

    /// Add a data source that only ever sees `symbols`.
    ///
    /// Without this every source received every symbol, so a crypto venue was
    /// asked for `aapl@trade` and an equities venue for `BTCUSDT`. Those
    /// subscriptions cannot be served, and the venue's response is to ignore
    /// them silently — which looks identical to a working connection that
    /// happens to be quiet.
    pub fn add_source_for(
        mut self,
        source: impl MarketDataSource + 'static,
        symbols: Vec<String>,
    ) -> Self {
        self.sources.push(Registered {
            source: Box::new(source),
            only_symbols: Some(symbols),
        });
        self
    }

    /// Connect all sources and merge their streams.
    ///
    /// Each source subscribes to the appropriate subset of the subscription
    /// based on what it supports. Failed connections are logged and skipped,
    /// but startup fails closed if no source connects.
    pub async fn connect(self, subscription: &Subscription) -> MarketStream {
        let mut select_all: SelectAll<MarketStream> = SelectAll::new();
        let mut connected_count = 0;

        for Registered {
            source,
            only_symbols,
        } in &self.sources
        {
            // Filter subscription to only include data types this source supports
            let supported = source.supported_data_types();
            let filtered_types: Vec<_> = subscription
                .data_types
                .iter()
                .filter(|dt| supported.contains(dt))
                .cloned()
                .collect();

            if filtered_types.is_empty() {
                info!(source = source.name(), "Skipped — no matching data types");
                continue;
            }

            // Symbols this source can actually serve. Matching is
            // case-insensitive because venues disagree about case and the
            // configuration should not have to.
            let symbols: Vec<String> = match only_symbols {
                Some(allowed) => subscription
                    .symbols
                    .iter()
                    .filter(|s| allowed.iter().any(|a| a.eq_ignore_ascii_case(s)))
                    .cloned()
                    .collect(),
                None => subscription.symbols.clone(),
            };

            if symbols.is_empty() {
                info!(source = source.name(), "Skipped - no matching symbols");
                continue;
            }

            let filtered_sub = Subscription {
                symbols,
                data_types: filtered_types,
            };

            match source.connect(&filtered_sub).await {
                Ok(stream) => {
                    info!(source = source.name(), "Connected successfully");
                    select_all.push(stream);
                    connected_count += 1;
                }
                Err(e) => {
                    warn!(
                        source = source.name(),
                        error = %e,
                        "Failed to connect — continuing without this source"
                    );
                }
            }
        }

        info!(
            total_sources = self.sources.len(),
            connected = connected_count,
            "Multiplexer ready"
        );

        if connected_count == 0 {
            let err = IngestionError::ConnectionFailed(
                "No market data sources connected; refusing silent empty stream".into(),
            );
            return Box::pin(stream::once(async move { Err(err) }));
        }

        Box::pin(select_all)
    }

    /// Health check all sources.
    pub async fn health_check(&self) -> Vec<(&str, bool)> {
        let mut results = Vec::new();
        for Registered { source, .. } in &self.sources {
            let healthy = source.is_healthy().await;
            results.push((source.name(), healthy));
        }
        results
    }
}

impl Default for Multiplexer {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Reconnecting wrapper ────────────────────────────────────────────────────

/// Wraps a MarketDataSource with automatic reconnection on stream errors.
/// Implements exponential backoff with configurable max retries.
pub struct ReconnectingSource<S: MarketDataSource + Clone + 'static> {
    inner: S,
    max_retries: usize,
    base_delay_ms: u64,
}

impl<S: MarketDataSource + Clone + 'static> ReconnectingSource<S> {
    pub fn new(source: S) -> Self {
        Self {
            inner: source,
            max_retries: 10,
            base_delay_ms: 1000,
        }
    }

    pub fn with_max_retries(mut self, n: usize) -> Self {
        self.max_retries = n;
        self
    }

    /// Connect with automatic reconnection.
    /// Returns a stream that internally reconnects on errors.
    pub async fn connect_resilient(&self, subscription: Subscription) -> MarketStream {
        let inner = self.inner.clone();
        let max_retries = self.max_retries;
        let base_delay_ms = self.base_delay_ms;
        let sub = subscription;

        let stream = async_stream::stream! {
            let mut retry_count = 0;

            loop {
                match inner.connect(&sub).await {
                    Ok(mut source_stream) => {
                        retry_count = 0; // Reset on successful connect
                        info!(source = inner.name(), "Connected");

                        while let Some(result) = source_stream.next().await {
                            match &result {
                                Ok(_) => {
                                    yield result;
                                }
                                Err(IngestionError::StreamClosed) => {
                                    warn!(source = inner.name(), "Stream closed, reconnecting");
                                    break;
                                }
                                Err(e) => {
                                    error!(
                                        source = inner.name(),
                                        error = %e,
                                        "Stream error"
                                    );
                                    yield result;
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            source = inner.name(),
                            error = %e,
                            retry = retry_count,
                            "Connection failed"
                        );
                    }
                }

                retry_count += 1;
                if retry_count > max_retries {
                    error!(
                        source = inner.name(),
                        "Max retries ({max_retries}) exceeded, giving up"
                    );
                    break;
                }

                // Exponential backoff: 1s, 2s, 4s, 8s... capped at 60s
                let delay = (base_delay_ms * (1 << retry_count.min(6))).min(60_000);
                warn!(
                    source = inner.name(),
                    delay_ms = delay,
                    retry = retry_count,
                    "Reconnecting after backoff"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        };

        Box::pin(stream)
    }
}
