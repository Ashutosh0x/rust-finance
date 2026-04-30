use serde::{Deserialize, Serialize};
use std::fmt;
use tracing::warn;

#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
    // Required keys
    pub finnhub_api_key: String,
    pub alpaca_api_key: String,
    pub alpaca_secret_key: String,

    // Optional keys (graceful degradation)
    pub anthropic_api_key: Option<String>,
    pub sol_private_key: Option<String>,

    // Feature flags with defaults
    #[serde(default = "default_use_mock")]
    pub use_mock: String,

    // Alpaca environment
    #[serde(default = "default_alpaca_base_url")]
    pub alpaca_base_url: String,

    // Logging
    #[serde(default = "default_log_level")]
    pub rust_log: String,
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("finnhub_api_key", &redact(&self.finnhub_api_key))
            .field("alpaca_api_key", &redact(&self.alpaca_api_key))
            .field("alpaca_secret_key", &"<redacted>")
            .field(
                "anthropic_api_key",
                &self.anthropic_api_key.as_ref().map(|k| redact(k)),
            )
            .field(
                "sol_private_key",
                &self.sol_private_key.as_ref().map(|_| "<redacted>"),
            )
            .field("use_mock", &self.use_mock)
            .field("alpaca_base_url", &self.alpaca_base_url)
            .field("rust_log", &self.rust_log)
            .finish()
    }
}

fn redact(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= 4 {
        "<redacted>".to_string()
    } else {
        let prefix: String = trimmed.chars().take(4).collect();
        format!("{}...<redacted>", prefix)
    }
}

fn default_use_mock() -> String {
    "0".to_string()
}
fn default_alpaca_base_url() -> String {
    "https://paper-api.alpaca.markets".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}

impl AppConfig {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        dotenvy::dotenv().ok(); // load .env if present, ignore if missing

        let mut config: Self = envy::from_env()?;

        if config.validate().is_err() && config.use_mock != "1" {
            warn!("Missing critical API keys. Automatically tumbling back to USE_MOCK=1 synthetic engine mode.");
            config.use_mock = "1".to_string();
        }

        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.use_mock != "1" {
            if self.finnhub_api_key.trim().is_empty() {
                return Err("FINNHUB_API_KEY cannot be empty when USE_MOCK=0".into());
            }
            if self.alpaca_api_key.trim().is_empty() {
                return Err("ALPACA_API_KEY cannot be empty when USE_MOCK=0".into());
            }
            if self.alpaca_secret_key.trim().is_empty() {
                return Err("ALPACA_SECRET_KEY cannot be empty when USE_MOCK=0".into());
            }
        }

        if !self.alpaca_base_url.starts_with("http") {
            return Err(format!(
                "Invalid Alpaca URL format: {}",
                self.alpaca_base_url
            ));
        }

        Ok(())
    }

    pub fn ai_enabled(&self) -> bool {
        self.anthropic_api_key
            .as_ref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false)
    }
}
