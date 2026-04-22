use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PolymarketConfig {
    /// Private key for signing (hex, with or without 0x)
    pub private_key: String,
    /// Proxy wallet address (Gnosis Safe) that holds funds
    pub funder_address: String,
    /// Signature type: 0=EOA, 1=Magic/Email, 2=Gnosis Safe
    #[serde(default = "default_sig_type")]
    pub signature_type: u8,
    /// CLOB API host
    #[serde(default = "default_clob_host")]
    pub clob_host: String,
    /// Gamma API host
    #[serde(default = "default_gamma_host")]
    pub gamma_host: String,
    /// Data API host
    #[serde(default = "default_data_host")]
    pub data_host: String,
    /// WebSocket market channel URL
    #[serde(default = "default_ws_market")]
    pub ws_market_url: String,
    /// Chain ID (137 = Polygon Mainnet)
    #[serde(default = "default_chain_id")]
    pub chain_id: u64,
    /// Dry run mode
    #[serde(default)]
    pub dry_run: bool,
    /// Copy trading target addresses - represented as a single comma-separated string from env
    #[serde(default, deserialize_with = "deserialize_comma_separated")]
    pub copy_target_addresses: Vec<String>,
    /// Copy trade size as percentage of target
    #[serde(default = "default_copy_pct")]
    pub copy_size_percent: f64,
}

fn deserialize_comma_separated<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    Ok(s.unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

fn default_sig_type() -> u8 {
    2
}
fn default_clob_host() -> String {
    "https://clob.polymarket.com".into()
}
fn default_gamma_host() -> String {
    "https://gamma-api.polymarket.com".into()
}
fn default_data_host() -> String {
    "https://data-api.polymarket.com".into()
}
fn default_ws_market() -> String {
    "wss://ws-subscriptions-clob.polymarket.com/ws/market".into()
}
fn default_chain_id() -> u64 {
    137
}
fn default_copy_pct() -> f64 {
    10.0
}

impl PolymarketConfig {
    pub fn from_env() -> Result<Self, envy::Error> {
        dotenvy::dotenv().ok();
        envy::prefixed("POLYMARKET_").from_env()
    }
}
