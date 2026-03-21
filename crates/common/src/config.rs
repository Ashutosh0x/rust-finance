use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Serialize, Deserialize, Clone)]
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

fn default_use_mock() -> String { "0".to_string() }
fn default_alpaca_base_url() -> String {
    "https://paper-api.alpaca.markets".to_string()
}
fn default_log_level() -> String { "info".to_string() }

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
        }
        
        if !self.alpaca_base_url.starts_with("http") {
             return Err(format!("Invalid Alpaca URL format: {}", self.alpaca_base_url));
        }

        Ok(())
    }

    pub fn ai_enabled(&self) -> bool {
        self.anthropic_api_key.as_ref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false)
    }
}
