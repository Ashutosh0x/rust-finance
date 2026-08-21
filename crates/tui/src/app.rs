use crate::widgets::candlestick_widget::{Candle, CandlestickState};
use crate::widgets::chart_widget::{ChartState, ChartStats};
use std::collections::VecDeque;

// ── Data structures for live panels ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WatchlistItem {
    pub symbol: String,
    pub name: String,
    pub price: f64,
    pub change_pct: f64,
}

#[derive(Debug, Clone)]
pub struct PositionEntry {
    pub symbol: String,
    pub holding: f64,
    pub pnl_pct: f64,
}

#[derive(Debug, Clone)]
pub struct OrderBookRow {
    pub ask_price: f64,
    pub ask_size: u64,
    pub ask_total: f64,
    pub bid_price: f64,
    pub bid_size: u64,
    pub bid_total: f64,
}

#[derive(Debug, Clone)]
pub struct NewsItem {
    pub source: String,
    pub time_ago: String,
    pub headline: String,
}

#[derive(Debug, Clone)]
pub struct AlertItem {
    pub text: String,
    pub severity: AlertSeverity,
}

#[derive(Debug, Clone, Copy)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

use common::models::exchange::{ExchangeInfo, ExchangeName, ExchangeStatus};

// ── Order types for dialogs ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DialogOrderType {
    Market,
    Limit,
    Stop,
    Ioc,
}

impl DialogOrderType {
    pub fn label(&self) -> &'static str {
        match self {
            DialogOrderType::Market => "MKT",
            DialogOrderType::Limit => "LMT",
            DialogOrderType::Stop => "STP",
            DialogOrderType::Ioc => "IOC",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            DialogOrderType::Market => DialogOrderType::Limit,
            DialogOrderType::Limit => DialogOrderType::Stop,
            DialogOrderType::Stop => DialogOrderType::Ioc,
            DialogOrderType::Ioc => DialogOrderType::Market,
        }
    }
}

// ── App Screens ───────────────────────────────────────────────────────────────

pub enum AppScreen {
    Setup(SetupState),
    Dashboard,
}

pub struct SetupState {
    pub fields: Vec<KeyField>,
    pub active_field: usize,
    pub error_msg: Option<String>,
    pub show_confirmation: bool,
}

pub struct KeyField {
    pub name: &'static str,
    pub label: &'static str,
    pub value: String,
    pub required: bool,
    pub masked: bool,
    pub hint: &'static str,
}

// ── Main App ──────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,
    pub connection_status: String,
    pub show_help: bool,
    pub paper_mode: bool,
    pub active_panel: u8,

    pub active_symbol: String,

    // ── Dialogs ───────────────────────────────────────────────────────────
    pub show_buy_dialog: bool,
    pub show_sell_dialog: bool,
    pub order_qty_input: String,
    pub order_price_input: String,
    pub dialog_order_type: DialogOrderType,

    // ── Chart ─────────────────────────────────────────────────────────────
    pub chart_data: Vec<(f64, f64)>,
    pub volume_data: Vec<(f64, f64)>,
    pub chart_state: ChartState,
    pub chart_stats: ChartStats,

    // ── Candlestick Chart ─────────────────────────────────────────────────
    pub candles: Vec<Candle>,
    pub candle_state: CandlestickState,

    // ── Live Data ─────────────────────────────────────────────────────────
    pub watchlist: Vec<WatchlistItem>,
    pub positions: Vec<PositionEntry>,
    pub order_book: Vec<OrderBookRow>,
    pub news: VecDeque<NewsItem>,
    pub alerts: VecDeque<AlertItem>,

    // ── Exchanges ─────────────────────────────────────────────────────────
    pub exchanges: Vec<ExchangeInfo>,

    // ── Kill Switch ───────────────────────────────────────────────────────
    pub kill_switch_active: bool,
    pub kill_switch_timestamp: Option<String>,
    pub kill_switch_orders_cancelled: u32,
    pub kill_switch_positions_closed: u32,

    // ── Dexter AI (enhanced) ──────────────────────────────────────────────
    pub dexter_output: Vec<String>,
    pub dexter_recommendation: Option<String>,
    pub dexter_loading: bool,
    pub dexter_confidence: f64,
    pub dexter_conviction: String,
    pub dexter_stop_loss_pct: f64,
    pub dexter_take_profit_pct: f64,
    pub dexter_kelly_fraction: f64,
    pub dexter_position_size_pct: f64,
    pub dexter_rationale: String,
    pub dexter_regime: String,
    pub dexter_safety_gate_pass: bool,
    pub dexter_call_count: u32,

    // ── Mirofish (enhanced) ───────────────────────────────────────────────
    pub mirofish_running: bool,
    pub mirofish_rally_pct: f64,
    pub mirofish_sideways_pct: f64,
    pub mirofish_dip_pct: f64,
    pub mirofish_agent_count: u32,
    pub mirofish_sim_time_ms: f64,
    pub mirofish_order_imbalance: f64,
    pub mirofish_simulated_vol: f64,
    pub mirofish_agent_agreement: f64,
    pub mirofish_bias_detected: bool,

    // ── Trading ───────────────────────────────────────────────────────────
    pub day_pnl: f64,
    pub available_power: f64,
    pub orders_sent: u32,
    pub fills_count: u32,
    pub rejections_count: u32,

    // ── Session ───────────────────────────────────────────────────────────
    pub sequence_id: u64,
    pub session_start: std::time::Instant,

    // ── Scroll states ─────────────────────────────────────────────────────
    pub watchlist_scroll: usize,
    pub news_scroll: usize,
    pub orderbook_scroll: usize,
}

