# RustFinance Terminal (rust-finance)

<div align="center">
  <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/Tokio-1A2421?style=for-the-badge&logo=Rust&logoColor=white" alt="Tokio" />
  <img src="https://img.shields.io/badge/Ratatui-0A0C0F?style=for-the-badge&logo=Linux&logoColor=white" alt="Ratatui" />
  <img src="https://img.shields.io/badge/Anthropic-FF7F50?style=for-the-badge&logo=Anthropic&logoColor=white" alt="Anthropic" />
  <img src="https://img.shields.io/badge/WebSocket-010101?style=for-the-badge&logo=socket.io&logoColor=white" alt="WebSocket" />
  <br />
  <a href="https://github.com/Ashutosh0x/rust-finance/stargazers"><img src="https://img.shields.io/github/stars/Ashutosh0x/rust-finance?style=for-the-badge&logo=github&color=gold" alt="GitHub stars" /></a>
  <a href="https://github.com/Ashutosh0x/rust-finance/network/members"><img src="https://img.shields.io/github/forks/Ashutosh0x/rust-finance?style=for-the-badge&logo=github&color=silver" alt="GitHub forks" /></a>
  <br />
  <img src="https://img.shields.io/badge/Alpaca-FACC15?style=for-the-badge&logo=alpaca&logoColor=black" alt="Alpaca" />
  <img src="https://img.shields.io/badge/Binance-FCD535?style=for-the-badge&logo=binance&logoColor=black" alt="Binance" />
  <img src="https://img.shields.io/badge/Finnhub-000000?style=for-the-badge&logo=graphql&logoColor=white" alt="Finnhub" />
  <img src="https://img.shields.io/badge/Polymarket-000000?style=for-the-badge&logo=Polymarket&logoColor=white" alt="Polymarket" />
  <br />
  <img src="https://img.shields.io/badge/NASDAQ-0090F7?style=for-the-badge&logo=nasdaq&logoColor=white" alt="NASDAQ" />
  <img src="https://img.shields.io/badge/NYSE-092140?style=for-the-badge&logo=new-york-stock-exchange&logoColor=white" alt="NYSE" />
  <img src="https://img.shields.io/badge/CME-003366?style=for-the-badge" alt="CME" />
  <img src="https://img.shields.io/badge/LSE-000000?style=for-the-badge" alt="LSE" />
  <img src="https://img.shields.io/badge/NSE-004481?style=for-the-badge" alt="NSE" />
  <img src="https://img.shields.io/badge/BSE-004C8F?style=for-the-badge" alt="BSE" />
  <img src="https://img.shields.io/badge/CRYPTO-F7931A?style=for-the-badge" alt="CRYPTO" />
  <br />
  <img src="https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white" alt="Windows" />
  <img src="https://img.shields.io/badge/macOS-000000?style=for-the-badge&logo=apple&logoColor=white" alt="macOS" />
  <img src="https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black" alt="Linux" />
  <br />
  <a href="https://scorecard.dev/viewer/?uri=github.com/Ashutosh0x/rust-finance"><img src="https://img.shields.io/ossf-scorecard/github.com/Ashutosh0x/rust-finance?style=for-the-badge&label=OpenSSF%20Scorecard" alt="OpenSSF Scorecard" /></a>
  <a href="https://github.com/Ashutosh0x/rust-finance/actions/workflows/security.yml"><img src="https://img.shields.io/github/actions/workflow/status/Ashutosh0x/rust-finance/security.yml?style=for-the-badge&logo=githubactions&logoColor=white&label=Security%20%26%20Supply%20Chain" alt="Security & Supply Chain" /></a>
  <a href="https://github.com/Ashutosh0x/rust-finance/actions/workflows/test.yml"><img src="https://img.shields.io/github/actions/workflow/status/Ashutosh0x/rust-finance/test.yml?style=for-the-badge&logo=githubactions&logoColor=white&label=CI%20Tests" alt="CI Tests" /></a>
  <a href="https://deps.rs/repo/github/Ashutosh0x/rust-finance"><img src="https://img.shields.io/badge/dependencies-up%20to%20date-brightgreen?style=for-the-badge" alt="dependency status" /></a>
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-yellow?style=for-the-badge" alt="License: MIT" /></a>
</div>

---

## Overview
<img width="1184" height="941" alt="image" src="https://github.com/user-attachments/assets/f4257b02-cca4-4349-904c-810256ffe5a3" />



