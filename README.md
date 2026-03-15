# RustForge Terminal (rust-finance)

<div align="center">
  <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/Tokio-1A2421?style=for-the-badge&logo=Rust&logoColor=white" alt="Tokio" />
  <img src="https://img.shields.io/badge/Solana-14F195?style=for-the-badge&logo=Solana&logoColor=white" alt="Solana" />
  <img src="https://img.shields.io/badge/Anthropic-FF7F50?style=for-the-badge&logo=Anthropic&logoColor=white" alt="Anthropic" />
  <img src="https://img.shields.io/badge/Ratatui-0A0C0F?style=for-the-badge&logo=Linux&logoColor=white" alt="Ratatui" />
  <img src="https://img.shields.io/badge/WebSocket-010101?style=for-the-badge&logo=socket.io&logoColor=white" alt="WebSocket" />
  <br />
  <img src="https://img.shields.io/badge/NASDAQ-0090F7?style=for-the-badge&logo=nasdaq&logoColor=white" alt="NASDAQ" />
  <img src="https://img.shields.io/badge/NYSE-092140?style=for-the-badge&logo=new-york-stock-exchange&logoColor=white" alt="NYSE" />
  <img src="https://img.shields.io/badge/LSE-000000?style=for-the-badge&logo=london-stock-exchange&logoColor=white" alt="LSE" />
  <img src="https://img.shields.io/badge/Euronext-0A2140?style=for-the-badge" alt="Euronext" />
  <img src="https://img.shields.io/badge/TSX-E0121A?style=for-the-badge" alt="TSX" />
  <img src="https://img.shields.io/badge/FWB-004B87?style=for-the-badge" alt="FWB" />
  <img src="https://img.shields.io/badge/TSE-D41A21?style=for-the-badge" alt="TSE" />
  <img src="https://img.shields.io/badge/SSE-CC0000?style=for-the-badge" alt="SSE" />
  <img src="https://img.shields.io/badge/HKEX-D0101A?style=for-the-badge" alt="HKEX" />
  <img src="https://img.shields.io/badge/NSE-004481?style=for-the-badge" alt="NSE" />
  <img src="https://img.shields.io/badge/BSE-004C8F?style=for-the-badge" alt="BSE" />
  <img src="https://img.shields.io/badge/ASX-11202C?style=for-the-badge" alt="ASX" />
  <img src="https://img.shields.io/badge/Binance-FCD535?style=for-the-badge&logo=binance&logoColor=black" alt="Binance" />
  <img src="https://img.shields.io/badge/Alpaca-FACC15?style=for-the-badge&logo=alpaca&logoColor=black" alt="Alpaca" />
  <img src="https://img.shields.io/badge/Finnhub-000000?style=for-the-badge&logo=graphql&logoColor=white" alt="Finnhub" />
</div>

A high-performance, low-latency trading terminal and daemon built completely in Rust. Engineered for direct connection to market data streams (Finnhub, Alpaca), real-time AI signal analysis, and Solana-based trade execution.

