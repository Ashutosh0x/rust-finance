# WebSocket Ingestion & Normalization

The ingestion crate serves as the critical entry point to RustForge.

## Finnhub vs Alpaca
- Finnhub is prioritized for absolute lowest latency crypto and stock consolidated tape data.
- Alpaca is utilized as a fallback or secondary confirmation source.
- Both streams are unified via the `Normalizer` struct into a single `BotEvent::MarketEvent` variant.

## Reconnection Logic
Network resilience is managed via the `tokio-retry` crate utilizing a strict `ExponentialBackoff`.
Instead of panic/crashing internally, the clients sleep and retry with a factor multiplier (integer based, max delay 10s-30s depending on the host), ensuring the daemon survives overnight without manual restarts.