https://github.com/user-attachments/assets/c769b2c2-cfa0-44bd-a261-99786ea653e1


RustForge is an institutional-grade AI trading terminal built in pure Rust. It combines real-time multi-exchange market data, Claude-powered AI analysis, quantitative risk management, prediction market trading, and a full TUI dashboard — all in a single binary with nanosecond-precision timestamps and sub-millisecond latency.

> **v0.4** — 2026 Quant Alpha Library: Multi-level Microprice (MLOFI), Adverse Selection Detection, Regime-Conditioned Quoting, Kelly+Bayesian Sizing, Alpha Health Monitor, IC-Weighted Composite Signals, Enhanced Avellaneda-Stoikov Quoting Engine, Almgren-Chriss Optimal Execution, Live Binance WebSocket Candlestick Feed, and WebSocket Gap Detection Engine. 82+ unit tests passing on Rust 1.95.0.

| Feature | Detail |
|:---|:---|
| Language | Pure Rust |
| Interface | Full TUI Dashboard (Ratatui, 6 screens) |
| AI Integration | Claude-powered Dexter Analyst |
| Execution Algorithms | TWAP, VWAP, Iceberg, POV, Almgren-Chriss Optimal Execution |
| Market Making | Enhanced Avellaneda-Stoikov with regime gating, VPIN toxicity, OFI skew |
| Microstructure | Multi-level MLOFI, OFI, Microprice, Kyle's Lambda, VPIN, Amihud, Lee-Ready |
| Smart Order Router | Multi-venue scoring (fill rate, latency, fees, impact) |
| Alpha Signals | Toxicity detection, regime classifier, IC-weighted composite, alpha health |
| Position Sizing | Quarter-Kelly with Bayesian shrinkage, conviction scaling |
| Live Data | Binance WebSocket candlestick feed with gap detection engine |
| Prediction Markets | Polymarket CLOB + cross-platform arbitrage engine |
| Agent Simulation | 100K-agent Rayon-parallel swarm |
| Knowledge Graph | petgraph-backed RAG engine |
| Risk Models | GARCH(1,1) + VaR + Kill Switch + SMP + Regime Detection + Toxicity Gating |
| Timestamp Precision | Nanosecond (`UnixNanos`) |
| Deterministic Replay | `DeterministicClock` + `SequenceId` ordering |
| Regulatory Compliance | SEBI 2026 Algo-ID + OPS threshold + pre-trade checks |
| Fill Simulation | Almgren-Chriss √-impact model + fixed slippage |
| Alpha Monitoring | Rolling IC, Sharpe, hit rate with auto-decay detection |
| FIX Protocol | Production FIX 4.4 parser with checksum validation |
| Market Sources | Alpaca, Binance, Finnhub, Polymarket, Mock |
| Execution | Alpaca REST, Polymarket CLOB, Paper Trading |
| License | MIT |

![Rust Trading Terminal](assets/rust_terminal.png)

---

