// ============================================================
// crates/tui/src/state.rs
//
// AppState — the single source of truth for the TUI.
// Updated by the daemon via broadcast channel.
// Read by the render loop every frame.
// Uses Arc<RwLock<>> for safe sharing across Tokio tasks.
// ============================================================

use chrono::{DateTime, Utc};
use std::collections::VecDeque;

use ai::dexter::DexterSignal;
use swarm_sim::engine::SwarmStep;

/// The complete TUI application state.
/// Cloned once per frame — keep it cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct AppState {
    // ── Market data ───────────────────────────────────────────────
    pub selected_symbol: Option<String>,
    pub price_history: Vec<f64>, // last 500 prices for chart
    pub sp500_price: f64,
    pub nasdaq_price: f64,
    pub latency_ms: f64,
    pub watchlist: Vec<WatchlistItem>,
    pub order_book: OrderBookState,
    pub positions: Vec<PositionItem>,
    pub news: Vec<NewsItem>,

    // ── AI signals ────────────────────────────────────────────────
    pub dexter_signal: Option<DexterSignal>,
    pub dexter_alerts: VecDeque<Alert>,
    pub swarm_step: Option<SwarmStep>,

    // ── Order entry ───────────────────────────────────────────────
    pub order_entry: OrderEntryState,

    // ── Session stats ─────────────────────────────────────────────
    pub rust_version: String,
    pub active_threads: usize,
    pub dexter_call_count: usize,
    pub swarm_active_agents: usize,
    pub orders_sent: usize,
    pub fill_rejection_ratio: String,
    pub session_uptime_min: f64,
    pub connected_venues: Vec<String>,

    // ── Polymarket state ──────────────────────────────────────────
    pub polymarket: PolymarketState,
}

#[derive(Debug, Clone, Default)]
pub struct PolymarketState {
    pub markets: Vec<PolymarketMarket>,
}

#[derive(Debug, Clone)]
pub struct PolymarketMarket {
    pub question: String,
    pub yes_price: f64,
    pub no_price: f64,
    pub volume_24hr: f64,
}

#[derive(Debug, Clone)]
pub struct WatchlistItem {
    pub symbol: String,
    pub name: String,
    pub price: f64,
    pub change_pct: f64,
}

#[derive(Debug, Clone, Default)]
pub struct OrderBookState {
    pub asks: Vec<BookLevel>,
    pub bids: Vec<BookLevel>,
    pub imbalance: f64,
}

#[derive(Debug, Clone)]
pub struct BookLevel {
    pub price: f64,
    pub size: u32,
    pub total: f64,
}

#[derive(Debug, Clone)]
pub struct PositionItem {
    pub symbol: String,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub quantity: f64,
    pub side: String,
}

#[derive(Debug, Clone)]
pub struct NewsItem {
    pub headline: String,
    pub source: String,
    pub age_minutes: u32,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub message: String,
    pub severity: String, // "buy" | "risk" | "info"
    pub symbol: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct OrderEntryState {
    pub symbol: String,
    pub quantity: u32,
    pub price_str: String,
    pub order_type: OrderType,
}

#[derive(Debug, Clone, Default)]
pub enum OrderType {
    #[default]
    Limit,
    Market,
    Stop,
    ImmediateOrCancel,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            rust_version: "6.19".to_string(),
            active_threads: 1,
            fill_rejection_ratio: "0".to_string(),
            order_entry: OrderEntryState {
                symbol: "".to_string(),
                quantity: 0,
                price_str: "".to_string(),
                order_type: OrderType::Limit,
            },
            watchlist: vec![],
            positions: vec![],
            news: vec![],
            connected_venues: vec![],
            ..Default::default()
        }
    }

    /// Push a new Dexter alert, capped at 50 items
    pub fn push_alert(&mut self, message: String, severity: String, symbol: String) {
        self.dexter_alerts.push_back(Alert {
            message,
            severity,
            symbol,
            timestamp: Utc::now(),
        });
        while self.dexter_alerts.len() > 50 {
            self.dexter_alerts.pop_front();
        }
    }

    /// Update from a live market event
    pub fn update_market(&mut self, symbol: &str, price: f64, change_pct: f64) {
        // Update watchlist
        if let Some(item) = self.watchlist.iter_mut().find(|w| w.symbol == symbol) {
            item.price = price;
            item.change_pct = change_pct;
        }

        // Update price history if this is the selected symbol
        if self.selected_symbol.as_deref() == Some(symbol) {
            self.price_history.push(price);
            if self.price_history.len() > 500 {
                self.price_history.remove(0);
            }
        }
    }

    /// Update order book state
    pub fn update_order_book(&mut self, asks: Vec<BookLevel>, bids: Vec<BookLevel>) {
        let bid_vol: f64 = bids.iter().take(5).map(|b| b.total).sum();
        let ask_vol: f64 = asks.iter().take(5).map(|a| a.total).sum();
        let imbalance = if bid_vol + ask_vol > 0.0 {
            (bid_vol - ask_vol) / (bid_vol + ask_vol)
        } else {
            0.0
        };

        self.order_book = OrderBookState {
            asks,
            bids,
            imbalance,
        };
    }
}
