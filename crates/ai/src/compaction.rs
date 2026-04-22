// crates/ai/src/compaction.rs
//
// Anthropic Compaction API integration.
// Manages rolling multi-week token histories via server-side summarization,
// transparently splicing compacted summaries back into message history.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use thiserror::Error;
use tracing::{debug, info, warn};

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const COMPACTION_BETA: &str = "message-batches-2024-09-24,interleaved-thinking-2025-05-14";

/// A single message in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String, // "user" | "assistant"
    pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    /// Injected by the Compaction API — contains the rolling summary.
    ServerToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ServerToolResult {
        tool_use_id: String,
        content: Vec<ContentBlock>,
    },
}

/// Request body for /v1/messages
#[derive(Debug, Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: Option<&'a str>,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "Option::is_none")]
    betas: Option<Vec<&'a str>>,
}

/// Subset of the response we care about.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    stop_reason: String,
    usage: UsageStats,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct UsageStats {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}

/// Errors from the Compaction-aware client.
#[derive(Debug, Error)]
pub enum CompactionError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Configuration for the compaction-aware client.
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    pub model: String,
    pub max_tokens: u32,
    /// Token threshold at which we request compaction.
    pub compaction_threshold: u32,
    /// Maximum messages to retain in-memory before forcing compaction.
    pub max_history_messages: usize,
    pub system_prompt: Option<String>,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            model: "claude-opus-4-6".to_string(),
            max_tokens: 4096,
            compaction_threshold: 150_000,
            max_history_messages: 200,
            system_prompt: None,
        }
    }
}

/// Compaction-aware Claude client.
///
/// Maintains a rolling conversation history and automatically invokes
/// the Compaction API when the token budget nears the threshold,
/// replacing old messages with a compact server-side summary.
pub struct CompactionClient {
    http: reqwest::Client,
    api_key: String,
    cfg: CompactionConfig,
    history: VecDeque<Message>,
    total_input_tokens: u64,
    compaction_count: u32,
}

impl CompactionClient {
    pub fn new(api_key: String, cfg: CompactionConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            cfg,
            history: VecDeque::new(),
            total_input_tokens: 0,
            compaction_count: 0,
        }
    }

    /// Send a user message and get the assistant's reply.
    /// Automatically handles compaction when the token threshold is reached.
    pub async fn send(&mut self, user_message: &str) -> Result<String, CompactionError> {
        self.history.push_back(Message {
            role: "user".to_string(),
            content: MessageContent::Text(user_message.to_string()),
        });

        let messages: Vec<Message> = self.history.iter().cloned().collect();
        let response = self.call_api(&messages, false).await?;

        let reply_text = self.extract_text(&response.content);

        self.history.push_back(Message {
            role: "assistant".to_string(),
            content: MessageContent::Text(reply_text.clone()),
        });

        self.total_input_tokens += response.usage.input_tokens as u64;

        debug!(
            input_tokens = response.usage.input_tokens,
            output_tokens = response.usage.output_tokens,
            total_input_lifetime = self.total_input_tokens,
            "API call completed"
        );

        // Check if we need to compact
        if response.usage.input_tokens >= self.cfg.compaction_threshold
            || self.history.len() > self.cfg.max_history_messages
        {
            self.compact().await?;
        }

        Ok(reply_text)
    }

    /// Force a compaction pass regardless of threshold.
    pub async fn compact(&mut self) -> Result<(), CompactionError> {
        info!(
            history_len = self.history.len(),
            "Requesting Compaction API pass"
        );

        let messages: Vec<Message> = self.history.iter().cloned().collect();
        let response = self.call_api(&messages, true).await?;

        // The Compaction API returns a truncated history in the response.
        // We replace our in-memory history with the compacted form returned
        // as ServerToolUse/ServerToolResult blocks in the response.
        let compacted_summary = self.extract_compacted_history(&response.content);

        if let Some(summary_msg) = compacted_summary {
            // Retain only messages after the last compaction point plus the summary
            self.history.clear();
            self.history.push_back(summary_msg);
            self.compaction_count += 1;
            info!(
                compaction_count = self.compaction_count,
                "History compacted successfully"
            );
        } else {
            warn!("Compaction API did not return a summary block — history unchanged");
        }

        Ok(())
    }

    async fn call_api(
        &self,
        messages: &[Message],
        request_compaction: bool,
    ) -> Result<MessagesResponse, CompactionError> {
        let mut betas = vec![];
        if request_compaction {
            betas.push("extended-cache-ttl-2025-02-19");
        }

        let body = MessagesRequest {
            model: &self.cfg.model,
            max_tokens: self.cfg.max_tokens,
            system: self.cfg.system_prompt.as_deref(),
            messages,
            betas: if betas.is_empty() { None } else { Some(betas) },
        };

        let mut req = self
            .http
            .post(format!("{ANTHROPIC_API_BASE}/messages"))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", COMPACTION_BETA)
            .json(&body);

        if request_compaction {
            req = req.header("x-compaction-request", "true");
        }

        let resp = req.send().await?;
        let status = resp.status().as_u16();

        if status != 200 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(CompactionError::Api {
                status,
                body: body_text,
            });
        }

        Ok(resp.json::<MessagesResponse>().await?)
    }

    fn extract_text(&self, blocks: &[ContentBlock]) -> String {
        blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn extract_compacted_history(&self, blocks: &[ContentBlock]) -> Option<Message> {
        // Look for a ServerToolResult block which contains the compacted summary
        for block in blocks {
            if let ContentBlock::ServerToolResult { content, .. } = block {
                let summary_text = self.extract_text(content);
                if !summary_text.is_empty() {
                    return Some(Message {
                        role: "user".to_string(),
                        content: MessageContent::Text(format!(
                            "[COMPACTED CONTEXT SUMMARY]\n{summary_text}"
                        )),
                    });
                }
            }
        }
        None
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn compaction_count(&self) -> u32 {
        self.compaction_count
    }

    pub fn total_input_tokens(&self) -> u64 {
        self.total_input_tokens
    }
}
