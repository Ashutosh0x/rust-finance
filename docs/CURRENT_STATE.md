# RustForge — Current State

**Version:** 1.0.0
**Date:** March 2026
**Status:** Production

---

## Workspace Overview

- **Active crates:** 30 (compiling in ~17s)
- **Excluded crates:** 4 (legacy Solana: `parser`, `executor`, `signer`, `relay`)
- **Source files:** 110+
- **Build status:** `cargo check --workspace` passes with 0 errors

---

## Architecture

### Hybrid Intelligence Pipeline

```
Market Data (Alpaca / Binance / Finnhub / Polymarket)
    |
    v
Ingestion Layer (MarketDataSource trait, Source Multiplexer)
    |
    v
Event Bus (Postcard-serialized TCP, broadcast channels)
    |
    v
Quant Features --> Swarm Simulator (100K agents) --> Knowledge Graph (petgraph RAG)
    |                                                        |
    v                                                        v
Fused Context ----> Dexter AI Analyst (Claude) ----> Signal Output
    |
    v
Strategy Dispatcher (PluggableStrategy trait)
    |
    v
Risk Gate (GARCH / VaR / Kill Switch / Interceptor Chain)
    |
    v
Execution Gateway (AlpacaExecutor / Polymarket CLOB / MockExecutor)
```

### Core Types (v2)

| Type | Purpose | Location |
|:---|:---|:---|
| `UnixNanos` | Nanosecond-precision timestamp | `common/src/time.rs` |
| `SequenceId` | Monotonic event ordering | `common/src/time.rs` |
| `Envelope<T>` | Universal event wrapper with ordering | `common/src/events.rs` |
| `MarketEvent` | Normalized market data (Trade/Quote/BookUpdate/Bar) | `common/src/events.rs` |
| `EngineEvent` | Top-level engine event union | `common/src/events.rs` |
| `OrderEvent` | Order lifecycle (Submitted/Accepted/Filled/Rejected/Cancelled) | `common/src/events.rs` |
| `AuditTick` | Full replay audit trail | `common/src/events.rs` |
| `BotEvent` | v1 event bus wire type (backward compat) | `common/src/events.rs` |
| `ControlCommand` | TUI-to-daemon control commands | `common/src/events.rs` |
| `Clock` trait | Swappable clock (Realtime / Deterministic) | `common/src/time.rs` |

---

## Implemented Features

### Market Data Sources
- [x] Alpaca WebSocket (5 feeds: IEX, SIP, BOATS, Delayed, Overnight)
- [x] Binance WebSocket (crypto: trades, bookTicker, depth5)
- [x] Finnhub WebSocket (global market data incl. NSE/BSE)
- [x] Polymarket CLOB (prediction markets: order book, midpoint, trades)
- [x] Mock source (deterministic replay)
- [x] Source Multiplexer (unified SelectAll stream)
- [x] Auto-reconnect with exponential backoff

### Execution
- [x] ExecutionGateway trait (plug-and-play backends)
- [x] Alpaca REST executor (25+ endpoints: orders, positions, assets, historical)
- [x] Polymarket CLOB (limit/market/FOK/GTC/GTD orders, EIP-712 signing)
- [x] Paper trading (MockExecutor)
- [x] Bracket orders (OCO/OTO)
- [x] Trailing stops

### AI and Intelligence
- [x] Dexter AI analyst (Claude-powered structured signals)
- [x] Mirofish scenario simulator (5K agents, rally/sideways/dip)
- [x] 100K agent swarm simulation (Rayon parallel)
- [x] Knowledge graph (petgraph RAG)
- [x] Fused context pipeline (Quant + Swarm + Graph -> Dexter)
- [x] Impact analysis engine

### Risk Management
- [x] Kill switch (emergency halt)
- [x] GARCH(1,1) volatility estimation
- [x] Value at Risk (VaR)
- [x] PnL attribution
- [x] Risk interceptor chain (composable pre-trade checks)
- [x] Kelly criterion position sizing
- [x] Max drawdown / daily loss limit

