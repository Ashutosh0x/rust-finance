// ============================================================
// crates/ai/src/dexter.rs
//
// Dexter — Financial Analyst AI
// Produces structured trade signals from the FusedContext.
// Uses Groq-hosted LLM (OpenAI-compatible API) for low-latency inference.
// ============================================================

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

// use crate::hybrid_pipeline::FusedContext; // FusedContext lives in daemon module
// We will access FusedContext directly during compilation if possible,
// or define DexterSignal here.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexterSignal {
    pub symbol: String,
    pub direction: TradeDirection,
    pub confidence: f64, // 0.0–1.0
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub position_size_pct: f64, // % of portfolio to allocate
    pub time_horizon: TimeHorizon,
    pub thesis: String,           // 2-3 sentences, specific and quantitative
    pub key_risks: Vec<String>,   // Max 3 concrete risk factors
    pub catalyst: Option<String>, // Specific event that could move price
    pub valuation: Option<ValuationMetrics>,
    pub recommendation: Recommendation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradeDirection {
    Long,
    Short,
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimeHorizon {
    Intraday,
    Swing,    // 2-5 days
    Position, // weeks
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Recommendation {
    Buy,
    Sell,
    Hold,
    Risk, // Hedge/reduce risk
}

/// Matches what you see in the TUI Dexter panel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValuationMetrics {
    pub pe_ratio: Option<f64>,
    pub ps_ratio: Option<f64>,
    pub ev_ebitda: Option<f64>,
    pub dcf_range_low: Option<f64>,
    pub dcf_range_high: Option<f64>,
    pub revenue_impact_usd_millions: Option<f64>,
    pub margin_change_pct: Option<f64>,
}

// ── The analyst function ─────────────────────────────────────────────────────

// Using a generic bound for context to avoid circular dependency with daemon
pub trait FusedContextLike {
    fn to_dexter_system_prompt(&self) -> String;
    fn get_symbol(&self) -> String;
}

#[allow(unused_imports)]
pub async fn analyse<T: FusedContextLike>(ctx: &T) -> Result<DexterSignal> {
    let system_prompt = ctx.to_dexter_system_prompt();
    let user_prompt = build_user_prompt(&ctx.get_symbol());

    let raw = call_llm_json(&system_prompt, &user_prompt).await?;

    // Heal common JSON issues before parsing
    let healed = heal_json(&raw);
    let preview: String = healed.chars().take(200).collect();
    let mut signal: DexterSignal = serde_json::from_str(&healed)
        .with_context(|| format!("Dexter JSON parse failed: {}", preview))?;

    // ── Post-parse constraint enforcement (never trust prompt-following) ──
    signal.confidence = signal.confidence.clamp(0.0, 1.0);
    signal.position_size_pct = signal.position_size_pct.clamp(0.0, 0.10);
    if signal.confidence < 0.70 {
        signal.position_size_pct = signal.position_size_pct.min(0.05);
    }
    // Ensure stop_loss is on the correct side of entry
    match signal.direction {
        TradeDirection::Long => {
            if signal.stop_loss > signal.entry_price {
                signal.stop_loss = signal.entry_price * 0.95;
            }
        }
        TradeDirection::Short => {
            if signal.stop_loss < signal.entry_price {
                signal.stop_loss = signal.entry_price * 1.05;
            }
        }
        TradeDirection::Neutral => {}
    }

    let thesis_preview: String = signal.thesis.chars().take(80).collect();
    info!(
        "Dexter → {} {:?} conf={:.2} size={:.2}% thesis={}",
        signal.symbol,
        signal.direction,
        signal.confidence,
        signal.position_size_pct * 100.0,
        thesis_preview,
    );

    Ok(signal)
}

fn build_user_prompt(symbol: &str) -> String {
    format!(
        r#"Analyse {symbol} using the quantitative data, swarm simulation, and knowledge graph above.

Return ONLY a JSON object matching this schema — no preamble, no markdown:

{{
  "symbol": "{symbol}",
  "direction": "Long" | "Short" | "Neutral",
  "confidence": 0.0-1.0,
  "entry_price": float,
  "stop_loss": float,
  "take_profit": float,
  "position_size_pct": 0.0-0.10,
  "time_horizon": "Intraday" | "Swing" | "Position",
  "thesis": "2-3 specific sentences with numbers from the data above",
  "key_risks": ["risk1", "risk2", "risk3"],
  "catalyst": "specific event or null",
  "valuation": {{
    "pe_ratio": float | null,
    "ps_ratio": float | null,
    "ev_ebitda": float | null,
    "dcf_range_low": float | null,
    "dcf_range_high": float | null,
    "revenue_impact_usd_millions": float | null,
    "margin_change_pct": float | null
  }},
  "recommendation": "Buy" | "Sell" | "Hold" | "Risk"
}}

Rules:
- Use ONLY data from the context above — no hallucinated numbers
- Thesis must cite specific figures (RSI level, swarm %, graph relationships)
- stop_loss and take_profit must be realistic given current GARCH volatility
- position_size_pct must be ≤ 0.05 if confidence < 0.70
"#,
        symbol = symbol
    )
}

/// Call the LLM backend (auto-detects Ollama vs Groq from env).
///
/// Provider selection:
///   LLM_PROVIDER=ollama  → Ollama at localhost:11434 (no rate limits)
///   LLM_PROVIDER=groq    → Groq cloud API (rate limited)
///   (default)            → Ollama if OLLAMA_MODEL is set, else Groq
async fn call_llm_json(system: &str, user: &str) -> Result<String> {
    let provider = std::env::var("LLM_PROVIDER")
        .unwrap_or_default()
        .to_lowercase();
    let ollama_model = std::env::var("OLLAMA_MODEL").ok();

    let use_ollama = provider == "ollama" || (provider.is_empty() && ollama_model.is_some());

    if use_ollama {
        call_ollama_json(system, user).await
    } else {
        call_groq_json(system, user).await
    }
}

/// Ollama backend — OpenAI-compatible API at localhost:11434. No rate limits.
async fn call_ollama_json(system: &str, user: &str) -> Result<String> {
    let model =
        std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "deepseek-v3.1:671b-cloud".to_string());
    let host =
        std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600)) // 10min — qwen3:8b on 4GB VRAM takes ~90s/call
        .build()?;

    // Use Ollama native /api/chat endpoint with JSON format enforcement
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
        "format": "json",
        "stream": false,
        "options": {
            "temperature": 0.3
        }
    });

    let resp = client
        .post(format!("{}/api/chat", host))
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let json: serde_json::Value = resp.json().await?;

    if !status.is_success() {
        let err_msg = json
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown Ollama error");
        anyhow::bail!("Ollama error ({}): {}", status, err_msg);
    }

    // Ollama native response: { "message": { "role": "assistant", "content": "..." } }
    if let Some(content) = json
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
    {
        return Ok(content.to_string());
    }

    anyhow::bail!("Ollama response missing message.content")
}

