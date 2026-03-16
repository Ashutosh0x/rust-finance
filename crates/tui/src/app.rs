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

use common::models::exchange::{ExchangeInfo, ExchangeStatus, ExchangeName};

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

pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,
    pub connection_status: String,
    pub show_help: bool,
    pub paper_mode: bool,
    pub active_panel: u8,

    pub active_symbol: String,
    
    // Dialogs
    pub show_buy_dialog: bool,
    pub show_sell_dialog: bool,
    pub order_qty_input: String,
    pub order_price_input: String,

    // Chart
    pub chart_data: Vec<(f64, f64)>,
    pub volume_data: Vec<(f64, f64)>,
    pub chart_state: ChartState,
    pub chart_stats: ChartStats,

    // Live Data
    pub watchlist: Vec<WatchlistItem>,
    pub positions: Vec<PositionEntry>,
    pub order_book: Vec<OrderBookRow>,
    pub news: VecDeque<NewsItem>,
    pub alerts: VecDeque<AlertItem>,

    // Exchanges
    pub exchanges: Vec<ExchangeInfo>,

    // Dexter AI
    pub dexter_output: Vec<String>,
    pub dexter_recommendation: Option<String>,

    // Mirofish
    pub mirofish_running: bool,
    pub mirofish_rally_pct: f64,
    pub mirofish_sideways_pct: f64,
    pub mirofish_dip_pct: f64,

    // Trading
    pub day_pnl: f64,
    pub available_power: f64,

    // Scroll states
    pub watchlist_scroll: usize,
    pub news_scroll: usize,
    pub orderbook_scroll: usize,
}

