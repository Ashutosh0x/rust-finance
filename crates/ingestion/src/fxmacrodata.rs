//! FXMacroData REST client for macro, FX, commodities, COT, curves, and
//! release-calendar context.
//!
//! API docs: <https://fxmacrodata.com/documentation>
//! Base URL: `https://fxmacrodata.com/api/v1`

use anyhow::{Context, Result};
use reqwest::header::{HeaderValue, InvalidHeaderValue};
use reqwest::{Client, Url};
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use tracing::debug;

const DEFAULT_BASE_URL: &str = "https://fxmacrodata.com/api/v1/";

/// Lightweight FXMacroData client.
///
/// Methods return `serde_json::Value` so strategies can use the full upstream
/// response shape without this ingestion crate needing endpoint-specific
/// schema structs for every macro data family.
#[derive(Clone)]
pub struct FxMacroDataClient {
    client: Client,
    api_key: String,
    base_url: Url,
}

impl fmt::Debug for FxMacroDataClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FxMacroDataClient")
            .field("client", &self.client)
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

#[derive(Debug, Serialize)]
struct GraphQlRequest<'a> {
    query: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    variables: Option<Value>,
}

impl FxMacroDataClient {
    /// Create from `FXMACRODATA_API_KEY` or `FXMD_API_KEY`.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("FXMACRODATA_API_KEY")
            .or_else(|_| std::env::var("FXMD_API_KEY"))
            .context("FXMACRODATA_API_KEY or FXMD_API_KEY not set")?;
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    /// Create with an explicit API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, DEFAULT_BASE_URL).expect("default FXMacroData URL is valid")
    }

    /// Create with an explicit API key and base URL.
    pub fn with_base_url(api_key: impl Into<String>, base_url: &str) -> Result<Self> {
        Ok(Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: Url::parse(base_url).context("invalid FXMacroData base URL")?,
        })
    }

    /// Generic GET helper for endpoints not yet wrapped by a named method.
    pub async fn request(&self, path: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(path, params).await
    }

    /// GET `/data_catalogue/{currency}`.
    pub async fn data_catalogue(&self, currency: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("data_catalogue/{}", currency), params)
            .await
    }

    /// GET `/v1/announcements/{currency}/{indicator}`.
    pub async fn announcements(
        &self,
        currency: &str,
        indicator: &str,
        params: &[(&str, String)],
    ) -> Result<Value> {
        self.get_json(&format!("announcements/{}/{}", currency, indicator), params)
            .await
    }

    /// GET `/announcements/{currency}/latest`.
    pub async fn latest_announcements(
        &self,
        currency: &str,
        params: &[(&str, String)],
    ) -> Result<Value> {
        self.get_json(&format!("announcements/{}/latest", currency), params)
            .await
    }

    /// GET `/announcements/changes`.
    pub async fn announcement_changes(&self, params: &[(&str, String)]) -> Result<Value> {
        self.get_json("announcements/changes", params).await
    }

    /// GET `/calendar/{currency}`.
    pub async fn calendar(&self, currency: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("calendar/{}", currency), params)
            .await
    }

    /// GET `/predictions/{currency}/{indicator}`.
    pub async fn predictions(
        &self,
        currency: &str,
        indicator: &str,
        params: &[(&str, String)],
    ) -> Result<Value> {
        self.get_json(&format!("predictions/{}/{}", currency, indicator), params)
            .await
    }

    /// GET `/forex/{base}/{quote}`.
    pub async fn forex(&self, base: &str, quote: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("forex/{}/{}", base, quote), params)
            .await
    }

    /// GET `/cot/{currency}`.
    pub async fn cot(&self, currency: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("cot/{}", currency), params).await
    }

    /// GET `/commodities/{indicator}`.
    pub async fn commodity(&self, indicator: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("commodities/{}", indicator), params)
            .await
    }

    /// GET `/commodities/latest`.
    pub async fn commodities_latest(&self, params: &[(&str, String)]) -> Result<Value> {
        self.get_json("commodities/latest", params).await
    }

    /// GET `/curves/{currency}`.
    pub async fn curves(&self, currency: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("curves/{}", currency), params).await
    }

    /// GET `/curve_proxies/{currency}`.
    pub async fn curve_proxies(&self, currency: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("curve_proxies/{}", currency), params)
            .await
    }

    /// GET `/forward_curves/{currency}`.
    pub async fn forward_curves(&self, currency: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("forward_curves/{}", currency), params)
            .await
    }

    /// GET `/rate_differentials/{base}/{quote}`.
    pub async fn rate_differentials(
        &self,
        base: &str,
        quote: &str,
        params: &[(&str, String)],
    ) -> Result<Value> {
        self.get_json(&format!("rate_differentials/{}/{}", base, quote), params)
            .await
    }

    /// GET `/forward_differentials/{base}/{quote}`.
    pub async fn forward_differentials(
        &self,
        base: &str,
        quote: &str,
        params: &[(&str, String)],
    ) -> Result<Value> {
        self.get_json(&format!("forward_differentials/{}/{}", base, quote), params)
            .await
    }

    /// GET `/market_sessions`.
    pub async fn market_sessions(&self, params: &[(&str, String)]) -> Result<Value> {
        self.get_json("market_sessions", params).await
    }

    /// GET `/risk_sentiment`.
    pub async fn risk_sentiment(&self, params: &[(&str, String)]) -> Result<Value> {
        self.get_json("risk_sentiment", params).await
    }

    /// GET `/news/{currency}`.
    pub async fn news(&self, currency: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("news/{}", currency), params).await
    }

    /// GET `/press-releases/{currency}`.
    pub async fn press_releases(&self, currency: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_json(&format!("press-releases/{}", currency), params)
            .await
    }

    /// POST `/graphql`.
    pub async fn graphql(&self, query: &str, variables: Option<Value>) -> Result<Value> {
        let url = self.build_url("graphql", &[])?;
        let body = GraphQlRequest { query, variables };
        let response = self
            .with_api_key(self.client.post(url.clone()))?
            .json(&body)
            .send()
            .await
            .context("FXMacroData GraphQL request failed")?;

        Self::parse_response(response, url.as_str()).await
    }

    async fn get_json(&self, path: &str, params: &[(&str, String)]) -> Result<Value> {
        let url = self.build_url(path, params)?;
        debug!(url = %url, "FXMacroData REST request");

        let response = self
            .with_api_key(self.client.get(url.clone()))?
            .send()
            .await
            .context("FXMacroData request failed")?;

        Self::parse_response(response, url.as_str()).await
    }

    fn build_url(&self, path: &str, params: &[(&str, String)]) -> Result<Url> {
        let relative_path = path.trim_start_matches('/');
        let mut url = self
            .base_url
            .join(relative_path)
            .context("failed to build FXMacroData endpoint URL")?;

        {
            let mut query = url.query_pairs_mut();
            for (key, value) in params {
                query.append_pair(key, value);
            }
        }

        Ok(url)
    }

    fn with_api_key(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, InvalidHeaderValue> {
        let value = HeaderValue::from_str(&self.api_key)?;
        Ok(request.header("X-API-Key", value))
    }

    async fn parse_response(response: reqwest::Response, url: &str) -> Result<Value> {
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("FXMacroData HTTP {} for {}: {}", status, url, body);
        }

        response
            .json::<Value>()
            .await
            .context("failed to parse FXMacroData response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_forex_url_without_api_key_query() {
        let client = FxMacroDataClient::with_base_url("test-key", "https://example.com/api/v1/")
            .expect("valid URL");
        let url = client
            .build_url("forex/eur/usd", &[("limit", "1".to_string())])
            .expect("URL should build");

        assert_eq!(
            url.as_str(),
            "https://example.com/api/v1/forex/eur/usd?limit=1"
        );
    }

    #[test]
    fn sends_api_key_in_header() {
        let client = FxMacroDataClient::with_base_url("test-key", "https://example.com/api/v1/")
            .expect("valid URL");
        let url = client
            .build_url("forex/eur/usd", &[("limit", "1".to_string())])
            .expect("URL should build");
        let request = client
            .with_api_key(client.client.get(url))
            .expect("header should be valid")
            .build()
            .expect("request should build");

        assert_eq!(
            request
                .headers()
                .get("X-API-Key")
                .and_then(|value| value.to_str().ok()),
            Some("test-key")
        );
    }

    #[test]
    fn debug_redacts_api_key() {
        let client = FxMacroDataClient::with_base_url("test-key", "https://example.com/api/v1/")
            .expect("valid URL");
        let rendered = format!("{client:?}");

        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(!rendered.contains("test-key"), "{rendered}");
    }

    #[test]
    fn preserves_base_path_for_full_endpoint_surface() {
        let client = FxMacroDataClient::with_base_url("test-key", "https://example.com/api/v1/")
            .expect("valid URL");

        for path in [
            "data_catalogue/usd",
            "announcements/usd/non_farm_payrolls",
            "announcements/usd/latest",
            "announcements/changes",
            "calendar/usd",
            "predictions/usd/non_farm_payrolls",
            "cot/usd",
            "commodities/brent",
            "commodities/latest",
            "curves/usd",
            "curve_proxies/usd",
            "forward_curves/usd",
            "rate_differentials/eur/usd",
            "forward_differentials/eur/usd",
            "market_sessions",
            "risk_sentiment",
            "news/usd",
            "press-releases/usd",
            "graphql",
        ] {
            let url = client.build_url(path, &[]).expect("URL should build");
            assert!(
                url.as_str().starts_with("https://example.com/api/v1/"),
                "{url}"
            );
            assert!(!url.as_str().contains("api_key"), "{url}");
        }
    }
}