/// Groq cloud backend — OpenAI-compatible API with rate limits.
async fn call_groq_json(system: &str, user: &str) -> Result<String> {
    let api_key = std::env::var("GROQ_API_KEY").unwrap_or_else(|_| "mock_key".to_string());
    let model = std::env::var("GROQ_MODEL").unwrap_or_else(|_| "openai/gpt-oss-120b".to_string());
    let client = reqwest::Client::new();

    // Detect if using a reasoning model (openai/gpt-oss-*)
    let is_reasoning = model.starts_with("openai/gpt-oss");

    let body = if is_reasoning {
        serde_json::json!({
            "model": model,
            "max_completion_tokens": 2048,
            "reasoning_effort": "low",
            "temperature": 1,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        })
    } else {
        serde_json::json!({
            "model": model,
            "max_tokens": 4096,
            "temperature": 0.3,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        })
    };

    let resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let json: serde_json::Value = resp.json().await?;

    if !status.is_success() {
        let err_msg = json
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown API error");
        anyhow::bail!("Groq API error ({}): {}", status, err_msg);
    }

    // OpenAI-compatible response: choices[0].message.content
    if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
        if let Some(first) = choices.first() {
            if let Some(text) = first
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|t| t.as_str())
            {
                return Ok(text.to_string());
            }
        }
    }

    anyhow::bail!("Groq response missing choices[0].message.content")
}