impl App {
    pub fn new(initial_screen: AppScreen) -> Self {
        Self {
            screen: initial_screen,
            should_quit: false,
            connection_status: "Connecting...".to_string(),
            show_help: false,
            paper_mode: false,
            active_panel: 0,
            active_symbol: "AAPL".to_string(),
            
            show_buy_dialog: false,
            show_sell_dialog: false,
            order_qty_input: String::new(),
            order_price_input: String::new(),

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
            },

            watchlist: default_watchlist(),
            positions: default_positions(),
            order_book: default_order_book(),
            news: default_news(),
            alerts: default_alerts(),
            exchanges: default_exchanges(),

            dexter_output: vec![
                "Revenue impact estimates - $44 mi in".to_string(),
                "showing revenue cooperates. +35% revenue".to_string(),
                "margin insert scanue, moast L, 42% on".to_string(),
                "margin comparison 50% ≈ 34% on margin.".to_string(),
                "".to_string(),
                "Key valuation multiples:".to_string(),
                "- P/E: 10.53".to_string(),
                "- P/S: 2.98".to_string(),
                "- EV/EBITDA: 2.99".to_string(),
                "- DCF fair value range: $3.30 - $7.6B".to_string(),
            ],
            dexter_recommendation: Some("BUY".to_string()),

            mirofish_running: true,
            mirofish_rally_pct: 77.0,
            mirofish_sideways_pct: 30.0,
            mirofish_dip_pct: 0.0,

            day_pnl: 10.90,
            available_power: 1729.8,

            watchlist_scroll: 0,
            news_scroll: 0,
            orderbook_scroll: 0,
        }
    }

    // ── Chart controls ────────────────────────────────────────────────────────

    pub fn chart_zoom_in(&mut self) {
        self.chart_state.zoom_in();
    }

    pub fn chart_zoom_out(&mut self) {
        self.chart_state.zoom_out();
    }

    pub fn chart_scroll_left(&mut self) {
        self.chart_state.scroll_left();
    }

    pub fn chart_scroll_right(&mut self) {
        self.chart_state.scroll_right();
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
        self.active_panel = if self.active_panel == 0 { 5 } else { self.active_panel - 1 };
    }

    // ── Trading ───────────────────────────────────────────────────────────────

    pub fn push_alert(&mut self, text: &str) {
        self.alerts.push_front(AlertItem { text: text.to_string(), severity: AlertSeverity::Info });
        if self.alerts.len() > 20 { self.alerts.pop_back(); }
    }

    pub fn open_buy_dialog(&mut self) {
        self.show_buy_dialog = true;
        self.show_sell_dialog = false;
        self.order_qty_input.clear();
        self.order_price_input.clear();
        self.push_alert("Opening BUY dialog pane...");
    }

    pub fn open_sell_dialog(&mut self) {
        self.show_sell_dialog = true;
        self.show_buy_dialog = false;
        self.order_qty_input.clear();
        self.order_price_input.clear();
        self.push_alert("Opening SELL dialog pane...");
    }

    pub fn cancel_selected(&mut self) {
        self.push_alert("Selected order cancelled.");
    }

    pub fn cancel_all(&mut self) {
        self.push_alert("WARNING: All pending orders cancelled.");
    }

    pub fn halve_position(&mut self) {
        self.push_alert("Position halved. Routing market sell for 50%.");
    }

    pub fn close_full_position(&mut self) {
        self.push_alert("Position closed completely.");
    }

    pub fn confirm_order(&mut self) {
        if self.show_buy_dialog {
             self.push_alert(&format!("BUY Order submitted for {} at {} qty", self.active_symbol, self.order_qty_input));
        } else if self.show_sell_dialog {
             self.push_alert(&format!("SELL Order submitted for {} at {} qty", self.active_symbol, self.order_qty_input));
        } else {
             self.push_alert("Order submitted to execution engine.");
        }
        self.show_buy_dialog = false;
        self.show_sell_dialog = false;
    }

    pub fn dismiss_dialog(&mut self) {
        self.show_buy_dialog = false;
        self.show_sell_dialog = false;
        self.push_alert("Dialog dismissed.");
    }

    // ── AI ─────────────────────────────────────────────────────────────────────

    pub fn trigger_dexter(&mut self) {
        self.push_alert("Dexter Analysis requested...");
        self.dexter_output = vec!["Analyzing market conditions...".to_string()];
        self.dexter_recommendation = None;
    }

    pub fn trigger_mirofish(&mut self) {
        self.push_alert("Mirofish 5,000-agent simulation started.");
        self.mirofish_running = true;
        self.mirofish_rally_pct = 20.0;
        self.mirofish_sideways_pct = 70.0;
        self.mirofish_dip_pct = 10.0;
    }

    pub fn cycle_confidence(&mut self) {
        self.push_alert("AI Confidence threshold cycled: Strong 80%.");
    }

    pub fn toggle_auto_trade(&mut self) {
        self.push_alert("Auto-trade mode TOGGLED.");
    }

    // ── Data ──────────────────────────────────────────────────────────────────

    pub fn export_csv(&mut self) {
        self.push_alert("Data exported to logs/export.csv");
    }

    pub fn run_backtest(&mut self) {
        self.push_alert("Backtest engine warming up...");
    }

    pub fn toggle_data_source(&mut self) {
        self.push_alert("Switched data source (Mock <-> Live)");
    }

    pub fn refresh_portfolio(&mut self) {
        self.push_alert("Refreshing portfolio via broker REST API...");
    }

    pub fn update_from_event(&mut self, event: common::events::BotEvent) {
        use common::events::BotEvent;
        match event {
            BotEvent::MarketEvent { symbol, price, volume, event_type, .. } => {
                // Update watchlist
                if let Some(item) = self.watchlist.iter_mut().find(|w| w.symbol == symbol) {
                    item.price = price;
                    // Mock change pct for demo
                    item.change_pct = (price - 100.0) / 100.0;
                }
                
                // Update chart if it's the active symbol
                if symbol == self.active_symbol && event_type == "trade" {
                    let next_x = self.chart_data.last().map(|(x, _)| *x + 1.0).unwrap_or(0.0);
                    self.chart_data.push((next_x, price));
                    if self.chart_data.len() > 2000 {
                        self.chart_data.remove(0); // rolling window
                    }
                    
                    if let Some(vol) = volume {
                        self.volume_data.push((next_x, vol));
                        if self.volume_data.len() > 2000 {
                            self.volume_data.remove(0);
                        }
                        self.chart_stats.volume += vol; // running daily volume total
                    }
                    
                    // Dynamically update stats
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
            BotEvent::AISignal { symbol, action, confidence, reason } => {
                self.alerts.push_front(AlertItem {
                    text: format!("AI {} {} at {}% ({})", action, symbol, (confidence * 100.0) as u32, reason),
                    severity: AlertSeverity::Warning,
                });
                if self.alerts.len() > 20 {
                    self.alerts.pop_back();
                }
            }
            BotEvent::QuoteEvent { symbol, bid_price, bid_size, ask_price, ask_size, .. } => {
                if symbol == self.active_symbol {
                    // We only get top-of-book from basic endpoints usually, 
                    // but for a dynamic order book simulation, let's treat it as an order book update.
                    
                    // Add/update Ask side (Sort Ascending)
                    if ask_size > 0 {
                        if !self.order_book.iter().any(|r| r.ask_price == ask_price) {
                            self.order_book.push(OrderBookRow {
                                ask_price, ask_size, ask_total: 0.0,
                                bid_price: 0.0, bid_size: 0, bid_total: 0.0,
                            });
                        } else if let Some(row) = self.order_book.iter_mut().find(|r| r.ask_price == ask_price) {
                            row.ask_size = ask_size;
                        }
                    }
                    
                    // Add/update Bid side (Sort Descending)
                    if bid_size > 0 {
                        if !self.order_book.iter().any(|r| r.bid_price == bid_price) {
                             self.order_book.push(OrderBookRow {
                                ask_price: 0.0, ask_size: 0, ask_total: 0.0,
                                bid_price, bid_size, bid_total: 0.0,
                            });
                        } else if let Some(row) = self.order_book.iter_mut().find(|r| r.bid_price == bid_price) {
                            row.bid_size = bid_size;
                        }
                    }
                    
                    // Sort order book: We want lowest asks at top, highest bids at top
                    // A proper L2 book splits bids and asks into two lists, but since the TUI 
                    // merges them into one row-based table, we just sort by price proximity to mid
                    let _mid_price = (ask_price + bid_price) / 2.0;
                    
                    // Best asks ascending
                    let mut asks: Vec<_> = self.order_book.iter().filter(|r| r.ask_price > 0.0).map(|r| (r.ask_price, r.ask_size)).collect();
                    asks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                    
                    // Best bids descending
                    let mut bids: Vec<_> = self.order_book.iter().filter(|r| r.bid_price > 0.0).map(|r| (r.bid_price, r.bid_size)).collect();
                    bids.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                    
                    self.order_book.clear();
                    let rows = std::cmp::max(asks.len(), bids.len()).min(15);
                    
                    let mut ask_cumulative = 0.0;
                    let mut bid_cumulative = 0.0;
                    
                    for i in 0..rows {
                        let (ap, asz) = asks.get(i).copied().unwrap_or((0.0, 0));
                        let (bp, bsz) = bids.get(i).copied().unwrap_or((0.0, 0));
                        
                        ask_cumulative += (asz as f64 * ap) / 1000.0; // In thousands
                        bid_cumulative += (bsz as f64 * bp) / 1000.0;
                        
                        self.order_book.push(OrderBookRow {
                            ask_price: ap, ask_size: asz, ask_total: ask_cumulative,
                            bid_price: bp, bid_size: bsz, bid_total: bid_cumulative,
                        });
                    }
                }
            }
            BotEvent::ExchangeHeartbeat { exchange, status, latency_ms } => {
                let parsed_name = match exchange.as_str() {
                    "NYSE" => ExchangeName::NYSE,
                    "NASDAQ" => ExchangeName::NASDAQ,
                    "CME" => ExchangeName::CME,
                    "CBOE" => ExchangeName::CBOE,
                    "LSE" => ExchangeName::LSE,
                    "CRYPTO" => ExchangeName::CRYPTO,
                    "NSE" => ExchangeName::NSE,
                    "BSE" => ExchangeName::BSE,
                    _ => return, // Unknown exchange
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
                    ex.last_heartbeat = Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64);
                }
            }
            _ => {}
        }
    }
}

