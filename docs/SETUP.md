# Setup Guide

## Prerequisites
- Rust 1.75+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- Git

## Step 1: Clone and Configure
```bash
git clone https://github.com/Ashutosh0x/rust-finance.git
cd rust-finance
cp .env.example .env
```

## Step 2: Get Your API Keys

### Finnhub (Required -- Market Data)
1. Go to https://finnhub.io/register
2. Create a free account
3. Copy your API key from the dashboard
4. Paste into `.env` as `FINNHUB_API_KEY=...`

**Free tier limits:** 60 calls/min, real-time US WebSocket
**What it provides:** Live trade prices, company profiles, quotes

### Alpaca (Required -- Paper Trading)
1. Go to https://app.alpaca.markets/signup
2. Create a free account
3. On dashboard, scroll to "API Keys" section (bottom-right)
4. Click "Generate New Keys"
5. **WARNING: Copy both keys immediately** -- secret is shown only once
6. Paste into `.env`:
   - `ALPACA_API_KEY=` (your Key ID)
   - `ALPACA_SECRET_KEY=` (your Secret Key)

**Default mode:** Paper trading (fake money, real market data)
**Free tier:** Unlimited paper trading, IEX real-time data

### Anthropic (Optional -- AI Analysis)
1. Go to https://console.anthropic.com/
2. Create account and add credits ($5 minimum)
3. Generate an API key
4. Paste into `.env` as `ANTHROPIC_API_KEY=...`

**Without this key:** Terminal works normally, AI features disabled
**Cost estimate:** ~$0.01-0.05 per analysis call (Sonnet)

### Polymarket (Optional -- Prediction Markets)
1. You only need a private key if you intend to send real transactions via the CLOB API.
2. For read-only access (Orderbook, Markets, Copy Trading signals), no real key is required; `POLYMARKET_DRY_RUN=true` will suffice.
3. If trading, paste a funded wallet (Polygon network) private key into `POLYMARKET_PRIVATE_KEY` and target Gnosis Safe proxy into `POLYMARKET_FUNDER_ADDRESS`.

## Step 3: Test Your Configuration
```bash
# Check if keys are loaded
cargo run --bin check_config

# Quick run validation (mock mode, no API keys needed)
USE_MOCK=1 cargo run -p daemon --release
```

## Step 4: Run
Start the background daemon process first:
```sh
cargo run -p daemon --release
```

In a separate terminal, launch the Terminal User Interface:
```sh
cargo run -p tui --release
```