/// Heal common LLM JSON output issues before parsing.
/// Handles: Qwen3 think blocks, markdown fences, trailing commas, bare JSON extraction.
fn heal_json(raw: &str) -> String {
    let trimmed = raw.trim();

    // ── Strip Qwen3 <think>...</think> blocks ──
    // Qwen3 in thinking mode wraps reasoning in <think>...</think> before the JSON.
    // Handle multiple blocks and unclosed tags gracefully.
    let trimmed = strip_think_blocks(trimmed);
    let trimmed = trimmed.trim();

    // Strip markdown code fences (```json ... ``` or ``` ... ```)
    let stripped = if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.rfind("```") {
            after[..end].trim().to_string()
        } else {
            after.trim().to_string()
        }
    } else if trimmed.starts_with("```") {
        let after = &trimmed[3..];
        if let Some(end) = after.rfind("```") {
            after[..end].trim().to_string()
        } else {
            after.trim().to_string()
        }
    } else {
        // Extract bare JSON object from any surrounding text
        if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
            trimmed[start..=end].to_string()
        } else {
            trimmed.to_string()
        }
    };

    // Fix trailing commas before } or ] (common LLM mistake)
    stripped.replace(",}", "}").replace(",]", "]")
}

/// Strip `<think>...</think>` blocks emitted by Qwen3 in thinking mode.
/// Handles multiple blocks, nested tags, and unclosed `<think>` gracefully.
fn strip_think_blocks(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut remaining = input;

    loop {
        // Find the next <think> tag (case-insensitive)
        let lower = remaining.to_lowercase();
        if let Some(start) = lower.find("<think>") {
            // Keep everything before <think>
            result.push_str(&remaining[..start]);

            let after_open = &remaining[start + 7..]; // skip "<think>"
            let after_lower = after_open.to_lowercase();

            if let Some(end) = after_lower.find("</think>") {
                // Skip past </think> and continue
                remaining = &after_open[end + 8..]; // skip "</think>"
            } else {
                // Unclosed <think> — discard everything after it
                // (the thinking block ran to the end of the string)
                break;
            }
        } else {
            // No more <think> blocks — keep the rest
            result.push_str(remaining);
            break;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heal_json_strips_think_blocks() {
        let raw = r#"<think>
Let me analyze the RSI data and swarm signals...
The bullish probability is 67% which suggests a long bias.
</think>
{"symbol":"NVDA","direction":"Long","confidence":0.82}"#;

        let healed = heal_json(raw);
        assert!(healed.starts_with('{'));
        assert!(healed.contains("\"NVDA\""));
        assert!(!healed.contains("<think>"));
        assert!(!healed.contains("</think>"));
    }

    #[test]
    fn heal_json_handles_multiple_think_blocks() {
        let raw = r#"<think>first thought</think>
<think>second thought</think>
{"symbol":"AAPL"}"#;

        let healed = heal_json(raw);
        assert!(healed.contains("\"AAPL\""));
        assert!(!healed.contains("thought"));
    }

    #[test]
    fn heal_json_handles_no_think_blocks() {
        let raw = r#"{"symbol":"TSLA","direction":"Short"}"#;
        let healed = heal_json(raw);
        assert_eq!(healed, raw);
    }

    #[test]
    fn heal_json_handles_unclosed_think() {
        let raw = r#"<think>reasoning forever..."#;
        let healed = heal_json(raw);
        // Unclosed think = everything discarded, heal returns empty-ish
        assert!(!healed.contains("reasoning"));
    }

    #[test]
    fn heal_json_strips_think_then_markdown_fence() {
        let raw = r#"<think>analyzing...</think>
```json
{"symbol":"SPY","confidence":0.9}
```"#;

        let healed = heal_json(raw);
        assert!(healed.contains("\"SPY\""));
        assert!(!healed.contains("```"));
        assert!(!healed.contains("<think>"));
    }
}