impl App {
    pub fn new(initial_screen: AppScreen) -> Self {
        Self {
            screen: initial_screen,
            should_quit: false,
            connection_status: "Standalone Mode (no daemon)".to_string(),
            show_help: false,
            paper_mode: true, // Default to paper mode for safety
            active_panel: 0,
            active_symbol: "BTCUSDT".to_string(),

            show_buy_dialog: false,
            show_sell_dialog: false,
            order_qty_input: String::new(),
            order_price_input: String::new(),
            dialog_order_type: DialogOrderType::Market,

            chart_data: Vec::new(),
            volume_data: Vec::new(),
            chart_state: ChartState::default(),
            chart_stats: ChartStats {
                last_price: 0.0,
                high_price: 0.0,
                high_date: "".to_string(),
                low_price: 0.0,
                low_date: "".to_string(),
                average: 0.0,
                volume: 0.0,
                volume_smavg: 0.0,
                market_cap: 0.0,
                price_change: 0.0,
                price_change_pct: 0.0,
            },

            candles: Vec::new(),
            candle_state: CandlestickState::default(),

            watchlist: vec![
                WatchlistItem {
                    symbol: "BTCUSDT".into(),
                    name: "Bitcoin".into(),
                    price: 0.0,
                    change_pct: 0.0,
                },
                WatchlistItem {
                    symbol: "ETHUSDT".into(),
                    name: "Ethereum".into(),
                    price: 0.0,
                    change_pct: 0.0,
                },
                WatchlistItem {
                    symbol: "SOLUSDT".into(),
                    name: "Solana".into(),
                    price: 0.0,
                    change_pct: 0.0,
                },
                WatchlistItem {
                    symbol: "BNBUSDT".into(),
                    name: "BNB".into(),
                    price: 0.0,
                    change_pct: 0.0,
                },
            ],
            positions: Vec::new(),
            order_book: Vec::new(),
            news: VecDeque::new(),
            alerts: VecDeque::new(),
            exchanges: vec![ExchangeInfo {
                name: ExchangeName::CRYPTO,
                status: ExchangeStatus::Disconnected,
                latency_ms: 0.0,
                last_heartbeat: None,
            }],

            // Kill switch
            kill_switch_active: false,
            kill_switch_timestamp: None,
            kill_switch_orders_cancelled: 0,
            kill_switch_positions_closed: 0,

            // Dexter AI (enhanced)
            dexter_output: vec![
                "Revenue impact estimates — $44M in".to_string(),
                "showing revenue cooperates. +35% revenue".to_string(),
                "margin insert scenario, most L, 42% on".to_string(),
                "margin comparison 50% ≈ 34% on margin.".to_string(),
                "".to_string(),
                "Key valuation multiples:".to_string(),
                "  P/E: 10.53".to_string(),
                "  P/S: 2.98".to_string(),
                "  EV/EBITDA: 2.99".to_string(),
                "  DCF fair value range: $3.30 — $7.6B".to_string(),
            ],
            dexter_recommendation: Some("BUY".to_string()),
            dexter_loading: false,
            dexter_confidence: 0.74,
            // Every field below is empty until an analysis produces one.
            //
            // These were seeded with a complete fake thesis — "HIGH" conviction,
            // a 3.2% stop, a 4.2% Kelly size and a paragraph about
            // "institutional accumulation with RSI 62.4" — and NOTHING in the
            // program ever wrote them again. The panel therefore showed the
            // same invented trade idea for the whole session, on every symbol,
            // with no connection required.
            dexter_conviction: String::new(),
            dexter_stop_loss_pct: 0.0,
            dexter_take_profit_pct: 0.0,
            dexter_kelly_fraction: 0.0,
            dexter_position_size_pct: 0.0,
            dexter_rationale: String::new(),
            dexter_regime: String::new(),
            // False until an analysis actually passes the gate. A safety
            // indicator that starts green and is never set false is a light
            // that cannot warn.
            dexter_safety_gate_pass: false,
            dexter_call_count: 0,

            // No simulation has run, so there is no distribution to show.
            // `mirofish_running: true` was especially misleading: the panel
            // rendered as though a 5,000-agent swarm were live from the moment
            // the terminal opened.
            mirofish_running: false,
            mirofish_rally_pct: 0.0,
            mirofish_sideways_pct: 0.0,
            mirofish_dip_pct: 0.0,
            // Configuration, not a result — this one is a real setting.
            mirofish_agent_count: 5_000,
            mirofish_sim_time_ms: 0.0,
            mirofish_order_imbalance: 0.0,
            mirofish_simulated_vol: 0.0,
            mirofish_agent_agreement: 0.0,
            mirofish_bias_detected: false,

            // Trading
            //
            // A terminal that opens showing a $10.90 day P&L, $1,729.80 of
            // buying power, 15 orders and 12 fills is describing a trading
            // session that never happened — and none of these were written
            // again anywhere in the program, so the numbers stood for the
            // entire run. Zero is the only honest starting value; the daemon
            // link fills them in once there is one.
            day_pnl: 0.0,
            available_power: 0.0,
            orders_sent: 0,
            fills_count: 0,
            rejections_count: 0,

            // Session
            sequence_id: 0,
            session_start: std::time::Instant::now(),

            watchlist_scroll: 0,
            news_scroll: 0,
            orderbook_scroll: 0,
        }
    }