// ── Default data generators ───────────────────────────────────────────────────

fn default_watchlist() -> Vec<WatchlistItem> {
    vec![
        WatchlistItem { symbol: "AAPL".into(), name: "Apple Inc.".into(), price: 322.50, change_pct: 1.58 },
        WatchlistItem { symbol: "NVDA".into(), name: "NVIDIA Corp.".into(), price: 297.75, change_pct: 0.32 },
        WatchlistItem { symbol: "TSLA".into(), name: "Tesla Inc.".into(), price: 103.35, change_pct: -0.31 },
        WatchlistItem { symbol: "AMZN".into(), name: "Amazon.com Inc.".into(), price: 83.50, change_pct: -0.17 },
        WatchlistItem { symbol: "MSFT".into(), name: "Microsoft Corp.".into(), price: 119.50, change_pct: -1.27 },
        WatchlistItem { symbol: "GOOG".into(), name: "Alphabet Inc.".into(), price: 223.90, change_pct: -0.79 },
        WatchlistItem { symbol: "META".into(), name: "Meta Platforms".into(), price: 52.55, change_pct: -0.33 },
        WatchlistItem { symbol: "NFLX".into(), name: "Netflix Inc.".into(), price: 308.83, change_pct: -0.32 },
        WatchlistItem { symbol: "AMD".into(), name: "AMD Inc.".into(), price: 111.93, change_pct: 1.22 },
        WatchlistItem { symbol: "INTC".into(), name: "Intel Corp.".into(), price: 52.27, change_pct: 0.12 },
        WatchlistItem { symbol: "CRM".into(), name: "Salesforce Inc.".into(), price: 275.19, change_pct: 0.12 },
        WatchlistItem { symbol: "ORCL".into(), name: "Oracle Corp.".into(), price: 38.20, change_pct: -0.30 },
        WatchlistItem { symbol: "UBER".into(), name: "Uber Tech.".into(), price: 135.15, change_pct: -0.38 },
    ]
}

