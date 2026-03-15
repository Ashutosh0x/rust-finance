# RustForge System Architecture

RustForge Terminal is a low-latency, modular trading architecture built heavily upon asynchronous message passing channels and lock-free concurrency.

## Core Philosophical Tenets
1. **Zero-Overhead Parsing:** Data ingestion must never block. `ingestion` yields streams into the EventBus.
2. **Actor-like Concurrency:** The `daemon` routes state exclusively through channels (`mpsc`, `broadcast`).
3. **Poka-yoke Safety:** The `KillSwitch` and `RiskManager` exist physically independent of the execution loop so runaway latency cannot block risk cutoff.
4. **Resiliency over Uptime:** Alpaca WebSockets and TCP EventBus connections feature exponential backoffs (`tokio-retry`) to outlast network drops.

## Component Flow
- **Component Flow**: Ingestion -> AI & Strategy -> OMS & SEBI Limits -> Risk KillSwitch -> Execution/Relay.
- **Relay Router:** Automatically routes transactions via the lowest-latency available JSON-RPC node using Exponential Moving Average (EMA) benchmarking between Helius, Triton, and QuickNode.
- **Strategy & AI (Opus 4.6 / Sonnet 4.6 Routing):** DL models act on the normalized stream. PPO Agents and Statistical Arbitrage evaluate. The daemon implements a strictly gated probability router in `ai_pipeline.rs` ensuring high-stakes calls utilize **Claude Opus 4.6**.
  - Includes **Mirofish Swarm** for 5,000-agent deterministic probability analysis.
  - Includes server-side **Compaction API** blocks to handle multi-week context retention natively.
- **OMS & Compliance Layer:** The Order Management System natively tracks position flipping, unrealised PNL, VWAP, and maintains the order lifecycle. The Order Blotter routes into the `sebi.rs` compliance engine (intraday MIS halts, ±20% bands, max exposures).
- **Risk Layer:** Advanced Kill Switch evaluating Historical 95% VaR, Maximum Drawdown Halts, and GARCH(1,1) Volatility Surges atomically prior to order submission.
- **Observability Stack:** OpenTelemetry (OTLP) tracing across all components with strict internal Prometheus metrics tracked on a dynamic 16-panel Grafana Dashboard.
- **Tiered Persistence:** Live trading state is persisted instantly into **DragonflyDB** (Redis). An **Async Worker Queue** continuously flushes the historically simulated tracks and trades into **PostgreSQL 16** and **TimescaleDB** Hypertables.

### Execution Path Latency Specifications

| System Layer | Technology | Implementation Crate | Target Latency |
| :--- | :--- | :--- | :--- |
| **In-Process State** | Rust Lock-Free Concurrency | `crates/common` | `~50 ns` |
| **Shared Hot-State** | DragonflyDB via `redis` | `crates/persistence/dragonfly.rs` | `~0.2 - 0.5 ms` |
| **Historical Storage**| PostgreSQL + TimescaleDB | `crates/persistence/db.rs` | `~2 - 5 ms` |

The entire routing path (`Tick Analysis` → `AI Veto Gate` → `Order Fill`) evaluates natively below `1 ms` internally before Solana RPC propagation.

## Quantitative Analytics & Pricing (Phase 5)
RustForge natively integrates Bloomberg-tier financial engineering formulas directly into the `pricing` and `risk` crates, built to execute in microseconds for live terminal display.

### 1. Options Pricing (Black-Scholes-Merton & Heston)
The system incorporates classical BSM with a closed-form Newton-Raphson IV solver, alongside the **Heston Stochastic Volatility Model** which uses the Gil-Pelaez characteristic function inversion to capture the volatility smile/skew that BSM fails to model:

**Heston Dynamics:**
- `dS = μ·S·dt + √v·S·dW₁`
- `dv = κ·(θ - v)·dt + σ_v·√v·dW₂`
- `corr(dW₁, dW₂) = ρ·dt`

### 2. Fixed Income (BVAL & Hull-White)
To capture interest rate term structures and price bond derivatives, the system implements the **Hull-White One-Factor** model wrapped around a Trinomial Tree algorithm to accurately compute American-style early exercise premiums. 

Additionally, the system replicates the **Bloomberg BVAL 3-Step Process** for bond pricing:
1. **Direct Observations** (Weighted most heavily)
2. **Historical Correlations** (Yield curve shifts)
3. **Comparable Relative Value (RV)** matrices

### 3. Risk & Volatility Forecasting (GARCH)
Rather than solely relying on historical standard deviations (which lag market shocks), the internal Risk Manager utilizes a rolling **GARCH(1,1)** Maximum Likelihood Estimation engine to forecast variance.

**GARCH(1,1) Conditional Variance formulation:**
`σ²_t = ω + α·ε²_{t-1} + β·σ²_{t-1}`
- *Where α + β < 1 guarantees mean-reversion stationarity.*

### 4. Machine Learning (NeurIPS Interval Regression)
Classical ML target-mapping breaks down when lit prints are sparse (like in corporate bonds). We incorporated Bloomberg's NeurIPS 2025 finding on **Interval Regression** (`crates/ml/interval_regression.rs`). This custom Neural Network loss function trains *only* on Bid/Ask bounds rather than forcing a naive mid-price assumption.

**Modified Interval Loss Gradient:**
- `If Prediction < Bid`: `Loss = (Bid - Prediction)²`
- `If Prediction > Ask`: `Loss = (Prediction - Ask)²`
- `Else (Inside Spread)`: `Loss = 0`