    // ── Chart controls ────────────────────────────────────────────────────────

    pub fn chart_zoom_in(&mut self) {
        self.chart_state.zoom_in();
        self.candle_state.zoom_in();
    }

    pub fn chart_zoom_out(&mut self) {
        self.chart_state.zoom_out();
        self.candle_state.zoom_out();
    }

    pub fn chart_scroll_left(&mut self) {
        self.chart_state.scroll_left();
        self.candle_state.scroll_left();
    }

    pub fn chart_scroll_right(&mut self) {
        self.chart_state.scroll_right();
        self.candle_state.scroll_right();
    }

    pub fn cycle_time_range(&mut self) {
        self.chart_state.cycle_time_range();
    }

    // ── Scrolling ─────────────────────────────────────────────────────────────

    pub fn scroll_up(&mut self) {
        match self.active_panel {
            0 => self.watchlist_scroll = self.watchlist_scroll.saturating_sub(1),
            2 => self.orderbook_scroll = self.orderbook_scroll.saturating_sub(1),
            _ => self.news_scroll = self.news_scroll.saturating_sub(1),
        }
    }

    pub fn scroll_down(&mut self) {
        match self.active_panel {
            0 => {
                if self.watchlist_scroll < self.watchlist.len().saturating_sub(1) {
                    self.watchlist_scroll += 1;
                }
            }
            2 => {
                if self.orderbook_scroll < self.order_book.len().saturating_sub(1) {
                    self.orderbook_scroll += 1;
                }
            }
            _ => self.news_scroll += 1,
        }
    }

