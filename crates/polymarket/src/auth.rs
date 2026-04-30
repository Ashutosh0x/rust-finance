// crates/polymarket/src/auth.rs

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;
use tracing::{error, info};

type HmacSha256 = Hmac<Sha256>;

/// L2 API credentials (derived from L1 signing or stored)
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub api_passphrase: String,
}

impl fmt::Debug for ApiCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiCredentials")
            .field("api_key", &redact(&self.api_key))
            .field("api_secret", &"<redacted>")
            .field("api_passphrase", &"<redacted>")
            .finish()
    }
}

fn redact(value: &str) -> String {
    let prefix: String = value.chars().take(4).collect();
    if prefix.is_empty() {
        "<redacted>".into()
    } else {
        format!("{prefix}...<redacted>")
    }
}

/// L1 auth header: sign a timestamp with the private key
/// Used only for deriving/creating L2 credentials
pub struct L1Auth {
    wallet: ethers_signers::LocalWallet,
}

impl L1Auth {
    pub fn new(wallet: ethers_signers::LocalWallet) -> Self {
        Self { wallet }
    }

    /// Create L1 auth headers for the /derive-api-key or /create-api-key endpoint
    pub async fn create_l1_headers(
        &self,
        timestamp: i64,
        nonce: u64,
    ) -> Result<HeaderMap, Box<dyn std::error::Error>> {
        use ethers_core::types::H256;
        use ethers_core::utils::keccak256;
        use ethers_signers::Signer;

        // The L1 auth message format Polymarket expects
        let message = format!("{}{}", timestamp, nonce);
        let msg_hash = H256::from(keccak256(message.as_bytes()));

        let signature = self.wallet.sign_hash(msg_hash)?;
        let sig_hex = format!("0x{}", hex::encode(signature.to_vec()));

        let mut headers = HeaderMap::new();
        headers.insert(
            "POLY_ADDRESS",
            HeaderValue::from_str(&format!("{:?}", self.wallet.address()))?,
        );
        headers.insert("POLY_SIGNATURE", HeaderValue::from_str(&sig_hex)?);
        headers.insert(
            "POLY_TIMESTAMP",
            HeaderValue::from_str(&timestamp.to_string())?,
        );
        headers.insert("POLY_NONCE", HeaderValue::from_str(&nonce.to_string())?);

        Ok(headers)
    }

    /// Derive or create API credentials from the CLOB
    pub async fn derive_api_credentials(
        &self,
        clob_host: &str,
    ) -> Result<ApiCredentials, Box<dyn std::error::Error>> {
        let http = reqwest::Client::new();
        let timestamp = chrono::Utc::now().timestamp();
        let nonce = 0u64;

        let headers = self.create_l1_headers(timestamp, nonce).await?;

        // Try derive-api-key first (idempotent, returns existing if present)
        let resp = http
            .get(format!("{}/auth/derive-api-key", clob_host))
            .headers(headers.clone())
            .send()
            .await?;

        if resp.status().is_success() {
            let creds: ApiCredentials = resp.json().await?;
            info!("L2 API credentials derived successfully");
            return Ok(creds);
        }

        // If derive fails, try create
        let resp = http
            .post(format!("{}/auth/api-key", clob_host))
            .headers(headers)
            .send()
            .await?;

        if resp.status().is_success() {
            let creds: ApiCredentials = resp.json().await?;
            info!("L2 API credentials created successfully");
            Ok(creds)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("Failed to get API credentials: {} - {}", status, body);
            Err(format!("Auth failed: {} - {}", status, body).into())
        }
    }
}

/// L2 HMAC auth: used for all authenticated CLOB requests
pub struct L2Auth {
    credentials: ApiCredentials,
}

impl L2Auth {
    pub fn new(credentials: ApiCredentials) -> Self {
        Self { credentials }
    }

    /// Build authentication headers for an L2 (HMAC) request
    pub fn build_headers(
        &self,
        method: &str,
        path: &str,
        body: &str,
    ) -> Result<HeaderMap, Box<dyn std::error::Error>> {
        if self.credentials.api_key.trim().is_empty()
            || self.credentials.api_secret.trim().is_empty()
            || self.credentials.api_passphrase.trim().is_empty()
        {
            return Err("Polymarket L2 credentials cannot be empty".into());
        }

        let timestamp = chrono::Utc::now().timestamp().to_string();

        // HMAC message: timestamp + method + path + body
        let message = format!("{}{}{}{}", timestamp, method, path, body);

        // Sign with API secret
        let secret_bytes = BASE64.decode(&self.credentials.api_secret)?;
        let mut mac =
            HmacSha256::new_from_slice(&secret_bytes).map_err(|e| format!("HMAC error: {}", e))?;
        mac.update(message.as_bytes());
        let signature = BASE64.encode(mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert(
            "POLY_HMAC_KEY",
            HeaderValue::from_str(&self.credentials.api_key)?,
        );
        headers.insert("POLY_HMAC_SIGNATURE", HeaderValue::from_str(&signature)?);
        headers.insert("POLY_HMAC_TIMESTAMP", HeaderValue::from_str(&timestamp)?);
        headers.insert(
            "POLY_HMAC_PASSPHRASE",
            HeaderValue::from_str(&self.credentials.api_passphrase)?,
        );
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));

        Ok(headers)
    }

    pub fn api_key_for_diagnostics(&self) -> String {
        redact(&self.credentials.api_key)
    }
}
