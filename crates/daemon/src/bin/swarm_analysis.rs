// ============================================================
// crates/daemon/src/bin/swarm_analysis.rs
//
// AI Swarm Analysis — Live Market Data Pipeline
//
// Fetches LIVE prices from Finnhub, computes real RSI from
// historical candles, runs 5,000-agent swarm simulation seeded
// at the live price, GARCH risk assessment, and Dexter AI.
//
// Usage:
//   cargo run -p daemon --bin swarm_analysis
//
// Env vars:
//   FINNHUB_API_KEY  — required for live prices + candles
//   OLLAMA_MODEL     — local LLM model (default: qwen3:8b)
//   GROQ_API_KEY     — fallback cloud LLM
//   DRY_RUN=1        — skips real API calls
// ============================================================

use std::sync::Arc;
use std::time::Instant;

use daemon::hybrid_pipeline::{FusedContext, QuantSnapshot};
use ingestion::finnhub_rest::FinnhubClient;
use risk::garch::{GarchParams, GarchState, GjrGarchParams, GjrGarchState};
use swarm_sim::{MarketState, SwarmConfig, SwarmEngine};
use tokio::sync::Semaphore;

/// Asset definition — prices are fetched LIVE from Finnhub, not hardcoded.
#[derive(Clone)]
struct AssetDef {
    symbol: &'static str,
    name: &'static str,
    class: &'static str,
    /// Annualized historical vol (approx, used for GARCH seeding)
    ann_vol: f64,
}

/// Parse custom symbols from SWARM_CUSTOM env var.
/// Format: SYMBOL:Name:Class:AnnVol (underscores become spaces in name/class)
/// Example: SWARM_CUSTOM=SUZLON.NS:Suzlon_Energy:Indian_Equity:0.45
/// Multiple: SWARM_CUSTOM=SUZLON.NS:Suzlon_Energy:Indian_Equity:0.45,RELIANCE.NS:Reliance:Indian_Equity:0.30
fn parse_custom_symbols() -> Vec<AssetDef> {
    let Ok(raw) = std::env::var("SWARM_CUSTOM") else {
        return vec![];
    };
    let mut defs = Vec::new();
    for entry in raw.split(',') {
        let parts: Vec<&str> = entry.trim().split(':').collect();
        if parts.len() >= 4 {
            // Leak strings to get 'static lifetime (fine for process-scoped config)
            let symbol: &'static str = Box::leak(parts[0].to_string().into_boxed_str());
            let name: &'static str = Box::leak(parts[1].replace('_', " ").into_boxed_str());
            let class: &'static str = Box::leak(parts[2].replace('_', " ").into_boxed_str());
            let ann_vol: f64 = parts[3].parse().unwrap_or(0.35);
            eprintln!(
                "  🔧 Custom asset: {} ({}) vol={:.0}%",
                symbol,
                name,
                ann_vol * 100.0
            );
            defs.push(AssetDef {
                symbol,
                name,
                class,
                ann_vol,
            });
        } else {
            eprintln!(
                "  ⚠ Invalid SWARM_CUSTOM entry '{}' — need SYMBOL:Name:Class:Vol",
                entry
            );
        }
    }
    defs
}

/// Runtime asset with live price and real RSI
struct Asset {
    symbol: String,
    name: String,
    class: String,
    price: f64, // LIVE from Finnhub
    ann_vol: f64,
    real_rsi: f64,               // Computed from real 14-day candle history
    historical_closes: Vec<f64>, // Real daily closes for seeding MarketState
}

