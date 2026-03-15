# RustForge System Architecture

RustForge Terminal is a low-latency, modular trading architecture built heavily upon asynchronous message passing channels and lock-free concurrency.

## Core Philosophical Tenets
1. **Zero-Overhead Parsing:** Data ingestion must never block. `ingestion` yields streams into the EventBus.
2. **Actor-like Concurrency:** The `daemon` routes state exclusively through channels (`mpsc`, `broadcast`).
3. **Poka-yoke Safety:** The `KillSwitch` and `RiskManager` exist physically independent of the execution loop so runaway latency cannot block risk cutoff.
4. **Resiliency over Uptime:** Alpaca WebSockets and TCP EventBus connections feature exponential backoffs (`tokio-retry`) to outlast network drops.

## Component Flow
- **Ingestion:** Connects to Finnhub, normalizes ticks, transmits over MPSC to EventBus.
- **Relay/Bus:** A localized TCP server broadcasting `BotEvent`s to the terminal UI (Ratatui) without coupling them to the Daemon process.
- **Strategy & AI:** ML models act on the normalized stream. PPO Agents and Statistical Arbitrage evaluate.
- **Risk Layer:** Daily Loss limits, Drawdown monitors, and Kill Switches filter actions.
- **Persistence:** Appended sequentially to a SQLite `trades` ledger.

## Quantitative Analytics & Pricing (Phase 5)
RustForge natively integrates Bloomberg-tier financial engineering formulas directly into the `pricing` and `risk` crates, built to execute in microseconds for live terminal display.

### 1. Options Pricing (Black-Scholes-Merton & Heston)
The system incorporates classical BSM with a closed-form Newton-Raphson IV solver, alongside the **Heston Stochastic Volatility Model** which uses the Gil-Pelaez characteristic function inversion to capture the volatility smile/skew that BSM fails to model:

**Heston Dynamics:**
- $dS = \mu \cdot S \cdot dt + \sqrt{v} \cdot S \cdot dW_1$
- $dv = \kappa (\theta - v) dt + \sigma_v \sqrt{v} dW_2$
- $corr(dW_1, dW_2) = \rho \cdot dt$

### 2. Fixed Income (BVAL & Hull-White)
To capture interest rate term structures and price bond derivatives, the system implements the **Hull-White One-Factor** model wrapped around a Trinomial Tree algorithm to accurately compute American-style early exercise premiums. 

Additionally, the system replicates the **Bloomberg BVAL 3-Step Process** for bond pricing:
1. **Direct Observations** (Weighted most heavily)
2. **Historical Correlations** (Yield curve shifts)
3. **Comparable Relative Value (RV)** matrices

### 3. Risk & Volatility Forecasting (GARCH)
Rather than solely relying on historical standard deviations (which lag market shocks), the internal Risk Manager utilizes a rolling **GARCH(1,1)** Maximum Likelihood Estimation engine to forecast variance.

**GARCH(1,1) Conditional Variance formulation:**
$\sigma_t^2 = \omega + \alpha \cdot \epsilon_{t-1}^2 + \beta \cdot \sigma_{t-1}^2$
- *Where $\alpha + \beta < 1$ guarantees mean-reversion stationarity.*

### 4. Machine Learning (NeurIPS Interval Regression)
Classical ML target-mapping breaks down when lit prints are sparse (like in corporate bonds). We incorporated Bloomberg's NeurIPS 2025 finding on **Interval Regression** (`crates/ml/interval_regression.rs`). This custom Neural Network loss function trains *only* on Bid/Ask bounds rather than forcing a naive mid-price assumption.

**Modified Interval Loss Gradient:**
- `If Prediction < Bid`: $\text{Loss} = (\text{Bid} - \text{Prediction})^2$
- `If Prediction > Ask`: $\text{Loss} = (\text{Prediction} - \text{Ask})^2$
- `Else (Inside Spread)`: $\text{Loss} = 0$