    // ── Panel navigation ──────────────────────────────────────────────────────

    pub fn next_panel(&mut self) {
        self.active_panel = (self.active_panel + 1) % 6;
    }

    pub fn prev_panel(&mut self) {
        self.active_panel = if self.active_panel == 0 {
            5
        } else {
            self.active_panel - 1
        };
    }

    // ── Kill Switch ───────────────────────────────────────────────────────────

    /// Flag the kill switch locally.
    ///
    /// This halts nothing. The daemon link is receive-only, so pressing this
    /// cannot cancel an order or flatten a position anywhere — and the counts
    /// it used to report were invented twice over: `orders_cancelled` was set
    /// to the running order count, and `positions_closed` to however many rows
    /// the table happened to hold.
    ///
    /// "ALL TRADING HALTED" is the single most consequential thing this program
    /// can display. Someone reading it stops managing their exposure. It now
    /// says what it actually did.
    pub fn activate_kill_switch(&mut self) {
        self.kill_switch_active = true;
        let now = chrono::Local::now();
        self.kill_switch_timestamp = Some(now.format("%Y-%m-%d %H:%M:%S%.3f").to_string());
        // Nothing was cancelled or closed by this process.
        self.kill_switch_orders_cancelled = 0;
        self.kill_switch_positions_closed = 0;
        self.sequence_id += 1;
        self.push_alert_severity(
            "KILL SWITCH FLAGGED LOCALLY -- this TUI cannot halt trading. Stop the engine directly.",
            AlertSeverity::Critical,
        );
    }

    // ── Trading ───────────────────────────────────────────────────────────────

    pub fn push_alert(&mut self, text: &str) {
        self.alerts.push_front(AlertItem {
            text: text.to_string(),
            severity: AlertSeverity::Info,
        });
        if self.alerts.len() > 20 {
            self.alerts.pop_back();
        }
    }

    /// Report an action the TUI cannot perform, and say why.
    ///
    /// The daemon link at 127.0.0.1:7001 is **receive-only** — `main.rs` splits
    /// the socket and drops the writer (`let (mut reader, _writer) = ...`).
    /// There is no command protocol from this process to the engine, so no key
    /// press here can cancel an order, flatten a position or reach a broker.
    ///
    /// These actions previously pushed a success alert. "All pending orders
    /// cancelled" when nothing was cancelled is the most dangerous sentence
    /// this program can print: an operator reads it during a drawdown, believes
    /// their exposure is flat, and stops acting. Saying nothing would be better
    /// than that, and saying what is actually true is better still.
    fn push_unavailable(&mut self, action: &str) {
        self.push_alert_severity(
            &format!("{action} is not available: this TUI has no command channel to the engine."),
            AlertSeverity::Warning,
        );
    }

    pub fn push_alert_severity(&mut self, text: &str, severity: AlertSeverity) {
        self.alerts.push_front(AlertItem {
            text: text.to_string(),
            severity,
        });
        if self.alerts.len() > 20 {
            self.alerts.pop_back();
        }
    }

    pub fn open_buy_dialog(&mut self) {
        self.show_buy_dialog = true;
        self.show_sell_dialog = false;
        self.order_qty_input.clear();
        self.order_price_input.clear();
        self.dialog_order_type = DialogOrderType::Market;
        self.push_alert("Opening BUY dialog pane...");
    }

    pub fn open_sell_dialog(&mut self) {
        self.show_sell_dialog = true;
        self.show_buy_dialog = false;
        self.order_qty_input.clear();
        self.order_price_input.clear();
        self.dialog_order_type = DialogOrderType::Market;
        self.push_alert("Opening SELL dialog pane...");
    }

    pub fn cycle_order_type(&mut self) {
        self.dialog_order_type = self.dialog_order_type.next();
    }