const ASSET_DEFS: &[AssetDef] = &[
    // ── US Large-Cap Stocks ──
    AssetDef {
        symbol: "NVDA",
        name: "NVIDIA Corp",
        class: "US Equity",
        ann_vol: 0.55,
    },
    AssetDef {
        symbol: "AAPL",
        name: "Apple Inc",
        class: "US Equity",
        ann_vol: 0.25,
    },
    AssetDef {
        symbol: "TSLA",
        name: "Tesla Inc",
        class: "US Equity",
        ann_vol: 0.65,
    },
    AssetDef {
        symbol: "MSFT",
        name: "Microsoft Corp",
        class: "US Equity",
        ann_vol: 0.25,
    },
    AssetDef {
        symbol: "AMZN",
        name: "Amazon.com",
        class: "US Equity",
        ann_vol: 0.35,
    },
    // ── Broad ETFs ──
    AssetDef {
        symbol: "SPY",
        name: "S&P 500 ETF",
        class: "ETF",
        ann_vol: 0.15,
    },
    AssetDef {
        symbol: "QQQ",
        name: "Nasdaq-100 ETF",
        class: "ETF",
        ann_vol: 0.22,
    },
    AssetDef {
        symbol: "IWM",
        name: "Russell 2000 ETF",
        class: "ETF",
        ann_vol: 0.20,
    },
    AssetDef {
        symbol: "GLD",
        name: "Gold ETF",
        class: "Commodity",
        ann_vol: 0.15,
    },
    AssetDef {
        symbol: "TLT",
        name: "20+ Year Treasury ETF",
        class: "Fixed Income",
        ann_vol: 0.18,
    },
    // ── Sector ETFs ──
    AssetDef {
        symbol: "XLK",
        name: "Technology Select",
        class: "Sector ETF",
        ann_vol: 0.22,
    },
    AssetDef {
        symbol: "XLF",
        name: "Financial Select",
        class: "Sector ETF",
        ann_vol: 0.18,
    },
    AssetDef {
        symbol: "XLE",
        name: "Energy Select",
        class: "Sector ETF",
        ann_vol: 0.28,
    },
    // ── International / Emerging ──
    AssetDef {
        symbol: "EEM",
        name: "Emerging Markets ETF",
        class: "Intl ETF",
        ann_vol: 0.20,
    },
    AssetDef {
        symbol: "FXI",
        name: "China Large-Cap ETF",
        class: "Intl ETF",
        ann_vol: 0.30,
    },
];

