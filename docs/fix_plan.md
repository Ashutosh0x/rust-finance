# Critical Fix Plan: Production Engineering Audit

This plan addresses the critical gaps identified in the architecture that would cause production failures. These items must be resolved before any new algorithmic developments.

## Priority Fix Order

### 1. Safety Layer (Critical Legal & Risk Requirements)
- [ ] **1. Kill Switch**: Implement `crates/daemon/kill_switch.rs` to instantly cancel open orders and halt the bot, complying with mandatory regulatory frameworks (e.g. SEBI 2026).
- [ ] **2. Daily Loss Limit**: Implement `crates/risk/daily_loss_limit.rs` and `drawdown_monitor.rs` to stop runway strategies from blowing up the account.
- [ ] **3. Paper Trading / Dry Run**: Implement True Paper Trading Mode in `crates/daemon/executor/dry_run.rs` so that `USE_MOCK=1` bypasses the actual Solana mainnet execution step.

### 2. Connectivity Resiliency
- [ ] **4. Alpaca Reconnect**: Implement WebSocket reconnect logic for the second data source in `crates/ingestion/alpaca/reconnect.rs`.
- [ ] **5. EventBus Subscriber Reconnect**: Add reconnection and heartbeat ping/pong to the TUI and Web Dashboards via `crates/event_bus/subscriber.rs` and `health.rs`.

### 3. Validation & Persistence
- [ ] **6. Backtest Engine & Metrics**: Build the historical validation framework in `crates/backtest/` including `engine.rs`, `metrics.rs`, `walk_forward.rs`, and `monte_carlo.rs`.
- [ ] **7. Persistence SQL Migrations**: Wire up the actual PostgreSQL/SQLite schemas and export repositories in `crates/persistence/` so Pnl and historical trades are actually recorded to disk.

### 4. Elite Algorithm Upgrades
- [ ] **8. Kalman Filter**: Replace static OLS in pairs trading with dynamic Kalman filtering.
- [ ] **9. GARCH Volatility Model**: Implement GARCH(1,1) for correct position sizing.
- [ ] **10. Almgren-Chriss Execution**: Add optimal trade trajectory market impact predictions.
- [ ] **11. ETF NAV Arbitrage**: Implement core structure for NAV arbitrage.
- [ ] **12. LLM -> RL Hybrid**: Feed Anthropic Claude text sentiments into the PPO RL state space.

### 5. Documentation Tasks
- [ ] **13. Write Missing Architecture Docs**: Complete `docs/architecture.md`, `docs/execution_flow.md`, and `docs/risk_management.md`.