    pub fn cancel_selected(&mut self) {
        self.push_unavailable("Cancelling an order");
    }

    pub fn cancel_all(&mut self) {
        self.push_unavailable("Cancelling all orders");
    }

    #[allow(dead_code)]
    pub fn halve_position(&mut self) {
        self.push_unavailable("Reducing a position");
    }

    pub fn close_full_position(&mut self) {
        self.push_unavailable("Closing a position");
    }

    /// Confirm the order dialog.
    ///
    /// Counts the attempt and closes the dialog, but does NOT claim the order
    /// was sent — see [`Self::push_unavailable`]. `orders_sent` is renamed in
    /// spirit if not in name: it is the number of submissions attempted from
    /// this screen, and the status line must not present it as fills.
    pub fn confirm_order(&mut self) {
        // The order details are echoed back so the operator can see what they
        // asked for, but the wording never says "submitted" - nothing was.
        // `orders_sent` is deliberately NOT incremented: a counter that climbs
        // for orders that were never placed turns one misleading alert into a
        // running total that looks like a position.
        let side = if self.show_buy_dialog {
            Some("BUY")
        } else if self.show_sell_dialog {
            Some("SELL")
        } else {
            None
        };

        match side {
            Some(side) => {
                let qty = self.order_qty_input.clone();
                let order_type = self.dialog_order_type.label();
                let symbol = self.active_symbol.clone();
                self.push_alert_severity(
                    &format!(
                        "NOT SENT - {side} {symbol} {qty} @ {order_type}: this TUI has no command channel to the engine."
                    ),
                    AlertSeverity::Critical,
                );
            }
            None => self.push_unavailable("Submitting an order"),
        }

        self.show_buy_dialog = false;
        self.show_sell_dialog = false;
    }

    pub fn dismiss_dialog(&mut self) {
        self.show_buy_dialog = false;
        self.show_sell_dialog = false;
    }

    // ── AI ─────────────────────────────────────────────────────────────────────

    /// Request a Dexter analysis.
    ///
    /// This used to print a fixed "BUY" at 74% confidence with invented
    /// valuation multiples (`P/E: 10.53`, `DCF fair value: $3.30`) for
    /// whatever symbol happened to be selected, under a comment saying it was
    /// simulating what a real async task would do.
    ///
    /// A trading terminal that shows a fabricated recommendation is worse than
    /// one that shows nothing: the numbers look like analysis, they are
    /// specific enough to act on, and nothing on screen says they are made up.
    ///
    /// The real implementation is `ai::dexter::analyse`, which calls an LLM and
    /// returns a `DexterSignal` — the `ValuationMetrics` struct there even
    /// carries the comment "Matches what you see in the TUI Dexter panel". It
    /// needs two things this process does not yet have: a `FusedContextLike`
    /// built from live engine state, and a configured model key. Until both
    /// exist, this reports that it cannot run.
    pub fn trigger_dexter(&mut self) {
        self.dexter_call_count += 1;
        self.dexter_loading = false;
        self.dexter_recommendation = None;
        self.dexter_confidence = 0.0;
        self.dexter_conviction = "—".to_string();
        self.dexter_safety_gate_pass = false;
        self.dexter_output = vec![
            "Dexter analysis is not wired up in this build.".to_string(),
            "".to_string(),
            "The analyst lives in crates/ai/src/dexter.rs and needs:".to_string(),
            "  - a fused context built from live engine state".to_string(),
            "  - a configured model key".to_string(),
            "".to_string(),
            "No recommendation is shown because none was produced.".to_string(),
        ];
        self.push_alert_severity(
            &format!(
                "Dexter analysis unavailable for {} - analyst not wired to this TUI.",
                self.active_symbol
            ),
            AlertSeverity::Warning,
        );
    }