/// Simulation parameters — rounds configurable via SWARM_ROUNDS env var
fn sim_rounds() -> u64 {
    std::env::var("SWARM_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .clamp(10, 500) // min 10 for statistical validity, max 500
}
const AGENT_COUNT: usize = 5_000;
/// Max allowed price drift from live seed before marking simulation unstable
const MAX_DRIFT_PCT: f64 = 0.05;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!();
    let rounds = sim_rounds();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║   🦀 RustForge AI Swarm Analysis — Live Market Intelligence    ║");
    println!(
        "║   5000 Agents · {} Rounds · GARCH(1,1) · Dexter AI          ║",
        rounds
    );
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();

    // ── LLM Provider detection ──────────────────────────────────────────────
    let dry_run = std::env::var("DRY_RUN").unwrap_or_default() == "1";
    let ollama_model = std::env::var("OLLAMA_MODEL").ok();
    let groq_key = std::env::var("GROQ_API_KEY").unwrap_or_default();
    let provider = std::env::var("LLM_PROVIDER")
        .unwrap_or_default()
        .to_lowercase();

    let use_ollama = provider == "ollama" || (provider.is_empty() && ollama_model.is_some());
    let ai_enabled = !dry_run && (use_ollama || (!groq_key.is_empty() && groq_key != "mock_key"));

    if dry_run {
        println!("  🧪 DRY_RUN=1 — skipping real API calls");
    } else if use_ollama {
        let model = ollama_model.as_deref().unwrap_or("qwen3:8b");
        println!("  ✅ Ollama — Dexter AI via local inference (no rate limits)");
        println!("  📡 Model: {}", model);
    } else if ai_enabled {
        let model =
            std::env::var("GROQ_MODEL").unwrap_or_else(|_| "openai/gpt-oss-120b".to_string());
        println!("  ✅ Groq API — Dexter AI via cloud inference");
        println!("  📡 Model: {}", model);
    } else {
        println!("  ⚠️  No LLM configured — running swarm + quant only");
        println!("     Set OLLAMA_MODEL=qwen3:8b or GROQ_API_KEY to enable AI");
    }

    // ── Ollama health check — wait up to 60s for Ollama to be ready ─────────
    if use_ollama && !dry_run {
        let host =
            std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
        println!();
        println!("  🔍 Checking Ollama availability...");
        match wait_for_ollama(&host, std::time::Duration::from_secs(60)).await {
            Ok(()) => {}
            Err(e) => {
                eprintln!("  ❌ Ollama not available: {}", e);
                eprintln!("     Start Ollama with: ollama serve");
                eprintln!("     Continuing without AI analysis...");
                // Don't exit — run swarm+quant without AI
            }
        }
    }

    // ── Build asset list (built-in + custom symbols) ──────────────────────
    let custom_defs = parse_custom_symbols();
    let all_defs: Vec<AssetDef> = {
        let mut v: Vec<AssetDef> = ASSET_DEFS.to_vec();
        v.extend(custom_defs.iter().cloned());
        v
    };

    // ── Optional symbol filter via SWARM_SYMBOLS env var ──────────────────
    // Applied BEFORE fetch so we only query the symbols you want.
    // Usage: SWARM_SYMBOLS=GLD or SWARM_SYMBOLS=GLD,NVDA,SPY or SWARM_SYMBOLS=SUZLON.NS
    let filtered_defs: Vec<AssetDef> = if let Ok(filter) = std::env::var("SWARM_SYMBOLS") {
        let symbols: Vec<&str> = filter.split(',').map(|s| s.trim()).collect();
        let filtered: Vec<AssetDef> = all_defs
            .into_iter()
            .filter(|d| symbols.iter().any(|s| s.eq_ignore_ascii_case(d.symbol)))
            .collect();
        if filtered.is_empty() {
            eprintln!("  ❌ No matching assets for filter '{}'", filter);
            eprintln!("     For custom symbols, also set: SWARM_CUSTOM=SUZLON.NS:Suzlon_Energy:Indian_Equity:0.45");
            std::process::exit(1);
        }
        println!("  🎯 Targeting {} asset(s): {}", filtered.len(), filter);
        filtered
    } else {
        all_defs
    };

    // ── Fetch LIVE prices from Finnhub ──────────────────────────────────────
    println!();
    println!("  📡 Fetching live prices from Finnhub...");
    let assets = match fetch_live_assets(&filtered_defs).await {
        Ok(a) => {
            println!("  ✅ {} live quotes received", a.len());
            a
        }
        Err(e) => {
            eprintln!(
                "  ❌ Finnhub error: {}. Cannot proceed without live prices.",
                e
            );
            eprintln!("     Verify FINNHUB_API_KEY is set and valid.");
            std::process::exit(1);
        }
    };
    println!();

    // ── Run all assets in parallel — semaphore gates AI calls to 2 concurrent ──
    // Swarm sim is CPU-instant. Only Dexter AI is slow (~3-4 min/call on 4GB VRAM).
    // Semaphore lets 2 AI calls run concurrently while sims all run immediately.
    let total_start = Instant::now();
    let ai_semaphore = Arc::new(Semaphore::new(2));

    let futures: Vec<_> = assets
        .iter()
        .enumerate()
        .map(|(idx, asset)| {
            let sem = ai_semaphore.clone();
            async move { analyse_asset(asset, ai_enabled, idx, rounds, sem).await }
        })
        .collect();
    let results = futures::future::join_all(futures).await;

    // ── Summary table ───────────────────────────────────────────────────────
    println!();
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!(
        "  {:<7} {:<22} {:<13} {:>9} {:>10} {:>7} {:>9} {:>7} {:>7} {:>6} {:>12}  {:<10}",
        "SYMBOL",
        "NAME",
        "CLASS",
        "LIVE $",
        "SWARM",
        "CONF%",
        "GARCH_A%",
        "R_RSI",
        "S_RSI",
        "DRIFT",
        "TOT_FLOW",
        "VERDICT"
    );
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════════════════");

    for r in &results {
        let direction_icon = match r.direction.as_str() {
            "Long" => "🟢 LONG ",
            "Short" => "🔴 SHORT",
            _ => "⚪ FLAT ",
        };
        let verdict_icon = match r.risk_verdict.as_str() {
            "Approved" => "✅ GO",
            "Hedge" => "🛡️ HEDGE",
            "Unstable" => "⚠️ DRIFT",
            _ => "❌ PASS",
        };
        let conviction = match r.conviction.as_str() {
            "High" => "H",
            "Medium" => "M",
            _ => "L",
        };
        let drift_str = format!("{:+.1}%", r.drift_pct * 100.0);

        println!("  {:<7} {:<22} {:<13} {:>9.2} {} {} {:>6.1}% {:>8.1}% {:>6.1} {:>5.1} {:>6} {:>+11.1}K  {:<10}",
            r.symbol, r.name, r.class, r.live_price,
            direction_icon, conviction,
            r.confidence * 100.0,
            r.garch_vol_ann * 100.0,
            r.real_rsi,
            r.swarm_rsi,
            drift_str,
            r.total_flow / 1000.0,
            verdict_icon,
        );
    }

    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════════════════");

    // ── Actionable signals ──────────────────────────────────────────────────
    let approved: Vec<&AssetResult> = results
        .iter()
        .filter(|r| r.risk_verdict == "Approved")
        .collect();
    let hedges: Vec<&AssetResult> = results
        .iter()
        .filter(|r| r.risk_verdict == "Hedge")
        .collect();
    let unstable: Vec<&AssetResult> = results
        .iter()
        .filter(|r| r.risk_verdict == "Unstable")
        .collect();

    println!();
    if !approved.is_empty() {
        println!("  📊 ACTIONABLE SIGNALS ({}):", approved.len());
        for r in &approved {
            let thesis_safe: String = r
                .ai_thesis
                .as_deref()
                .unwrap_or("(no AI)")
                .chars()
                .take(120)
                .collect();
            println!(
                "    → {} {} @ ${:.2} | conf={:.0}% | flow=${:+.0}K",
                r.symbol,
                r.direction,
                r.live_price,
                r.confidence * 100.0,
                r.total_flow / 1000.0
            );
            println!("      AI: {}", thesis_safe);
        }
    }

    if !hedges.is_empty() {
        println!();
        println!("  🛡️  HEDGE SIGNALS ({}):", hedges.len());
        for r in &hedges {
            println!(
                "    → {} — GARCH annualized vol={:.1}% exceeds 40% circuit breaker",
                r.symbol,
                r.garch_vol_ann * 100.0
            );
        }
    }

    if !unstable.is_empty() {
        println!();
        println!("  ⚠️  UNSTABLE SIMULATIONS ({}):", unstable.len());
        for r in &unstable {
            println!(
                "    → {} — price drifted {:+.1}% from live ${:.2} (sim=${:.2})",
                r.symbol,
                r.drift_pct * 100.0,
                r.live_price,
                r.sim_price
            );
        }
    }

    let rejected = results.len() - approved.len() - hedges.len() - unstable.len();
    let duration = total_start.elapsed();
    println!();
    println!("  ⏱ Total: {:.2}s | {} assets | {} approved, {} hedge, {} unstable, {} rejected | {:.0}ms/asset",
        duration.as_secs_f64(), assets.len(),
        approved.len(), hedges.len(), unstable.len(), rejected,
        duration.as_secs_f64() * 1000.0 / assets.len() as f64);
    println!();

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// ─── Live Data Fetch ────────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

/// Fetch live quotes + real RSI from Finnhub for all assets.
/// Candle fallback chain: Finnhub candles → Alpaca Data API v2 bars → single-point price.
/// Finnhub free tier: quotes work, but candles often return 403.
/// Alpaca free tier: IEX bars for all US stocks/ETFs, 7+ years history.
async fn fetch_live_assets(defs: &[AssetDef]) -> anyhow::Result<Vec<Asset>> {
    let client = FinnhubClient::from_env()?;

    // Check for Alpaca credentials for candle fallback
    let alpaca_key = std::env::var("ALPACA_API_KEY").ok();
    let alpaca_secret = std::env::var("ALPACA_SECRET_KEY").ok();
    let has_alpaca = alpaca_key.is_some() && alpaca_secret.is_some();

    if has_alpaca {
        eprintln!("  📊 Alpaca Data API available for candle fallback");
    }

    let mut assets = Vec::new();

    for def in defs {
        // Small delay between assets to respect rate limits
        if !assets.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }

        // Fetch live quote — Finnhub first, Yahoo Finance fallback for non-US
        let finnhub_quote = client.get_quote(def.symbol).await;
        let (current_price, change_pct) = match finnhub_quote {
            Ok(q) if q.current > 0.0 => (q.current, q.change_percent.unwrap_or(0.0)),
            _ => {
                // Finnhub doesn't have this symbol — try Yahoo Finance
                match fetch_yahoo_quote(def.symbol).await {
                    Ok((price, chg)) => {
                        eprintln!(
                            "    📊 {} quote via Yahoo Finance: ${:.2}",
                            def.symbol, price
                        );
                        (price, chg)
                    }
                    Err(e) => {
                        eprintln!(
                            "    ⚠ {} no quote from any source: {} — skipping",
                            def.symbol, e
                        );
                        continue;
                    }
                }
            }
        };

        if current_price <= 0.0 {
            eprintln!("    ⚠ {} returned price $0 — skipping", def.symbol);
            continue;
        }

        // Try Finnhub candles first, fall back to Alpaca bars
        let now = chrono::Utc::now().timestamp();
        let from = now - (45 * 86400); // 45 days back
        let candles = client.get_stock_candles(def.symbol, "D", from, now).await;

        let (real_rsi, historical_closes) = match candles {
            Ok(c) if c.close.len() >= 15 => {
                let rsi = compute_rsi_14(&c.close);
                eprintln!(
                    "    📊 {} got {} Finnhub candles, RSI={:.1}",
                    def.symbol,
                    c.close.len(),
                    rsi
                );
                (rsi, c.close)
            }
            Ok(c) if c.close.len() >= 2 => {
                eprintln!(
                    "    ⚠ {} got only {} Finnhub candles (need 15 for RSI)",
                    def.symbol,
                    c.close.len()
                );
                (50.0, c.close)
            }
            _ => {
                // Finnhub candles failed — try Alpaca (US only), then Yahoo Finance (global)
                let mut got_bars = false;
                let mut result = (50.0_f64, vec![current_price]);

                // Tier 2: Alpaca Data API v2 (US stocks/ETFs only)
                if !got_bars {
                    if let (Some(ref key), Some(ref secret)) = (&alpaca_key, &alpaca_secret) {
                        match fetch_alpaca_daily_bars(def.symbol, key, secret, 45).await {
                            Ok(closes) if closes.len() >= 15 => {
                                let rsi = compute_rsi_14(&closes);
                                eprintln!(
                                    "    📊 {} got {} Alpaca bars, RSI={:.1}",
                                    def.symbol,
                                    closes.len(),
                                    rsi
                                );
                                result = (rsi, closes);
                                got_bars = true;
                            }
                            Ok(closes) if closes.len() >= 2 => {
                                eprintln!(
                                    "    ⚠ {} got only {} Alpaca bars",
                                    def.symbol,
                                    closes.len()
                                );
                                result = (50.0, closes);
                                got_bars = true;
                            }
                            _ => {} // Fall through to Yahoo
                        }
                    }
                }

                // Tier 3: Yahoo Finance chart API (global — supports .NS, .L, .PA etc.)
                if !got_bars {
                    match fetch_yahoo_daily_closes(def.symbol, 45).await {
                        Ok(closes) if closes.len() >= 15 => {
                            let rsi = compute_rsi_14(&closes);
                            eprintln!(
                                "    📊 {} got {} Yahoo bars, RSI={:.1}",
                                def.symbol,
                                closes.len(),
                                rsi
                            );
                            result = (rsi, closes);
                        }
                        Ok(closes) if closes.len() >= 2 => {
                            eprintln!("    ⚠ {} got only {} Yahoo bars", def.symbol, closes.len());
                            result = (50.0, closes);
                        }
                        Ok(_) => {
                            eprintln!("    ⚠ {} no historical bars from any source", def.symbol);
                        }
                        Err(e) => {
                            eprintln!(
                                "    ❌ {} all candle sources failed (last: {})",
                                def.symbol, e
                            );
                        }
                    }
                }

                result
            }
        };

        eprint!(
            "  📈 {:<6} ${:<9.2}  RSI={:.1}  ",
            def.symbol, current_price, real_rsi
        );
        if change_pct >= 0.0 {
            eprintln!("▲ {:+.2}%", change_pct);
        } else {
            eprintln!("▼ {:+.2}%", change_pct);
        }

        assets.push(Asset {
            symbol: def.symbol.to_string(),
            name: def.name.to_string(),
            class: def.class.to_string(),
            price: current_price,
            ann_vol: def.ann_vol,
            real_rsi,
            historical_closes,
        });
    }

    if assets.is_empty() {
        anyhow::bail!("No valid quotes received from Finnhub");
    }

    Ok(assets)
}