![Rust Trading Terminal](https://raw.githubusercontent.com/Ashutosh0x/rust-finance/main/rust%20terminal.png)
 
## System Architecture

```mermaid
graph TD;
    subgraph "External Feeds"
        FH(Finnhub WS) --> |Market Data| Ingest
        ALP(Alpaca) --> |Account Data| Ingest
        LLM(Anthropic Claude) <--> AI
    end

    subgraph "RustForge Daemon"
        Ingest(Ingestion Engine) --> Relay(Latency Relay)
        Relay --> Bus(TCP Event Bus)
        Bus --> AI(AI Engine - Dexter/MiroFish)
        AI --> Strategy(Strategy Dispatcher)
        Strategy --> Exec(Execution Guard)
        Exec --> Blockchain(Solana RPC/Jupiter)
    end
    
    subgraph "Elite Quant Algorithms"
        Strategy --> MM(Avellaneda-Stoikov MM)
        Strategy --> Arb(Z-Score Stat Arb)
        Strategy --> PPO(PPO RL Agent)
    end
```

## Workspace Crates

The workspace is organized into discrete, highly decoupled crates:

* **`daemon`**: The central orchestrator. It manages the Tokio asynchronous runtime, spawns the EventBus, starts ingestion pipelines, controls the AI analyst intervals, and routes signals to the execution engine.
* **`tui`**: A standalone Ratatui application featuring an advanced 3-column layout mimicking professional desktop terminals. It subscribes to the `event_bus` to render watchlists, deep order books, high-res braille charts, and live AI intelligence.
* **`ai`**: Contains `DexterAnalyst` and `MiroFishSimulator`. Interacts natively with Anthropic APIs to detect catalysts, perform fundamental analysis, and run swarm probability algorithms on market feeds.
* **`ingestion`**: Connects to `Finnhub` and `Alpaca` WebSockets. Normalizes trade and quote data into a standard `MarketEvent` format and pumps it into the system at extremely low latency.
* **`relay`**: Handles network routing and edge measurement. Specifically benchmarks multiple RPC nodes (Helius, Triton, QuickNode) and routes transactions through the lowest-latency path available.
* **`event_bus`**: A custom-built, lightweight TCP broadcasting system that decouples producers and consumers. Allows the TUI and Web Dashboards to run in entirely separate processes from the Daemon.
* **`persistence`**: Storage layer designed to record transactional records, system P&L tracking, and order history.
* **`common`**: Shared models, structs, commands, and `BotEvent` enumerations used across all systems to guarantee strict typing on inter-process communications.

## Configuration & Usage

The system expects several environment variables to be set for external API integrations:

```sh
export ANTHROPIC_API_KEY="..."
export FINNHUB_API_KEY="..."
export ALPACA_API_KEY="..."
export ALPACA_SECRET_KEY="..."
export USE_MOCK="1" # Enables mocked market generation for UI testing
```

### Running the System

Start the background daemon process first:
```sh
cargo run -p daemon --release
```

In a separate terminal, launch the Terminal User Interface:
```sh
cargo run -p tui --release
```

## Features

* **Real-time Market Data:** Direct integrations with Finnhub and Alpaca WebSocket streams for sub-millisecond market events.
* **Low-Latency Order Execution:** Hardware-accelerated Solana RPC interactions via intelligent `relay` routing.
* **Dual AI Decision Engines:**
    * **Dexter Analyst AI:** Reads fundamental data and market news simultaneously using Anthropic APIs to identify macro catalysts.
    * **MiroFish Swarm AI:** Simulates 5,000 algorithmic agent iterations to predict micro-price movements and generate actionable probability matrices.
* **Terminal UI (TUI):** A professional-grade, multi-column dashboard rendered directly in your terminal using Ratatui. Features high-res Braille price charts, real-time depth-of-market order books, and live portfolio P&L tracking.
* **Decoupled Event Bus:** TCP-based internal broadcasting ensures the TUI, Web Dashboard, and Daemon can crash and restart independently without losing state.

## Detailed Documentation

For a deep dive into the system's internal workings, component integration details, and deployment guides, please refer to the inner documentation:

* [Architecture Overview](./docs/ARCHITECTURE.md) *(Coming Soon)*
* [AI Analyst Integration](./docs/AI_INTEGRATION.md) *(Coming Soon)*
* [WebSocket Normalization Strategies](./docs/WSS_INGESTION.md) *(Coming Soon)*

*(Note: Documentation nodes are actively being written and compiled by the contributors.)*

## Contributing

We strictly enforce high professional standards for contributions. 

Please take the time to read our detailed **[Contribution Guidelines](CONTRIBUTING.md)** before submitting a pull request. It contains instructions regarding:
* Local Environment Setup
* Cargo Testing and Formatting requirements
* Commit Message Standards

## UI and Visual Constraints

The TUI utilizes `Constraint::Length` and custom Ratatui widget styling to enforce a strict immutable grid layout. Custom hex colors have been applied globally to match a proprietary theme design.