    /// Request a Mirofish swarm simulation.
    ///
    /// Previously announced "simulation started" and immediately assigned a
    /// fixed 70/27/3 outcome with 72% agreement and an 847.3ms runtime, none
    /// of which came from running anything.
    ///
    /// A real engine exists — `swarm_sim::default_engine` with `step_round`
    /// and `run_forever`, and `ai::mirofish::run_swarm` above it. Driving it
    /// needs a market snapshot from live state and somewhere to run the rounds
    /// off the render thread. Until that is built, the panel reports no result
    /// rather than a plausible one.
    pub fn trigger_mirofish(&mut self) {
        self.mirofish_running = false;
        // Zeroed, not left at the previous values: a stale distribution from an
        // earlier press would read as this run's answer.
        self.mirofish_rally_pct = 0.0;
        self.mirofish_sideways_pct = 0.0;
        self.mirofish_dip_pct = 0.0;
        self.mirofish_agent_agreement = 0.0;
        self.mirofish_bias_detected = false;
        self.mirofish_sim_time_ms = 0.0;
        self.push_alert_severity(
            "Mirofish swarm is not wired to this TUI - no simulation was run.",
            AlertSeverity::Warning,
        );
    }

    pub fn cycle_confidence(&mut self) {
        let current = (self.dexter_confidence * 100.0) as u32;
        let next = match current {
            0..=59 => 60,
            60..=74 => 75,
            75..=89 => 90,
            _ => 60,
        };
        self.dexter_confidence = next as f64 / 100.0;
        self.push_alert(&format!("AI Confidence threshold cycled: {}%", next));
    }

    pub fn toggle_auto_trade(&mut self) {
        self.push_alert_severity(
            "Auto-trade mode TOGGLED. [!] Confirm with Ctrl+A again.",
            AlertSeverity::Warning,
        );
    }

    // ── Data ──────────────────────────────────────────────────────────────────

    /// Export the visible data.
    ///
    /// Named a destination file that was never created. Someone reading
    /// "Data exported to logs/export.csv" goes looking for it, finds nothing,
    /// and has no way to tell whether the export or their shell was at fault.
    pub fn export_csv(&mut self) {
        self.push_unavailable("Exporting to CSV");
    }

    pub fn run_backtest(&mut self) {
        // crates/backtest is real, but nothing here starts it.
        self.push_unavailable("Running a backtest");
    }

    #[allow(dead_code)]
    pub fn toggle_data_source(&mut self) {
        // The feed is whatever main.rs connected; nothing here switches it.
        self.push_unavailable("Switching the data source");
    }

    pub fn refresh_portfolio(&mut self) {
        // There is no broker REST client in this process.
        self.push_unavailable("Refreshing the portfolio");
    }

    // ── Live Data Ingestion ───────────────────────────────────────────────────

    /// Ingest a live kline/bar event into the candlestick chart.
    /// Called from the Binance WebSocket kline stream.
    pub fn push_candle(&mut self, open: f64, high: f64, low: f64, close: f64, volume: f64) {
        let time = self.candles.len() as f64;
        let candle = Candle {
            time,
            open,
            high,
            low,
            close,
            volume,
        };
        self.candles.push(candle);
        // Keep max 500 candles
        if self.candles.len() > 500 {
            self.candles.remove(0);
            // Re-index time values
            for (i, c) in self.candles.iter_mut().enumerate() {
                c.time = i as f64;
            }
        }

        // Also update chart stats
        self.chart_stats.last_price = close;
        if close > self.chart_stats.high_price || self.chart_stats.high_price == 0.0 {
            self.chart_stats.high_price = close;
        }
        if close < self.chart_stats.low_price || self.chart_stats.low_price == 0.0 {
            self.chart_stats.low_price = close;
        }
        self.chart_stats.volume += volume;
    }

    /// Update the last candle in-place (for in-progress kline bars).
    pub fn update_current_candle(
        &mut self,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    ) {
        if let Some(last) = self.candles.last_mut() {
            last.high = high;
            last.low = low;
            last.close = close;
            last.volume = volume;
        } else {
            self.push_candle(open, high, low, close, volume);
        }
        self.chart_stats.last_price = close;
    }