/// Fetch daily close prices from Alpaca Data API v2.
/// Free tier uses IEX feed — works for all US stocks and ETFs.
/// Returns a Vec of close prices (oldest first).
async fn fetch_alpaca_daily_bars(
    symbol: &str,
    api_key: &str,
    secret_key: &str,
    days_back: i64,
) -> anyhow::Result<Vec<f64>> {
    let client = reqwest::Client::new();

    let start_date = chrono::Utc::now() - chrono::Duration::days(days_back);
    let start_str = start_date.format("%Y-%m-%d").to_string();

    let url = format!("https://data.alpaca.markets/v2/stocks/{}/bars", symbol);

    let resp = client
        .get(&url)
        .header("APCA-API-KEY-ID", api_key)
        .header("APCA-API-SECRET-KEY", secret_key)
        .query(&[
            ("timeframe", "1Day"),
            ("start", &start_str),
            ("limit", "50"),
            ("feed", "iex"),
            ("adjustment", "split"),
        ])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Alpaca bars {} for {}: {}", status, symbol, body);
    }

    let json: serde_json::Value = resp.json().await?;

    // Alpaca response: { "bars": [ { "c": 150.0, ... }, ... ], "symbol": "AAPL", ... }
    let closes: Vec<f64> = json
        .get("bars")
        .and_then(|b| b.as_array())
        .map(|bars| {
            bars.iter()
                .filter_map(|bar| bar.get("c").and_then(|c| c.as_f64()))
                .collect()
        })
        .unwrap_or_default();

    Ok(closes)
}

