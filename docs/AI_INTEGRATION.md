# AI & RL Integration

RustForge relies on two distinct intelligence systems: A foundational LLM for deep sentiment and fundamental extraction, and an RL agent to formulate tactical execution maps.

## Anthropic Claude API (DexterAnalyst)
- Analyzes unstructured text (SEC filings, Twitter Firehose, macro news) via the `json_schema` strict enforcement pipeline.
- Translates qualitative news into quantitative bias modifiers (e.g., probability drift +/- 0.1).
- Rate-limited to prevent token bucket exhaustion.

## Proximal Policy Optimization (PPO Agent)
- An Actor-Critic neural backbone taking the full order-book state array.
- Injects the LLM bias directly into the hidden state evaluation stage.
- Output deterministic argmax selections over `Action::Buy`, `Action::Sell`, `Action::Hold`.
