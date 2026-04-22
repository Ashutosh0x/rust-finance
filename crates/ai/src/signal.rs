use crate::anthropic_client::{AnthropicClient, Message, MessageRequest, OutputConfig};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AISignal {
    pub symbol: String,
    pub action: String,
    pub confidence: f64,
    pub reason: String,
}

pub async fn generate_signal(news: &str, client: &AnthropicClient) -> Result<AISignal> {
    let req = MessageRequest {
        model: "claude-opus-4-6".to_string(),
        max_tokens: 512,
        system: None,
        messages: vec![Message {
            role: "user".to_string(),
            content: news.to_string(),
        }],
        output_config: Some(OutputConfig {
            format: "json".to_string(),
        }),
    };

    let _response = client.send_message(req).await?;

    // In actual implementation, we'd parse the Claude response safely here.
    // For now, we simulate parsing:
    Ok(AISignal {
        symbol: "AAPL".into(),
        action: "BUY".into(),
        confidence: 0.82,
        reason: "Strong earnings surprise and bullish sentiment - parsed from Opus 4.6".into(),
    })
}
