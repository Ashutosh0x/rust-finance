# RustForge v1.0.0 — Production Release

## RustForge v1.0.0
### Institutional-Grade AI Trading Terminal in Rust

![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)
![Tokio](https://img.shields.io/badge/Tokio-000000?style=for-the-badge)
![License](https://img.shields.io/badge/License-MIT-blue?style=for-the-badge)
![Platform](https://img.shields.io/badge/Platform-Windows%20|%20macOS%20|%20Linux-green?style=for-the-badge)

![Alpaca](https://img.shields.io/badge/Alpaca-FCD535?style=flat-square&logo=alpaca&logoColor=black)
![Binance](https://img.shields.io/badge/Binance-FCD535?style=flat-square&logo=binance&logoColor=black)
![Polymarket](https://img.shields.io/badge/Polymarket-0052FF?style=flat-square)
![Finnhub](https://img.shields.io/badge/Finnhub-00C805?style=flat-square)
![Claude AI](https://img.shields.io/badge/Claude_AI-7C3AED?style=flat-square&logo=anthropic&logoColor=white)

[Documentation](docs/) | [Features](#whats-included) | [Security](#security) | [Changelog](CHANGELOG.md)

---

## Quick Install

**From source:**
```bash
git clone https://github.com/Ashutosh0x/rust-finance.git
cd rust-finance
cargo build --release
```

**Run the terminal:**
```bash
# Launch the TUI trading dashboard
cargo run --bin tui

# Launch the daemon (headless mode)
cargo run --bin daemon

# Launch the CLI
cargo run --bin cli
```

---

## Downloads

| Platform | Architecture | File | Size |
|:---|:---|:---|:---|
| Windows | x86_64 | `rustforge-v1.0.0-x86_64-pc-windows-msvc.zip` | ~12 MB |
| macOS | Intel | `rustforge-v1.0.0-x86_64-apple-darwin.tar.gz` | ~11 MB |
| macOS | Apple Silicon (M1/M2/M3) | `rustforge-v1.0.0-aarch64-apple-darwin.tar.gz` | ~10 MB |
| Linux | x86_64 (static musl) | `rustforge-v1.0.0-x86_64-unknown-linux-musl.tar.gz` | ~12 MB |
| Linux | ARM64 (static musl) | `rustforge-v1.0.0-aarch64-unknown-linux-musl.tar.gz` | ~11 MB |

Verify integrity: `sha256sum -c SHA256SUMS.txt`

---

## Overview

RustForge is a native institutional-grade trading terminal built in pure Rust. It combines real-time market data, AI-powered analysis, quantitative risk management, and prediction market trading in a single TUI dashboard with sub-millisecond latency and nanosecond-precision timestamps.

| Feature | Detail |
|:---|:---|
| **Language** | Pure Rust |
| **Interface** | Full TUI Dashboard (Ratatui, 6 screens) |
| **AI Integration** | Claude-powered Dexter Analyst |
| **Prediction Markets** | Polymarket CLOB with EIP-712 signing |
| **Agent Simulation** | 100K-agent Rayon-parallel swarm |
| **Knowledge Graph** | petgraph-backed RAG engine |
| **Risk Models** | GARCH(1,1) + VaR + Kill Switch + Interceptor Chain |
| **Timestamp Precision** | Nanosecond (`UnixNanos`) |
| **Deterministic Replay** | `DeterministicClock` + `SequenceId` ordering |
| **Market Sources** | Alpaca, Binance, Finnhub, Polymarket, Mock |
| **Execution** | Alpaca REST, Polymarket CLOB, Paper Trading |
| **License** | MIT |

---

## Architecture

34 modular crates, 110+ source files, strict dependency boundaries.

```
common           Nanosecond timestamps, events, config, models
ingestion        Multi-source market data (Alpaca, Binance, Finnhub, Polymarket)
execution        Trait-based ExecutionGateway, AlpacaExecutor
strategy         Strategy trait, momentum, mean-reversion engines
risk             Kill switch, GARCH vol, VaR, risk interceptor chain
pricing          Black-Scholes-Merton, Heston, GARCH(1,1) models
backtest         Walk-forward, Monte Carlo, backtesting engine
ai               Dexter AI analyst, Claude integration, signal routing
swarm_sim        100,000-agent market microstructure simulator
knowledge_graph  petgraph-backed RAG knowledge engine
polymarket       Polymarket CLOB client + EIP-712 signing
daemon           Hybrid intelligence pipeline, engine orchestration
event_bus        Postcard-serialized TCP event bus (daemon ↔ TUI)
tui              Ratatui-powered 6-screen trading dashboard
oms              Order Management System (netting + hedging)
alerts           Rule-based alert engine
signals          Technical indicator signal generation
compliance       Pre-trade compliance, audit trail
persistence      PostgreSQL + SQLite persistence layer
metrics          Prometheus-compatible telemetry
ml               Machine learning model inference
model            Model registry and versioning
feature          Feature engineering pipeline
fix              FIX protocol adapter (stub)
cli              Command-line interface
web              REST API server
web-dashboard    Web-based dashboard
dashboard        Dashboard data models
tests            Integration test suite
benchmarks       Criterion performance benchmarks
```

---

## What's Included

### Core Engine
- **Hybrid Intelligence Pipeline** — Quant, Swarm, Knowledge Graph, Dexter AI, Risk Gate, Execution
- **Nanosecond-precision timestamps** (`UnixNanos`) with monotonic `SequenceId` ordering
- **Swappable clock** — `RealtimeClock` for live, `DeterministicClock` for backtesting
- **Event-driven architecture** with typed `Envelope<T>` wrapping every event
- **Deterministic Safety Gate** — Zero-AI verification layer preventing agent confirmation bias
- **30-crate workspace** compiles in ~17s with zero `unsafe` code

### Market Data
- **Alpaca** — Real-time US equities via WebSocket (5 feeds: IEX, SIP, BOATS, Delayed, Overnight)
- **Binance** — Crypto WebSocket streams (trades, bookTicker, depth5)
- **Finnhub** — Global market data (incl. NSE/BSE) and live trades via WebSocket
- **Polymarket** — Prediction market CLOB (order book, trades, EIP-712 signing)
- **Mock source** — Deterministic replay for backtesting
- **Auto-reconnect** with exponential backoff on all sources
- **Source Multiplexer** — Unified `SelectAll` stream from any combination

### AI Intelligence
- **Dexter AI Analyst** — Claude-powered market analysis with structured signal output
- **100K Agent Swarm Simulation** — Rayon-parallel microstructure Monte Carlo
- **Knowledge Graph** — petgraph RAG with entity linking and context fusion
- **Fused Context** — Quant + Swarm + Graph consensus into Dexter prompt
- **Impact Analysis Engine** — AI-driven market impact estimation
- **Mirofish** — 5,000-agent scenario simulator (rally/sideways/dip probabilities)

### Execution
- **ExecutionGateway trait** — Plug-and-play execution backends
- **Alpaca Executor** — Full REST integration (25+ endpoints: orders, positions, assets, historical data)
- **Polymarket CLOB** — Full order lifecycle (limit/market/FOK/GTC/GTD)
- **Paper trading** — MockExecutor for risk-free strategy testing
- **Bracket orders** — OCO/OTO stop-loss + take-profit combos
- **Trailing stops** — Dynamic stop-loss that follows price

### Risk Management
- **Deterministic Safety Gate** — Zero-AI verification layer that detects agent confirmation bias (>85% agreement), checks concentration, drawdown, and correlation exposure
- **Kill Switch** — Emergency circuit breaker (hotkey `K` in TUI)
- **GARCH(1,1) Volatility** — Real-time volatility estimation
- **Value at Risk (VaR)** — Parametric + historical VaR calculation
- **PnL Attribution** — Component-level profit/loss decomposition
- **Risk Interceptor Chain** — Composable pre-trade risk checks
- **Kelly Criterion Sizing** — Optimal position sizing
- **Max Drawdown / Daily Loss Limit** — Automated trading guardrails

### Quantitative Models
- **Black-Scholes-Merton** — Options pricing with Greeks
- **Heston Stochastic Volatility** — Smile-calibrated pricing
- **GARCH(1,1)** — Volatility forecasting
- **Monte Carlo Engine** — Path simulation for derivative pricing
- **Walk-Forward Backtesting** — Out-of-sample validation
- **Latency Queue** — Priority-queue latency simulation

### TUI Trading Dashboard
- **6-screen navigation** — Dashboard, Charts, Orderbook, Positions, AI, Settings
- **Real-time sparkline charts** with zoom, scroll, and time range cycling
- **Live order book visualization** — L2 depth with cumulative volume
- **13-symbol watchlist** — Auto-updating from market data feed
- **Exchange heartbeat monitor** — NYSE, NASDAQ, CME, CBOE, LSE, CRYPTO, NSE, BSE
- **Dexter AI panel** — Live analysis output with BUY/SELL/HOLD recommendation
- **Mirofish simulation widget** — Rally/Sideways/Dip probability bars
- **Trading dialogs** — Buy/Sell order entry with qty/price inputs
- **Emergency controls** — Kill switch (`K`), paper/live toggle (`M`), risk adjust (`+`/`-`)

### Compliance and Audit
- **Full audit trail** — Every state transition logged with `AuditTick`
- **Pre-trade compliance** — Rule-based order validation
- **Deterministic replay** — Reproduce any historical trading session

### News Feed Sources
- **Finnhub News API** — General market news, company-specific news, sector news
- **Alpaca News API** — US equities breaking news, earnings, SEC filings
- **NewsAPI.org** — Aggregates Reuters, Bloomberg, CNBC, WSJ, Financial Times, BBC Business
- **Polygon.io** — SEC filings, earnings reports, company reference data
- **BSE/NSE RSS** — Indian market news from Bombay and National Stock Exchanges
- **CoinGecko** — Cryptocurrency market news and sentiment
- **SEC EDGAR** — Real-time regulatory filings (10-K, 10-Q, 8-K)

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

## Getting Started

```bash
# Clone the repository
git clone https://github.com/Ashutosh0x/rust-finance.git
cd rust-finance

# Set up environment variables
cp .env.example .env
# Edit .env with your API keys:
#   ALPACA_API_KEY, ALPACA_API_SECRET       (market data + execution)
#   FINNHUB_API_KEY                         (market data + news)
#   ANTHROPIC_API_KEY                       (Dexter AI analyst)
#   NEWSAPI_KEY                             (aggregated news feed)
#   POLYGON_API_KEY                         (options chains + news)
#   POLYMARKET_PRIVATE_KEY                  (prediction markets)
#   POLYMARKET_FUNDER_ADDRESS               (prediction markets)
#   TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID   (alert notifications)
#   DISCORD_WEBHOOK_URL                     (alert notifications)

# Build all crates
cargo build --release

# Launch the TUI trading terminal
cargo run --release --bin tui

# Or launch headless daemon
cargo run --release --bin daemon

# Run diagnostics
cargo check --workspace
```

---

## Key Hotkeys (TUI)

| Hotkey | Action |
|:---|:---|
| `Tab` / `Shift+Tab` | Cycle between panels |
| `B` | Open BUY dialog |
| `S` | Open SELL dialog |
| `Enter` | Confirm order |
| `Esc` | Dismiss dialog |
| `K` | KILL SWITCH — Emergency halt all trading |
| `M` | Toggle paper/live mode |
| `+` / `-` | Adjust risk threshold |
| `D` | Trigger Dexter AI analysis |
| `F` | Run Mirofish simulation |
| `Z` / `X` | Chart zoom in/out |
| `←` / `→` | Chart scroll |
| `T` | Cycle chart time range |
| `E` | Export data to CSV |
| `R` | Refresh portfolio |
| `?` | Toggle help overlay |
| `Q` | Quit |

---

## Performance

| Subsystem | Performance |
|:---|:---|
| Timestamp precision | Nanosecond (`UnixNanos`) |
| Event ordering | Lock-free `AtomicU64` sequence generator |
| Market data latency | Sub-millisecond (WebSocket) |
| Event bus serialization | Postcard (zero-copy binary) |
| Swarm simulation | 100K agents via Rayon parallel |
| Release binary | `opt-level=3`, `lto=fat`, `codegen-units=1` |
| Workspace build | ~17s (30 crates, incremental) |

---

## Security

- **Risk interceptor chain** — Every order passes through composable risk checks
- **Kill switch** — Instant emergency halt with hotkey or programmatic trigger
- **Paper/live mode isolation** — Separate execution paths prevent accidental live trades
- **Environment secret management** — `.env` + `dotenvy` with key validation
- **Audit trail** — Full deterministic replay of every state transition
- **Pre-trade compliance** — Rule-based order validation before execution
- **Zeroize** — Sensitive key material zeroized on drop

Report vulnerabilities via [SECURITY.md](SECURITY.md).

---

## Tech Stack

![Rust](https://img.shields.io/badge/Rust-000000?style=flat-square&logo=rust&logoColor=white)
![Tokio](https://img.shields.io/badge/Tokio-000000?style=flat-square)
![Ratatui](https://img.shields.io/badge/Ratatui-000000?style=flat-square)
![Serde](https://img.shields.io/badge/Serde-000000?style=flat-square)
![Ethers](https://img.shields.io/badge/Ethers--rs-3C3C3D?style=flat-square&logo=ethereum&logoColor=white)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-4169E1?style=flat-square&logo=postgresql&logoColor=white)
![Redis](https://img.shields.io/badge/Redis-DC382D?style=flat-square&logo=redis&logoColor=white)
![Petgraph](https://img.shields.io/badge/Petgraph-000000?style=flat-square)

---

**Built with Rust. Nanosecond precision. Institutional grade. Production ready.**

Star this repository if RustForge helps you trade smarter.