    /// Ingest a live trade tick — updates watchlist, chart line data, and stats.
    pub fn push_live_trade(&mut self, symbol: &str, price: f64, volume: f64) {
        // Update watchlist prices
        if let Some(item) = self.watchlist.iter_mut().find(|w| w.symbol == symbol) {
            let old_price = item.price;
            item.price = price;
            if old_price > 0.0 {
                item.change_pct = (price - old_price) / old_price * 100.0;
            }
        }

        // Update chart line data for active symbol
        if symbol == self.active_symbol {
            let next_x = self.chart_data.last().map(|(x, _)| *x + 1.0).unwrap_or(0.0);
            self.chart_data.push((next_x, price));
            if self.chart_data.len() > 2000 {
                self.chart_data.remove(0);
            }
            self.volume_data.push((next_x, volume));
            if self.volume_data.len() > 2000 {
                self.volume_data.remove(0);
            }

            self.chart_stats.last_price = price;
            if price > self.chart_stats.high_price || self.chart_stats.high_price == 0.0 {
                self.chart_stats.high_price = price;
            }
            if price < self.chart_stats.low_price || self.chart_stats.low_price == 0.0 {
                self.chart_stats.low_price = price;
            }
        }
    }

    /// Mark the Binance exchange as connected with latency.
    pub fn set_exchange_connected(&mut self, latency_ms: f64) {
        if let Some(ex) = self
            .exchanges
            .iter_mut()
            .find(|e| e.name == ExchangeName::CRYPTO)
        {
            ex.status = ExchangeStatus::Connected;
            ex.latency_ms = latency_ms;
            ex.last_heartbeat = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as i64,
            );
        }
    }

    // ── Session helpers ───────────────────────────────────────────────────────

    pub fn session_uptime(&self) -> String {
        let elapsed = self.session_start.elapsed();
        let secs = elapsed.as_secs();
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        format!("{:02}:{:02}:{:02}", h, m, s)
    }

    pub fn fill_ratio_str(&self) -> String {
        if self.fills_count + self.rejections_count == 0 {
            "—".to_string()
        } else {
            format!("{}/{}", self.fills_count, self.rejections_count)
        }
    }

    pub fn update_from_event(&mut self, event: common::events::BotEvent) {
        use common::events::BotEvent;
        self.sequence_id += 1;

        match event {
            BotEvent::MarketEvent {
                symbol,
                price,
                volume,
                event_type,
                ..
            } => {
                if let Some(item) = self.watchlist.iter_mut().find(|w| w.symbol == symbol) {
                    item.price = price;
                    item.change_pct = (price - 100.0) / 100.0;
                }

                if symbol == self.active_symbol && event_type == "trade" {
                    let next_x = self.chart_data.last().map(|(x, _)| *x + 1.0).unwrap_or(0.0);
                    self.chart_data.push((next_x, price));
                    if self.chart_data.len() > 2000 {
                        self.chart_data.remove(0);
                    }

                    if let Some(vol) = volume {
                        self.volume_data.push((next_x, vol));
                        if self.volume_data.len() > 2000 {
                            self.volume_data.remove(0);
                        }
                        self.chart_stats.volume += vol;
                    }

                    self.chart_stats.last_price = price;
                    if price > self.chart_stats.high_price || self.chart_stats.high_price == 0.0 {
                        self.chart_stats.high_price = price;
                    }
                    if price < self.chart_stats.low_price || self.chart_stats.low_price == 0.0 {
                        self.chart_stats.low_price = price;
                    }

                    let sum: f64 = self.chart_data.iter().map(|(_, p)| p).sum();
                    self.chart_stats.average = sum / self.chart_data.len() as f64;
                }
            }
            BotEvent::PositionUpdate { token, size, .. } => {
                if let Some(pos) = self.positions.iter_mut().find(|p| p.symbol == token) {
                    pos.holding = size;
                } else {
                    self.positions.push(PositionEntry {
                        symbol: token.to_string(),
                        holding: size,
                        pnl_pct: 0.0,
                    });
                }
            }
            BotEvent::WalletUpdate { sol_balance, .. } => {
                self.available_power = sol_balance;
            }
            BotEvent::AISignal {
                symbol,
                action,
                confidence,
                reason,
            } => {
                self.alerts.push_front(AlertItem {
                    text: format!(
                        "AI {} {} at {}% ({})",
                        action,
                        symbol,
                        (confidence * 100.0) as u32,
                        reason
                    ),
                    severity: AlertSeverity::Warning,
                });
                if self.alerts.len() > 20 {
                    self.alerts.pop_back();
                }
            }
            BotEvent::QuoteEvent {
                symbol,
                bid_price,
                bid_size,
                ask_price,
                ask_size,
                ..
            } => {
                if symbol == self.active_symbol {
                    if ask_size > 0 {
                        if !self.order_book.iter().any(|r| r.ask_price == ask_price) {
                            self.order_book.push(OrderBookRow {
                                ask_price,
                                ask_size,
                                ask_total: 0.0,
                                bid_price: 0.0,
                                bid_size: 0,
                                bid_total: 0.0,
                            });
                        } else if let Some(row) = self
                            .order_book
                            .iter_mut()
                            .find(|r| r.ask_price == ask_price)
                        {
                            row.ask_size = ask_size;
                        }
                    }

                    if bid_size > 0 {
                        if !self.order_book.iter().any(|r| r.bid_price == bid_price) {
                            self.order_book.push(OrderBookRow {
                                ask_price: 0.0,
                                ask_size: 0,
                                ask_total: 0.0,
                                bid_price,
                                bid_size,
                                bid_total: 0.0,
                            });
                        } else if let Some(row) = self
                            .order_book
                            .iter_mut()
                            .find(|r| r.bid_price == bid_price)
                        {
                            row.bid_size = bid_size;
                        }
                    }

                    let mut asks: Vec<_> = self
                        .order_book
                        .iter()
                        .filter(|r| r.ask_price > 0.0)
                        .map(|r| (r.ask_price, r.ask_size))
                        .collect();
                    asks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

                    let mut bids: Vec<_> = self
                        .order_book
                        .iter()
                        .filter(|r| r.bid_price > 0.0)
                        .map(|r| (r.bid_price, r.bid_size))
                        .collect();
                    bids.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

                    self.order_book.clear();
                    let rows = std::cmp::max(asks.len(), bids.len()).min(15);

                    let mut ask_cumulative = 0.0;
                    let mut bid_cumulative = 0.0;

                    for i in 0..rows {
                        let (ap, asz) = asks.get(i).copied().unwrap_or((0.0, 0));
                        let (bp, bsz) = bids.get(i).copied().unwrap_or((0.0, 0));

                        ask_cumulative += (asz as f64 * ap) / 1000.0;
                        bid_cumulative += (bsz as f64 * bp) / 1000.0;

                        self.order_book.push(OrderBookRow {
                            ask_price: ap,
                            ask_size: asz,
                            ask_total: ask_cumulative,
                            bid_price: bp,
                            bid_size: bsz,
                            bid_total: bid_cumulative,
                        });
                    }
                }
            }
            BotEvent::ExchangeHeartbeat {
                exchange,
                status,
                latency_ms,
            } => {
                let parsed_name = match exchange.as_str() {
                    "NYSE" => ExchangeName::NYSE,
                    "NASDAQ" => ExchangeName::NASDAQ,
                    "CME" => ExchangeName::CME,
                    "CBOE" => ExchangeName::CBOE,
                    "LSE" => ExchangeName::LSE,
                    "CRYPTO" => ExchangeName::CRYPTO,
                    "NSE" => ExchangeName::NSE,
                    "BSE" => ExchangeName::BSE,
                    _ => return,
                };

                let parsed_status = match status.as_str() {
                    "Connected" => ExchangeStatus::Connected,
                    "Degraded" => ExchangeStatus::Degraded,
                    "Disconnected" => ExchangeStatus::Disconnected,
                    _ => ExchangeStatus::Disabled,
                };

                if let Some(ex) = self.exchanges.iter_mut().find(|e| e.name == parsed_name) {
                    ex.status = parsed_status;
                    ex.latency_ms = latency_ms;
                    ex.last_heartbeat = Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as i64,
                    );
                }
            }
            _ => {}
        }
    }
}