### Quantitative Models
- [x] Black-Scholes-Merton (options pricing with Greeks)
- [x] Heston stochastic volatility
- [x] GARCH(1,1) forecasting
- [x] Monte Carlo simulation
- [x] Walk-forward backtesting
- [x] Latency queue simulation

### TUI Dashboard
- [x] 6-screen navigation (Dashboard, Charts, Orderbook, Positions, AI, Settings)
- [x] Real-time sparkline charts with zoom/scroll
- [x] Live L2 order book with cumulative volume
- [x] 13-symbol watchlist
- [x] Exchange heartbeat monitor (8 exchanges)
- [x] Dexter AI panel with recommendations
- [x] Mirofish simulation widget
- [x] Buy/Sell order dialogs
- [x] Emergency controls (kill switch, paper/live toggle, risk adjust)

### Infrastructure
- [x] Postcard-serialized TCP event bus
- [x] PostgreSQL + SQLite persistence
- [x] Prometheus-compatible metrics
- [x] Pre-trade compliance and audit trail
- [x] Deterministic clock for replay

---

## Excluded Legacy Crates

| Crate | Reason | v2 Replacement |
|:---|:---|:---|
| `parser` | Solana SDK v1.18.x has yanked `solana_rbpf =0.8.0` | `crates/ingestion` (trait-based `MarketDataSource`) |
| `executor` | Same Solana dependency conflict | `crates/execution` (trait-based `ExecutionGateway`) |
| `signer` | Dependency of `executor` | `crates/execution` (auth handled per executor) |
| `relay` | Dependency of `executor` | `crates/ingestion` (built into each source) |

---

## Build and Run

```bash
# Full workspace check
cargo check --workspace

# Release build
cargo build --release

# Mock mode (no API keys)
USE_MOCK=1 cargo run -p daemon --release

# TUI (separate terminal)
cargo run -p tui --release
```

---

## Dependencies

Key workspace dependencies:
- `tokio 1.37` (async runtime)
- `serde >=1.0.228` (serialization)
- `chrono 0.4.38` (time)
- `tracing 0.1.40` (structured logging)
- `async-trait 0.1.80` (async traits)
- `ratatui` (TUI rendering)
- `ethers-core / ethers-signers` (Polymarket EIP-712)
- `postcard` (binary serialization for event bus)
- `petgraph` (knowledge graph)
- `rayon` (parallel swarm simulation)

---

## API Keys Required

| Service | Environment Variable | Purpose | Free Tier |
|:---|:---|:---|:---|
| Alpaca | `ALPACA_API_KEY`, `ALPACA_API_SECRET` | US equities market data + execution | Yes (paper trading) |
| Finnhub | `FINNHUB_API_KEY` | Market data + news API | Yes (60 calls/min) |
| Anthropic | `ANTHROPIC_API_KEY` | Dexter AI analyst (Claude) | No (pay-per-token) |
| NewsAPI.org | `NEWSAPI_KEY` | Aggregated news (Reuters, Bloomberg, WSJ) | Yes (100 req/day) |
| Polygon.io | `POLYGON_API_KEY` | Options chains (GEX), reference data, news | Yes (5 calls/min) |
| Polymarket | `POLYMARKET_PRIVATE_KEY`, `POLYMARKET_FUNDER_ADDRESS` | Prediction market trading (EIP-712) | N/A (needs ETH wallet) |
| Telegram | `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID` | Alert notifications | Yes |
| Discord | `DISCORD_WEBHOOK_URL` | Alert notifications | Yes |

---

## News Feed Sources

| Source | Method | Content |
|:---|:---|:---|
| Finnhub News API | REST `/api/v1/news` | General market news, company news, sector news |
| Alpaca News API | REST `/v1beta1/news` | US equities breaking news, earnings, SEC filings |
| NewsAPI.org | REST | Aggregates Reuters, Bloomberg, CNBC, WSJ, Financial Times |
| Polygon.io | REST `/v2/reference/news` | SEC filings, earnings reports, company reference data |
| BSE/NSE RSS | HTTP + XML | Indian market news (free, no key needed) |
| CoinGecko | REST | Cryptocurrency market news and sentiment (free) |
| SEC EDGAR | REST | Regulatory filings: 10-K, 10-Q, 8-K (free, no key needed) |
