use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct MessageRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<OutputConfig>,
}

#[derive(Serialize, Clone)]
pub struct OutputConfig {
    pub format: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize, Debug)]
pub struct MessageResponse {
    pub content: Vec<ContentBlock>,
}

#[derive(Deserialize, Debug)]
pub struct ContentBlock {
    pub text: Option<String>,
}

pub struct AnthropicClient {
    client: Client,
    api_key: String,
}

impl AnthropicClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub async fn send_message(&self, req: MessageRequest) -> Result<String> {
        let url = "https://api.anthropic.com/v1/messages";
        let res = self
            .client
            .post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&req)
            .send()
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await?;
            return Err(anyhow::anyhow!("Anthropic API error {}: {}", status, body));
        }

        let resp: MessageResponse = res.json().await?;

        let text = resp
            .content
            .into_iter()
            .find_map(|b| b.text)
            .unwrap_or_else(|| "No text response".to_string());

        Ok(text)
    }
}