fn default_positions() -> Vec<PositionEntry> {
    vec![
        PositionEntry { symbol: "AAPL".into(), holding: 222.50, pnl_pct: 1.72 },
        PositionEntry { symbol: "NVDA".into(), holding: 100.00, pnl_pct: 1.53 },
        PositionEntry { symbol: "NVDA".into(), holding: 50.00, pnl_pct: 1.15 },
        PositionEntry { symbol: "TSLA".into(), holding: 0.00, pnl_pct: -0.23 },
        PositionEntry { symbol: "CNBC".into(), holding: -10.00, pnl_pct: -0.27 },
    ]
}

fn default_order_book() -> Vec<OrderBookRow> {
    vec![
        OrderBookRow { ask_price: 7871.71, ask_size: 100, ask_total: 2382.0, bid_price: 7871.70, bid_size: 300, bid_total: 10033.0 },
        OrderBookRow { ask_price: 7871.70, ask_size: 100, ask_total: 2543.0, bid_price: 7871.69, bid_size: 200, bid_total: 9893.0 },
        OrderBookRow { ask_price: 7871.70, ask_size: 120, ask_total: 1592.0, bid_price: 7871.68, bid_size: 300, bid_total: 4083.0 },
        OrderBookRow { ask_price: 7871.70, ask_size: 100, ask_total: 1193.0, bid_price: 7871.68, bid_size: 200, bid_total: 3283.0 },
        OrderBookRow { ask_price: 7871.80, ask_size: 100, ask_total: 1213.0, bid_price: 7871.67, bid_size: 1000, bid_total: 3132.0 },
        OrderBookRow { ask_price: 7871.80, ask_size: 360, ask_total: 2133.0, bid_price: 7871.66, bid_size: 1000, bid_total: 4282.0 },
        OrderBookRow { ask_price: 7871.90, ask_size: 80, ask_total: 593.0, bid_price: 7871.65, bid_size: 400, bid_total: 3282.0 },
    ]
}

