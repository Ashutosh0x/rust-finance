# Configuration Reference

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `FINNHUB_API_KEY` | Yes* | -- | Finnhub.io API key for market data |
| `ALPACA_API_KEY` | Yes* | -- | Alpaca Key ID for paper/live trading |
| `ALPACA_SECRET_KEY` | Yes* | -- | Alpaca Secret Key |
| `ALPACA_BASE_URL` | No | `https://paper-api.alpaca.markets` | Paper or live endpoint |
| `ANTHROPIC_API_KEY` | No | -- | Enables AI signal commentary |
| `USE_MOCK` | No | `0` | `1` = synthetic data, no keys needed |
| `SOL_PRIVATE_KEY` | No | -- | Base58 Solana key (experimental) |
| `POLYMARKET_PRIVATE_KEY` | No | -- | Hex private key for Polymarket CLOB signing |
| `POLYMARKET_FUNDER_ADDRESS`| No | -- | Gnosis Safe proxy wallet address |
| `POLYMARKET_DRY_RUN`| No | `false` | Set to `true` to disable live orders |
| `RUST_LOG` | No | `info` | Log level: `debug`, `info`, `warn`, `error` |

*\*Not required when `USE_MOCK=1`*

## Graceful Degradation

RustForge degrades gracefully based on available keys:

| Keys Available | Behavior |
|---|---|
| None + `USE_MOCK=1` | Synthetic data, TUI works, no real trades |
| Finnhub only | Live market data, no trading |
| Finnhub + Alpaca | Full paper trading, live data |
| All + Anthropic | Paper trading + AI signal annotations |

## Security Notes
- Never commit your `.env` file (it's in `.gitignore`)
- Use paper trading keys for development
- The terminal will warn on startup if live Alpaca URL is detected