/// Fetch a live quote from Yahoo Finance chart API.
/// Returns (current_price, change_percent). Works globally.
async fn fetch_yahoo_quote(symbol: &str) -> anyhow::Result<(f64, f64)> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}",
        symbol
    );

    let resp = client
        .get(&url)
        .query(&[("interval", "1d"), ("range", "2d")])
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Yahoo Finance quote {} for {}: {}", status, symbol, body);
    }

    let json: serde_json::Value = resp.json().await?;

    // Get current price from meta.regularMarketPrice
    let price = json
        .pointer("/chart/result/0/meta/regularMarketPrice")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    // Get previous close to calculate change %
    let prev_close = json
        .pointer("/chart/result/0/meta/chartPreviousClose")
        .or_else(|| json.pointer("/chart/result/0/meta/previousClose"))
        .and_then(|v| v.as_f64())
        .unwrap_or(price);

    let change_pct = if prev_close > 0.0 {
        ((price - prev_close) / prev_close) * 100.0
    } else {
        0.0
    };

    if price <= 0.0 {
        anyhow::bail!("Yahoo Finance returned $0 for {}", symbol);
    }

    Ok((price, change_pct))
}

/// Fetch daily close prices from Yahoo Finance chart API (unofficial).
/// Works globally — supports .NS (India), .L (London), .PA (Paris), etc.
/// No API key required. Returns Vec of close prices (oldest first).
async fn fetch_yahoo_daily_closes(symbol: &str, days_back: i64) -> anyhow::Result<Vec<f64>> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    // Yahoo Finance v8 chart API: range=45d, interval=1d
    let range = format!("{}d", days_back);
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}",
        symbol
    );

    let resp = client
        .get(&url)
        .query(&[("interval", "1d"), ("range", &range)])
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Yahoo Finance {} for {}: {}", status, symbol, body);
    }

    let json: serde_json::Value = resp.json().await?;

    // Yahoo response structure:
    // { "chart": { "result": [ { "indicators": { "quote": [ { "close": [...] } ] } } ] } }
    let closes: Vec<f64> = json
        .pointer("/chart/result/0/indicators/quote/0/close")
        .and_then(|c| c.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();

    Ok(closes)
}

