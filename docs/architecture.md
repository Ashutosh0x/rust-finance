# RustForge — Architecture Guide

## System Overview

RustForge is a modular trading terminal built as a Cargo workspace of 30+ crates. The system follows an event-driven architecture where all components communicate through typed events flowing via broadcast channels and a TCP event bus.

---

## Core Pipeline

The Hybrid Intelligence Pipeline processes market data through six stages:

1. **Ingestion** — `MarketDataSource` trait implementations connect to exchanges (Alpaca, Binance, Finnhub, Polymarket) and normalize raw feeds into `Envelope<MarketEvent>` events with nanosecond timestamps.

2. **Feature Extraction** — The `feature` crate computes technical indicators and quantitative features from the normalized market data stream.

3. **Intelligence Layer** — Three parallel subsystems analyze the features:
   - **Swarm Simulator** — 100K autonomous agents model market microstructure via Rayon
   - **Knowledge Graph** — petgraph-backed RAG links entities, catalysts, and context
   - **Dexter AI** — Claude-powered analyst receives fused context and produces structured signals

4. **Strategy** — `PluggableStrategy` trait implementations consume signals and emit `TradeSignal` objects with direction, size, and confidence.

5. **Risk Gate** — Composable `RiskInterceptor` chain validates orders against:
   - GARCH(1,1) volatility thresholds
   - Value at Risk limits
   - Position size limits (Kelly criterion)
   - Max drawdown and daily loss limits
   - Kill switch state

6. **Execution** — `ExecutionGateway` trait routes validated orders to the target venue:
   - `AlpacaExecutor` for US equities (REST API)
   - `ClobClient` for Polymarket prediction markets (EIP-712 signed)
   - `MockExecutor` for paper trading

---

## Timestamp Model

All events carry nanosecond-precision timestamps following the NautilusTrader pattern:

| Field | Type | Purpose |
|:---|:---|:---|
| `ts_event` | `UnixNanos` | When the real-world event occurred (exchange timestamp) |
| `ts_init` | `UnixNanos` | When the system processed the event |
| `sequence_id` | `SequenceId` | Monotonic counter for total ordering |

The `Clock` trait enables swapping between `RealtimeClock` (live trading) and `DeterministicClock` (backtesting) without changing any downstream code.

---

## Event Bus

The daemon and TUI communicate via a TCP event bus:

- **Wire format:** Postcard binary serialization (zero-copy, compact)
- **Protocol:** Length-prefixed frames over TCP (`127.0.0.1:7001`)
- **Direction:** Bidirectional — daemon broadcasts `BotEvent`, TUI sends `ControlCommand`
- **In-process:** `tokio::sync::broadcast` channels with 100K buffer

---

## Crate Dependency Layers

```
Layer 0 (Foundation):   common
Layer 1 (Data):         ingestion, persistence
Layer 2 (Analysis):     feature, pricing, signals, ml
Layer 3 (Intelligence): ai, swarm_sim, knowledge_graph
Layer 4 (Trading):      strategy, risk, execution, oms, compliance
Layer 5 (Infra):        event_bus, metrics, alerts, backtest
Layer 6 (Interface):    daemon, tui, cli, web, web-dashboard
```

Each layer only depends on layers below it. No circular dependencies.

---

## Release Profile

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true
```

This produces a single optimized binary with link-time optimization across all crates.