## Table of Contents
- [Overview](#overview)
- [Architecture](#architecture)
- [Quick Start](#quick-start)
- [Features](#features)
- [TUI Hotkeys](#tui-hotkeys)
- [Performance](#performance)
- [Configuration](#configuration)
- [Strategy Development](#strategy-development)
- [API Reference](#api-reference)
- [Troubleshooting](#troubleshooting)
- [License and Disclaimer](#license-and-disclaimer)

---

## Architecture

34 modular crates, 250+ source files, strict dependency boundaries.

```mermaid
graph TD;
    subgraph "External Feeds"
        ALP(Alpaca WS) --> Ingest
        BIN(Binance WS) --> Ingest
        FH(Finnhub WS) --> Ingest
        PM(Polymarket WS) --> Ingest
        LLM(Anthropic Claude) <--> AI
    end

    subgraph "RustForge Engine"
        Ingest(Ingestion / Source Multiplexer) --> Bus(TCP Event Bus)
        Bus --> AI(AI Engine — Dexter / Mirofish)
        Bus --> Quant(Quant Features)
        Quant --> Swarm(Swarm Simulator — 100K Agents)
        Swarm --> KG(Knowledge Graph — petgraph RAG)
        KG --> AI

        AI --> Strategy(Strategy Dispatcher)
        Strategy --> Risk(Risk Gate — GARCH / VaR / Kill Switch)
        Risk --> Exec(Execution Gateway)

        Exec -.-> |Paper Mode| Mock(MockExecutor)
        Exec --> |Live Mode| AlpacaAPI(Alpaca REST)
        Exec --> |Prediction| PolyCLOB(Polymarket CLOB)
    end

    subgraph "Quantitative Models"
        Pricing(Pricing Engine) --> BSM(Black-Scholes-Merton)
        Pricing --> HESTON(Heston Stochastic Vol)
        Risk --> GARCH(GARCH 1,1 Volatility)
        Backtest(Backtest Engine) --> WF(Walk-Forward)
        Backtest --> MC(Monte Carlo)
    end

    subgraph "Persistence"
        Bus --> PG[(PostgreSQL)]
        Bus --> Redis[(Redis Hot-State)]
    end

    subgraph "Frontends"
        Bus --> TUI(Ratatui TUI Dashboard)
        Bus --> Web(REST API / Web Dashboard)
    end
```

### Crate Map

```
common           Nanosecond timestamps, events, config, models
ingestion        Multi-source market data (Alpaca, Binance, Finnhub, Polymarket) + gap detector
execution        ExecutionGateway + TWAP/VWAP/Iceberg/POV + Almgren-Chriss optimal execution
strategy         Momentum, MeanReversion, Avellaneda-Stoikov (Welford O(1) variance)
risk             Kill switch, GARCH vol, VaR, risk interceptor chain, self-match prevention
pricing          Black-Scholes-Merton, Heston, GARCH(1,1) models
backtest         Walk-forward, Monte Carlo, backtesting engine, √-impact fill model
ai               Dexter AI analyst, Claude integration, signal routing
swarm_sim        100,000-agent market microstructure simulator
knowledge_graph  petgraph-backed RAG knowledge engine
polymarket       CLOB + EIP-712 signing + sum-to-one/cross-platform arb engine
daemon           Hybrid intelligence pipeline, engine orchestration
event_bus        Postcard-serialized TCP event bus (daemon <-> TUI)
tui              Ratatui TUI + live Binance candlestick feed + 500 msg/frame cap
oms              Order Management System (netting + hedging + SEBI 2026 Algo-ID)
alerts           Rule-based alert engine
signals          2026 Quant Alpha Library (see below) + indicators + microstructure
compliance       Pre-trade compliance, audit trail
persistence      PostgreSQL + SQLite persistence layer
metrics          Prometheus-compatible telemetry
ml               Machine learning model inference, alpha decay monitoring
model            Model registry and versioning
feature          Feature engineering pipeline
fix              FIX 4.4 protocol engine — production parser + session layer
cli              Command-line interface
web              REST API server
web-dashboard    Web-based dashboard
dashboard        Dashboard data models
tests            Integration test suite
benchmarks       Criterion performance benchmarks
```

#### Signals Crate — 2026 Quant Alpha Library

Based on 18 research papers (2025–2026). 55 unit tests.

```
Indicators       SMA, EMA, RSI, MACD, Bollinger Bands, VWAP
Microstructure   OFI (Cont 2014), Microprice, Kyle's Lambda, VPIN (Easley 2012),
                 Amihud Illiquidity, Lee-Ready Trade Classifier
Microprice ML    Multi-level microprice MLOFI (Oxford 2019, arXiv 2602.00776)
Adverse Select.  Toxicity detection + halt/widen (Barzykin 2025, Crypto 2026)
Regime           Fast/slow EMA vol ratio → LowVol/Normal/HighVol/Crisis (RegimeFolio 2025)
Kelly            Quarter-Kelly + Bayesian shrinkage sizing (PolySwarm Apr 2026)
Alpha Health     IC decay + hit rate monitor → Healthy/Degraded/Decayed (AlphaForgeBench 2026)
Composite        IC-weighted signal combiner with regime + toxicity gating (Chain-of-Alpha 2025)
Quoting Engine   Enhanced Avellaneda-Stoikov: regime γ, VPIN widening, OFI skew, Kelly sizing
```

#### Execution Crate — Almgren-Chriss Optimal Execution

```
Almgren-Chriss   Optimal trajectory: xⱼ = X × sinh(κ(N-j)) / sinh(κN)
                 Urgency parameter κ = √(λσ²/η), square-root impact model
                 Presets: liquid_default, aggressive, passive
                 7 unit tests (TWAP convergence, front-loading, quantity conservation)
```

---

## Quick Start

```bash
# 1. Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Clone
git clone https://github.com/Ashutosh0x/rust-finance.git
cd rust-finance

# 3. Configure
cp .env.example .env
# Edit .env — add your API keys (see Configuration below)

# 4. Build
cargo build --release

# 5. Run (mock mode — no API keys required)
USE_MOCK=1 cargo run -p daemon --release

# 6. Run TUI (separate terminal)
cargo run -p tui --release
```

---

## Features

### Core Engine
- Hybrid Intelligence Pipeline — Quant, Swarm, Knowledge Graph, Dexter AI, Risk Gate, Execution
- Nanosecond-precision timestamps (`UnixNanos`) with monotonic `SequenceId` ordering
- Swappable clock — `RealtimeClock` for live trading, `DeterministicClock` for backtesting
- Event-driven architecture with typed `Envelope<T>` wrapping every system event
- Deterministic Safety Gate — zero-AI verification layer preventing agent confirmation bias
- 34-crate workspace compiling in ~17s

### Market Data
- **Alpaca** — Real-time US equities via WebSocket (5 feeds: IEX, SIP, BOATS, Delayed, Overnight)
- **Binance** — Crypto streams (trades, bookTicker, kline, depth5) via combined WS endpoint
  - **Live Candlestick Feed** — Direct WebSocket kline_1m stream → real-time OHLCV candlestick rendering in TUI
  - **Gap Detection Engine** — Sequence-based WebSocket gap detector with configurable thresholds (small gap: retransmit, medium: snapshot, large: failover)
- **Finnhub** — Global market data (incl. NSE/BSE) and live trades via WebSocket
- **Polymarket** — Prediction market data via 3 APIs:
  - **Gamma API** — Events, markets, tags, search, profiles (`gamma-api.polymarket.com`)
  - **CLOB API** — Orderbook, midpoint, spread, prices-history, tick-size, fee-rate (`clob.polymarket.com`)
  - **Data API** — Positions, trades, leaderboards, open interest (`data-api.polymarket.com`)
- **Mock source** — Deterministic replay for backtesting
- Auto-reconnect with exponential backoff on all sources
- Source Multiplexer — unified `SelectAll` stream from any combination of sources
- **500 msg/frame cap** — Prevents TUI freeze during high-volume market activity (e.g., BTC flash crashes)

### AI Intelligence
- **Dexter AI Analyst** — Claude-powered market analysis with structured signal output
- **100K Agent Swarm Simulation** — Rayon-parallel microstructure Monte Carlo
- **Knowledge Graph** — petgraph RAG with entity linking and context fusion
- **Fused Context** — Quant + Swarm + Graph consensus fed into Dexter prompt
- **Impact Analysis Engine** — AI-driven market impact estimation
- **Mirofish** — 5,000-agent scenario simulator (rally/sideways/dip probabilities)

### Execution & Algorithmic Trading
- `ExecutionGateway` trait — plug-and-play execution backends
- **Almgren-Chriss Optimal Execution** — closed-form trajectory minimizing E[cost] + λ×Var[cost], with urgency parameter κ = √(λσ²/η), square-root impact model, and adaptive σ/λ updates from GARCH + regime detector
- **TWAP** — Time-weighted slicing with configurable `horizon_secs` / `interval_secs`
- **VWAP** — Volume-weighted execution with U-shape intraday profile
- **Iceberg** — Hidden liquidity: only `display_qty` visible, auto-replenish on fill
- **POV (Percentage of Volume)** — Adaptive participation tracking real-time market volume
- **Smart Order Router (SOR)** — Multi-venue scoring (fill rate, latency, fees, liquidity, market impact), dark pool preference for large orders, spray routing, MiFID II best execution reporting
- **Alpaca Executor** — Full REST integration (25+ endpoints: orders, positions, assets, historical data)
- **Polymarket CLOB** — full order lifecycle (limit/market/FOK/GTC/GTD), EIP-712 signed orders
- **Polymarket Arbitrage** — Sum-to-one (YES+NO < $1), cross-market, and cross-platform (Polymarket vs Kalshi) spread detection with fee-aware P&L
- **Paper trading** — MockExecutor for risk-free strategy testing
- **Bracket orders** — OCO/OTO stop-loss + take-profit combos
- **Trailing stops** — dynamic stop-loss that follows price

### Risk Management
- **Deterministic Safety Gate** — zero-AI verification layer detecting agent confirmation bias (>85% agreement), concentration, drawdown, and correlation exposure
- **Self-Match Prevention (SMP)** — wash trade prevention for market makers with 3 exchange-standard modes: `CancelResting` (CME), `CancelAggressive` (NASDAQ), `CancelBoth` (safest)
- **Kill Switch** — emergency circuit breaker (hotkey `K` in TUI)
- **GARCH(1,1) + EGARCH** — real-time symmetric and asymmetric volatility estimation
- **Value at Risk (VaR)** — Historical, Parametric (Delta-Normal), and Student-t fat-tail VaR at 95%/99% confidence with CVaR (Expected Shortfall)
- **Component VaR** — per-position marginal contribution to portfolio risk
- **10-Day Basel VaR** — √10 scaling for regulatory compliance
- **PnL Attribution** — component-level profit/loss decomposition
- **Risk Interceptor Chain** — composable pre-trade risk checks (MaxPositionSize, MaxDrawdown, MaxOpenOrders, DailyLossLimit, SMP)
- **Kelly Criterion Sizing** — quarter-Kelly position sizing with conviction scaling
- **Cross-Asset Correlation** — rolling pairwise correlation and concentration penalty
- **Regime Detection** — GARCH-based volatility regime classification with position scaling
- **Alpha Decay Monitor** — rolling Information Coefficient (Spearman IC), Sharpe ratio, and hit rate tracking; auto-flags `Healthy` → `Degraded` → `Decayed` health states for strategy auto-pause
- Max Drawdown and Daily Loss Limit trading guardrails
- **SEBI 2026 Compliance** — Algo-ID tagging (mandatory since April 1, 2026), OPS threshold monitoring, order variety classification, price band checks, uptick rule, squareoff time enforcement

### Market Making & Microstructure (2026 Enhanced)
- **Enhanced Avellaneda-Stoikov Quoting Engine** — Reservation pricing `r = FV - q·γ·σ²·τ` with:
  - **Regime-conditioned γ** — auto-adjusts risk aversion per volatility regime (RegimeFolio 2025)
  - **VPIN spread widening** — widens quotes when informed trading probability exceeds 0.5 (Easley 2012)
  - **Toxicity halt** — stops passive quoting at toxicity > 0.65 (Barzykin 2025)
  - **OFI skew** — shifts quotes by λ_ofi × OFI for order flow anticipation (Cont 2014)
  - **Kelly-scaled sizing** — position size = quarter-Kelly × conviction × regime_mult × (1 - toxicity)
- **Adverse Selection Detector** — EMA-smoothed post-fill price move analysis; classifies fills as toxic/safe and recommends Normal/Widen/HaltPassive actions
- **Multi-Level Microprice (MLOFI)** — Oxford 2019 depth-decayed fair value across 5+ LOB levels; ~50-tick predictive lead over arithmetic mid
- **Two-State Regime Classifier** — Fast/slow EMA vol ratio → LowVol/Normal/HighVol/Crisis with debounce; each regime maps to concrete γ, spread_mult, size_mult parameters
- **Order Flow Imbalance (OFI)** — Cont et al. (2014) — net bid/ask volume change for short-term direction
- **VPIN Toxicity** — Volume-synchronized Probability of Informed Trading (Easley et al. 2012)
- **Kyle's Lambda** — Price impact coefficient per unit signed order flow (Kyle 1985)
- **Amihud Illiquidity** — `|return| / dollar_volume` for position sizing in thin names
- **Lee-Ready Classifier** — Buyer/seller-initiated trade classification
- **EWMA Volatility** — Real-time annualized vol estimation (RiskMetrics λ=0.94)

### Alpha Signal Pipeline (2026 Quant Library)
- **Alpha Health Monitor** — Tracks IC (information coefficient) and hit rate per signal; classifies Healthy (weight=1.0) / Degraded (0.4) / Decayed (0.0) with automatic disabling of decayed signals
- **Composite Signal Generator** — IC-proportional weighting (Chain-of-Alpha 2025) with regime scaling and toxicity discounting; outputs (score, conviction) for downstream Kelly sizing
- **Kelly Criterion + Bayesian Shrinkage** — Quarter-Kelly (industry standard per PolySwarm Apr 2026) with n/(n+30) Bayesian shrinkage for small sample sizes
- **Bayesian Fair Value** — Weighted combination of microprice, fast EMA, and slow EMA with configurable alpha weights

### Quantitative Models
- **Black-Scholes-Merton** — options pricing with full Greeks (Delta, Gamma, Theta, Vega, Rho)
- **Heston Stochastic Volatility** — smile-calibrated pricing via Monte Carlo
- **SABR Model** — stochastic alpha-beta-rho for FX/rates vol surface
- **Hull-White** — short-rate model for fixed income
- **Bond Pricer** — yield-to-maturity, duration, convexity
- **GARCH(1,1) + EGARCH** — volatility forecasting with leverage effects
- **Fundamental Analysis** — DCF, Graham, and PEG valuation models
- **Monte Carlo Engine** — path simulation for derivative pricing
- **Walk-Forward Backtesting** — out-of-sample validation with Sharpe, Sortino, CAGR, profit factor
- **Pluggable Fill Model** — `FillModel` trait with two implementations:
  - `FixedSlippage` — constant basis-point slippage (backward compat)
  - `SquareRootImpact` — Almgren-Chriss institutional model: `impact = σ × η × √(q/ADV)` with presets for liquid (η=0.1), mid-cap (η=0.25), and illiquid (η=0.5) instruments
- **Transaction Cost Analysis (TCA)** — Implementation Shortfall, VWAP/TWAP slippage, market impact, per-strategy breakdown
- **Gamma Exposure (GEX)** — dealer gamma surface, flip points, pin risk zones, vol regime detection
- **Latency Queue** — priority-queue latency simulation for realistic fills

### TUI Dashboard
- 6-screen navigation — Dashboard, Charts, Orderbook, Positions, AI, Settings
- Real-time sparkline charts with zoom, scroll, and time range cycling
- Live order book visualization — L2 depth with cumulative volume
- 13-symbol watchlist auto-updating from market data feed
- Exchange heartbeat monitor — NYSE, NASDAQ, CME, CBOE, LSE, CRYPTO, NSE, BSE
- Dexter AI panel with live analysis output and BUY/SELL/HOLD recommendation
- Mirofish simulation widget — rally/sideways/dip probability bars
- Buy/Sell order entry dialogs with quantity and price inputs
- Emergency controls — kill switch, paper/live toggle, risk adjustment

### Compliance and Audit
- Full audit trail — every state transition logged with `AuditTick`
- Pre-trade compliance — rule-based order validation
- **SEBI 2026 Algo Framework** — Algo-ID tagging on every order to NSE/BSE, static IP whitelisting, OPS threshold monitoring (>10 OPS requires registration), order value caps, MIS squareoff time enforcement, uptick rule, price band circuit filters
- Deterministic replay — reproduce any historical trading session

### FIX Protocol (v0.3)
- **Production FIX 4.4 Parser** — length-delimited (`BodyLength` tag 9) message framing with checksum validation
- **Tag-Value Extraction** — full field parsing into `HashMap<u32, String>` with `MsgType` derivation from tag 35
- **Streaming Parser** — `push_bytes()` + `next_message()` pattern for TCP stream processing
- **Session Layer** — Logon/Logout/Heartbeat/TestRequest/ResendRequest/SequenceReset handling
- **Supported Messages** — Logon, Logout, Heartbeat, TestRequest, ResendRequest, SequenceReset, ExecutionReport, OrderCancelReject, NewOrderSingle, OrderCancelRequest
- Zero external dependencies — hand-rolled for maximum control and auditability

### Security & Supply Chain (CI/CD)
- **8-job Security pipeline** — cargo-audit (CVE), cargo-deny (licenses/bans/advisories), cargo-vet (supply chain verification), Clippy (pedantic + cargo lints), rustfmt, SHA-pin enforcement, CodeQL SAST, OpenSSF Scorecard
- All GitHub Actions SHA-pinned to full 40-character commit hashes
- Cross-platform CI — tests on Ubuntu, macOS, and Windows
- MSRV verification — Minimum Supported Rust Version enforced
- Benchmark compilation checks — Criterion benchmarks verified on every push

### News Feed Sources
- **Finnhub News API** — general market news, company-specific news, sector news
- **Alpaca News API** — US equities breaking news, earnings, SEC filings
- **NewsAPI.org** — aggregates Reuters, Bloomberg, CNBC, WSJ, Financial Times, BBC Business
- **Polygon.io** — SEC filings, earnings reports, company reference data
- **BSE/NSE RSS** — Indian market news from Bombay and National Stock Exchanges
- **CoinGecko** — cryptocurrency market news and sentiment
- **SEC EDGAR** — real-time regulatory filings (10-K, 10-Q, 8-K)

---

## TUI Hotkeys

| Hotkey | Action |
|:---|:---|
| `Tab` / `Shift+Tab` | Cycle between panels |
| `B` | Open BUY dialog |
| `S` | Open SELL dialog |
| `Enter` | Confirm order |
| `Esc` | Dismiss dialog |
| `K` | KILL SWITCH — emergency halt all trading |
| `M` | Toggle paper/live mode |
| `+` / `-` | Adjust risk threshold |
| `D` | Trigger Dexter AI analysis |
| `F` | Run Mirofish simulation |
| `Z` / `X` | Chart zoom in/out |
| `Left` / `Right` | Chart scroll |
| `T` | Cycle chart time range |
| `E` | Export data to CSV |
| `R` | Refresh portfolio |
| `?` | Toggle help overlay |
| `Q` | Quit |

---

## Performance

| Component | Benchmark | Execution Time |
|:---|:---|:---|
| Tick Pipeline | Order book mutation | ~40 ns |
| Pricing Models | BSM European Call | ~34 ns |
| Risk Constraints | GARCH(1,1) Update | ~2.3 ns |
| Risk Constraints | Branchless Safety Check | ~1.6 ns |
| Event Bus | Postcard serialization | Zero-copy binary |
| Swarm Sim | 100K agents | Rayon parallel |
| Timestamps | `UnixNanos` precision | Nanosecond |
| Event Ordering | `AtomicU64` sequence | Lock-free |

Release profile: `opt-level=3`, `lto=fat`, `codegen-units=1`, `strip=true`

---

## Configuration

### API Keys Required

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

### Setup

1. Copy the example environment file:
   ```bash
   cp .env.example .env
   ```

2. Edit `.env` and add your API keys (see table above).

3. Quick test with no API keys required:
   ```bash
   USE_MOCK=1 cargo run -p daemon --release
   ```

See [docs/SETUP.md](docs/SETUP.md) for step-by-step key creation instructions.
See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for the full configuration reference.

---

## Strategy Development

Strategies are implemented in the `strategy` crate using the `PluggableStrategy` async trait:

1. Define your strategy struct and internal state
2. Implement `on_market_event()` to process live tick data
3. Emit `TradeSignal` objects with desired positions and dynamic confidences
4. Register in the daemon's strategy registry for hot-swapping

For examples, see `AiGatedMomentum` in `crates/daemon/src/strategy_registry.rs`.

---

## API Reference

| Endpoint | Port | Protocol |
|:---|:---|:---|
| Market Data Ingestion | `4310` | WebSocket |
| Event Bus (daemon to TUI) | `7001` | TCP + Postcard |
| Prometheus Metrics | `3000` | HTTP GET `/metrics` |
| Tracing Export (Jaeger) | `4318` | OTLP UDP |

### Alpaca Broker Integration
- `POST /v2/orders` — order submission via `AlpacaBroker::submit_order`, rate-limited to 150 req/min
- `GET /v2/positions` — periodic position reconciliation into TUI

### Polymarket CLOB Integration
- `POST /order` — EIP-712 signed order placement
- `DELETE /order/{id}` — cancel specific order
- `DELETE /cancel-all` — cancel all open orders
- `GET /orders` — list open orders
- `GET /book` — order book snapshot
- `GET /midpoint` — midpoint price
- `GET /balance-allowance` — USDC balance

---

## Troubleshooting

- **Build errors on Solana crates**: The legacy `parser`, `executor`, `signer`, and `relay` crates are excluded from the workspace due to a yanked `solana_rbpf` dependency. They are replaced by `crates/ingestion` and `crates/execution` in the v2 architecture.
- **WebSocket timeout**: Ensure your Finnhub/Alpaca API keys are correct. `reconnect.rs` will log warnings on exponential backoff attempts.
- **Missing API keys**: Run in mock mode with `USE_MOCK=1` to test without any API keys.
- **TUI not connecting**: Start the daemon first (`cargo run -p daemon`), then the TUI (`cargo run -p tui`) in a separate terminal. The TUI connects via TCP to `127.0.0.1:7001`.

---

## License and Disclaimer

> **WARNING**
> This software is provided for educational and research purposes only. The authors are not responsible for any financial losses incurred from running autonomous code on live capital.

MIT License (c) 2026 Ashutosh0x