/// Compute RSI-14 using Wilder's smoothing on close prices.
fn compute_rsi_14(closes: &[f64]) -> f64 {
    if closes.len() < 15 {
        return 50.0;
    }

    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;

    // Initial SMA for first 14 periods
    for i in 1..15 {
        let change = closes[i] - closes[i - 1];
        if change > 0.0 {
            avg_gain += change;
        } else {
            avg_loss += change.abs();
        }
    }
    avg_gain /= 14.0;
    avg_loss /= 14.0;

    // Wilder's smoothing for remaining periods
    for i in 15..closes.len() {
        let change = closes[i] - closes[i - 1];
        let (gain, loss) = if change > 0.0 {
            (change, 0.0)
        } else {
            (0.0, change.abs())
        };
        avg_gain = (avg_gain * 13.0 + gain) / 14.0;
        avg_loss = (avg_loss * 13.0 + loss) / 14.0;
    }

    if avg_loss == 0.0 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

// ═══════════════════════════════════════════════════════════════════════════════
// ─── Per-Asset Analysis ─────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

struct AssetResult {
    symbol: String,
    name: String,
    class: String,
    live_price: f64,
    sim_price: f64,
    drift_pct: f64,
    direction: String,
    conviction: String,
    confidence: f64,
    garch_vol_ann: f64,
    real_rsi: f64,
    swarm_rsi: f64,
    total_flow: f64,
    risk_verdict: String,
    ai_thesis: Option<String>,
}

async fn analyse_asset(
    asset: &Asset,
    ai_enabled: bool,
    idx: usize,
    rounds: u64,
    ai_semaphore: Arc<Semaphore>,
) -> AssetResult {
    let start = Instant::now();

    // ── Swarm simulation — seeded at LIVE price + real candle history ────────
    // Each asset gets a deterministic but unique seed derived from its symbol.
    // This ensures different assets produce different agent behaviors.
    let asset_seed = symbol_seed(&asset.symbol);
    let config = SwarmConfig {
        agent_count: AGENT_COUNT,
        annualized_vol: asset.ann_vol,
        round_delay_ms: 0,
        db_path: ":memory:".to_string(),
        seed: asset_seed,
        ..SwarmConfig::default()
    };

    let market = MarketState::new(&asset.symbol, asset.price); // LIVE price as seed
    let mut engine = SwarmEngine::new(config.clone(), market);

    // Seed price history with real daily closes — RSI is meaningful from round 0
    for close in &asset.historical_closes {
        engine.market.price_history.push_back(*close);
    }
    // Update VWAP from real data instead of single-point init
    if !asset.historical_closes.is_empty() {
        let mean_close: f64 =
            asset.historical_closes.iter().sum::<f64>() / asset.historical_closes.len() as f64;
        engine.market.vwap = mean_close;
    }

    let mut total_flow = 0.0;
    let mut last_step = None;
    for _ in 0..rounds {
        let step = engine.step_round();
        total_flow += step.net_flow_usd;
        last_step = Some(step);
    }
    let step = last_step.unwrap();

    // ── Price drift check ───────────────────────────────────────────────────
    let drift_pct = (step.price_after - asset.price) / asset.price;

    // ── GJR-GARCH — asymmetric volatility (leverage effect) ─────────────────
    // GJR-GARCH adds a gamma leverage term: negative returns amplify vol more
    // than positive returns. This prevents NVDA/TSLA from always triggering
    // the 40% vol circuit breaker on bullish moves.
    let daily_var_seed = (asset.ann_vol / (252_f64).sqrt()).powi(2);
    let gjr_params = GjrGarchParams {
        omega: daily_var_seed * 0.01,
        alpha: 0.05,
        beta: 0.90,
        gamma: 0.08, // leverage coefficient — negative shocks amplify vol
    };
    let mut garch = GjrGarchState::new(gjr_params, daily_var_seed);

    // Also keep symmetric GARCH for comparison
    let sym_params = GarchParams {
        omega: daily_var_seed * 0.01,
        alpha: 0.09,
        beta: 0.90,
    };
    let mut _sym_garch = GarchState::new(sym_params, daily_var_seed);

    let prices: Vec<f64> = engine.market.price_history.iter().cloned().collect();
    let rpd = config.rounds_per_day as usize;
    if prices.len() > rpd {
        let mut i = 0;
        while i + rpd < prices.len() {
            let daily_ret = (prices[i + rpd] / prices[i]).ln();
            garch.update(daily_ret);
            _sym_garch.update(daily_ret);
            i += rpd;
        }
    }
    let garch_vol_ann = garch.current_vol_annualized();

    // ── Risk verdict ────────────────────────────────────────────────────────
    let direction_str = format!("{:?}", step.signal.direction);
    let conviction_str = format!("{:?}", step.signal.conviction);

    let risk_verdict = if drift_pct.abs() > MAX_DRIFT_PCT {
        "Unstable".to_string() // simulation drifted too far from live price
    } else if garch_vol_ann > 0.40 {
        "Hedge".to_string()
    } else if step.signal.confidence > 0.50 && direction_str != "Neutral" {
        "Approved".to_string()
    } else {
        "Rejected".to_string()
    };

    // ── Dexter AI — semaphore-gated, skip for unstable simulations ───────────
    let ai_thesis = if ai_enabled && (risk_verdict == "Approved" || risk_verdict == "Hedge") {
        // Acquire semaphore permit — limits concurrent Ollama/Groq calls
        let _permit = ai_semaphore.acquire().await.unwrap();

        // Only stagger for Groq (rate-limited). Ollama has no rate limits.
        let is_groq = std::env::var("LLM_PROVIDER").unwrap_or_default() != "ollama"
            && std::env::var("OLLAMA_MODEL").is_err();
        if is_groq {
            tokio::time::sleep(std::time::Duration::from_millis(4000 * idx as u64)).await;
        }
        match call_dexter_for_asset(asset, &step, garch_vol_ann).await {
            Ok(signal) => Some(signal.thesis),
            Err(e) => Some(format!("AI error: {}", e)),
        }
    } else {
        None
    };

    let elapsed = start.elapsed();
    let verdict_tag = match risk_verdict.as_str() {
        "Approved" => "✓",
        "Hedge" => "⚠",
        "Unstable" => "✗",
        _ => "–",
    };
    let ai_tag = if ai_thesis.is_some() { " +AI" } else { "" };
    eprintln!(
        "  {} {:<7} ({}) {:.0}ms{}",
        verdict_tag,
        asset.symbol,
        asset.class,
        elapsed.as_millis(),
        ai_tag
    );

    AssetResult {
        symbol: asset.symbol.clone(),
        name: asset.name.clone(),
        class: asset.class.clone(),
        live_price: asset.price,
        sim_price: step.price_after,
        drift_pct,
        direction: direction_str,
        conviction: conviction_str,
        confidence: step.signal.confidence,
        garch_vol_ann,
        real_rsi: asset.real_rsi,
        swarm_rsi: step.signal.rsi,
        total_flow,
        risk_verdict,
        ai_thesis,
    }
}

/// Call Dexter AI — now uses LIVE price + REAL RSI in the context
async fn call_dexter_for_asset(
    asset: &Asset,
    step: &swarm_sim::engine::SwarmStep,
    garch_vol_ann: f64,
) -> anyhow::Result<ai::dexter::DexterSignal> {
    let quant = QuantSnapshot {
        symbol: asset.symbol.clone(),
        price: asset.price,     // LIVE price, not simulated
        rsi_14: asset.real_rsi, // REAL RSI from Finnhub candles
        garch_vol_forecast: garch_vol_ann,
        heston_implied_vol: 0.0,
        vwap: asset.price * 0.998,
        order_book_imbalance: step.signal.bullish_prob - step.signal.bearish_prob,
        momentum_signal: step.signal.momentum_1h,
        bsm_fair_value: None,
    };

    let fused = FusedContext {
        quant,
        swarm: step.clone(),
        graph_context: format!(
            "{} ({}) — Live ${:.2}. {:.0}% bullish, {:.0}% bearish. Net flow: ${:+.0}K. Regime: {:?}",
            asset.name, asset.class, asset.price,
            step.signal.bullish_prob * 100.0,
            step.signal.bearish_prob * 100.0,
            step.net_flow_usd / 1000.0,
            step.signal.regime,
        ),
        impact_table: format!(
            "GARCH annualized vol: {:.1}%. Real RSI-14: {:.1}. Conviction: {:?}. Confidence: {:.0}%.",
            garch_vol_ann * 100.0, asset.real_rsi, step.signal.conviction, step.signal.confidence * 100.0,
        ),
    };

    ai::dexter::analyse(&fused).await
}

// ═══════════════════════════════════════════════════════════════════════════════
// ─── Utility Functions ──────────────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════════

/// Derive a deterministic, unique seed from a symbol name.
/// Each symbol produces a different seed, ensuring per-asset behavioral diversity
/// in the swarm simulation while remaining reproducible across runs.
fn symbol_seed(symbol: &str) -> u64 {
    symbol
        .bytes()
        .enumerate()
        .fold(0x517cc1b727220a95u64, |acc, (i, b)| {
            acc.wrapping_mul(6364136223846793005)
                .wrapping_add((b as u64) << ((i % 8) * 8))
        })
}

/// Wait for Ollama to be ready, with exponential backoff.
/// Returns Ok(()) when Ollama responds to GET /api/tags.
/// Returns Err after `timeout` if Ollama never becomes available.
async fn wait_for_ollama(host: &str, timeout: std::time::Duration) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let start = Instant::now();
    let mut attempt = 0u32;

    loop {
        match client
            .get(format!("{}/api/tags", host))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                println!(
                    "  ✅ Ollama connected (took {:.1}s)",
                    start.elapsed().as_secs_f64()
                );
                return Ok(());
            }
            Ok(resp) => {
                eprintln!("  ⚠️ Ollama responded with {}, retrying...", resp.status());
            }
            Err(e) => {
                if start.elapsed() > timeout {
                    anyhow::bail!(
                        "Ollama not available after {:.0}s: {}",
                        timeout.as_secs_f64(),
                        e
                    );
                }
                eprintln!("  ⏳ Waiting for Ollama... attempt {} ({})", attempt + 1, e);
            }
        }
        // Exponential backoff: 1s, 2s, 4s, 8s, capped at 10s
        let delay = std::time::Duration::from_secs((1u64 << attempt.min(3)).min(10));
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}