fn default_news() -> VecDeque<NewsItem> {
    let items = vec![
        NewsItem { source: "Reuters".into(), time_ago: "25m ago".into(), headline: "Apple reports record quarterly earnings driven by strong iPhone 16 sales across global markets".into() },
        NewsItem { source: "Bloomberg".into(), time_ago: "2m ago".into(), headline: "NVIDIA announces next-gen Blackwell GPU architecture with 2x AI inference performance gains".into() },
        NewsItem { source: "Bloomberg".into(), time_ago: "19m ago".into(), headline: "Reuters: Federal Reserve signals potential rate cuts in upcoming September FOMC meeting".into() },
        NewsItem { source: "WSJ".into(), time_ago: "19m ago".into(), headline: "Tesla's autonomous driving milestone - FSD v13 achieves breakthrough in urban navigation".into() },
        NewsItem { source: "WSJ".into(), time_ago: "5m ago".into(), headline: "Tech sector rally continues as institutional investors increase allocation to megacap stocks".into() },
        NewsItem { source: "CNBC".into(), time_ago: "1m ago".into(), headline: "Market update: S&P 500 hits new all-time high on strong economic data and earnings beat".into() },
    ];
    VecDeque::from(items)
}

fn default_alerts() -> VecDeque<AlertItem> {
    let items = vec![
        AlertItem { text: "EV subsidy catalyst detected - TSLA".into(), severity: AlertSeverity::Info },
        AlertItem { text: "Earnings beat expected - AAPL Q4 +12%".into(), severity: AlertSeverity::Info },
        AlertItem { text: "Unusual options activity - NVDA $350C".into(), severity: AlertSeverity::Warning },
        AlertItem { text: "Overbought RSI 78.4 - NVDA".into(), severity: AlertSeverity::Warning },
        AlertItem { text: "Volume spike 3.2x avg - AMD".into(), severity: AlertSeverity::Info },
        AlertItem { text: "Support level test - META $50.00".into(), severity: AlertSeverity::Critical },
        AlertItem { text: "Bullish MACD crossover - GOOG".into(), severity: AlertSeverity::Info },
        AlertItem { text: "Institutional accumulation - MSFT".into(), severity: AlertSeverity::Info },
    ];
    VecDeque::from(items)
}

fn default_exchanges() -> Vec<ExchangeInfo> {
    vec![
        ExchangeInfo { name: ExchangeName::NYSE, status: ExchangeStatus::Connected, latency_ms: 12.5, last_heartbeat: None },
        ExchangeInfo { name: ExchangeName::NASDAQ, status: ExchangeStatus::Connected, latency_ms: 15.2, last_heartbeat: None },
        ExchangeInfo { name: ExchangeName::CME, status: ExchangeStatus::Connected, latency_ms: 8.4, last_heartbeat: None },
        ExchangeInfo { name: ExchangeName::CBOE, status: ExchangeStatus::Degraded, latency_ms: 250.0, last_heartbeat: None },
        ExchangeInfo { name: ExchangeName::LSE, status: ExchangeStatus::Disabled, latency_ms: 0.0, last_heartbeat: None },
        ExchangeInfo { name: ExchangeName::CRYPTO, status: ExchangeStatus::Connected, latency_ms: 45.1, last_heartbeat: None },
    ]
}
