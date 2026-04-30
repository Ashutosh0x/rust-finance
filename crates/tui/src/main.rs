#![forbid(unsafe_code)]
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, text::Line, widgets::*};
use std::{io, time::Duration};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

// ── Color Palette (production-grade dark theme) ──────────────────────────────
const BG: Color = Color::Rgb(10, 12, 15);
const BG_ELEVATED: Color = Color::Rgb(18, 22, 28);
const BORDER: Color = Color::Rgb(30, 37, 48);
const BORDER_ACTIVE: Color = Color::Rgb(59, 130, 246);
const TEXT_PRIMARY: Color = Color::Rgb(226, 232, 240);
const TEXT_SECONDARY: Color = Color::Rgb(148, 163, 184);
const TEXT_DIM: Color = Color::Rgb(80, 90, 100);
const GREEN: Color = Color::Rgb(74, 222, 128);
const RED: Color = Color::Rgb(248, 113, 113);
const RED_KILL: Color = Color::Rgb(180, 0, 0);
const ORANGE: Color = Color::Rgb(249, 115, 22);
const BLUE: Color = Color::Rgb(96, 165, 250);
const PURPLE: Color = Color::Rgb(167, 139, 250);
const CYAN: Color = Color::Rgb(0, 210, 220);
const AMBER: Color = Color::Rgb(240, 180, 0);
const YELLOW: Color = Color::Rgb(250, 204, 21);

mod app;
mod event_handler;
pub mod layout;
mod live_feed;
pub mod setup;
pub mod state;
pub mod widgets;

use app::App;
use common::models::exchange::ExchangeStatus;
use live_feed::{spawn_binance_feed, LiveFeedEvent};
use widgets::candlestick_widget::render_candlestick_chart;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        crossterm::event::EnableMouseCapture,
        EnterAlternateScreen
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let _ = dotenvy::dotenv();

    let needs_setup = std::env::var("FINNHUB_API_KEY").is_err()
        || std::env::var("ALPACA_API_KEY").is_err()
        || std::env::var("ALPACA_SECRET_KEY").is_err();

    let force_setup = std::env::args().any(|a| a == "--setup");

    let initial_screen = if needs_setup || force_setup {
        crate::app::AppScreen::Setup(crate::app::SetupState::new())
    } else {
        crate::app::AppScreen::Dashboard
    };

    let mut app = App::new(initial_screen);

    // ── Daemon Event Bus (legacy) ─────────────────────────────────────────
    let (tx_status, mut rx_status) = mpsc::channel::<String>(100);
    let (tx_event, mut rx_event) = mpsc::channel::<common::events::BotEvent>(1000);

    let tx_status_clone = tx_status.clone();

    tokio::spawn(async move {
        loop {
            match TcpStream::connect("127.0.0.1:7001").await {
                Ok(stream) => {
                    let _ = tx_status_clone
                        .send("Connected to Daemon (127.0.0.1:7001/binary)".to_string())
                        .await;
                    let (mut reader, _writer) = tokio::io::split(stream);
                    let mut length_buf = [0u8; 4];

                    loop {
                        if reader.read_exact(&mut length_buf).await.is_err() {
                            break;
                        }
                        let len = u32::from_le_bytes(length_buf) as usize;
                        if len > 1024 * 1024 {
                            break;
                        }

                        let mut buf = vec![0u8; len];
                        if reader.read_exact(&mut buf).await.is_err() {
                            break;
                        }

                        if let Ok(event) = postcard::from_bytes::<common::events::BotEvent>(&buf) {
                            let _ = tx_event.send(event).await;
                        }
                    }
                    let _ = tx_status_clone
                        .send("Daemon Disconnected. Reconnecting...".to_string())
                        .await;
                }
                Err(_) => {
                    let _ = tx_status_clone
                        .send("Connection Failed. Retrying...".to_string())
                        .await;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    // ── Direct Binance WebSocket Feed (real-time) ─────────────────────────
    let (tx_feed, mut rx_feed) = mpsc::channel::<LiveFeedEvent>(5000);
    spawn_binance_feed(tx_feed);

    loop {
        // Drain daemon events
        while let Ok(msg) = rx_status.try_recv() {
            app.connection_status = msg;
        }
        while let Ok(event) = rx_event.try_recv() {
            app.update_from_event(event);
        }

        // Drain live Binance feed events (capped at 500/frame to prevent UI freeze)
        let mut feed_budget = 500;
        while feed_budget > 0 {
            let feed_event = match rx_feed.try_recv() {
                Ok(e) => e,
                Err(_) => break,
            };
            feed_budget -= 1;
            match feed_event {
                LiveFeedEvent::Kline {
                    symbol,
                    open,
                    high,
                    low,
                    close,
                    volume,
                    is_closed,
                } => {
                    if symbol == app.active_symbol {
                        if is_closed {
                            // Bar closed — push a new finalized candle
                            app.push_candle(open, high, low, close, volume);
                        } else {
                            // Bar still forming — update the current candle in-place
                            app.update_current_candle(open, high, low, close, volume);
                        }
                    }
                }
                LiveFeedEvent::Trade {
                    symbol,
                    price,
                    quantity,
                } => {
                    app.push_live_trade(&symbol, price, quantity);
                }
                LiveFeedEvent::BookTicker {
                    symbol,
                    bid_price,
                    bid_size,
                    ask_price,
                    ask_size,
                } => {
                    if symbol == app.active_symbol {
                        // Update the top-of-book in the order book
                        if app.order_book.is_empty() {
                            app.order_book.push(crate::app::OrderBookRow {
                                ask_price,
                                ask_size: ask_size as u64,
                                ask_total: ask_price * ask_size,
                                bid_price,
                                bid_size: bid_size as u64,
                                bid_total: bid_price * bid_size,
                            });
                        } else {
                            app.order_book[0] = crate::app::OrderBookRow {
                                ask_price,
                                ask_size: ask_size as u64,
                                ask_total: ask_price * ask_size,
                                bid_price,
                                bid_size: bid_size as u64,
                                bid_total: bid_price * bid_size,
                            };
                        }
                    }
                    app.set_exchange_connected(0.5);
                }
                LiveFeedEvent::Status(msg) => {
                    app.connection_status = msg;
                }
            }
        }

        terminal.draw(|f| match &app.screen {
            crate::app::AppScreen::Setup(state) => crate::setup::render_setup(f, state),
            crate::app::AppScreen::Dashboard => ui(f, &app),
        })?;

        if crossterm::event::poll(Duration::from_millis(16))? {
            // ~60 FPS
            match event::read()? {
                Event::Key(key) => match &mut app.screen {
                    crate::app::AppScreen::Setup(state) => {
                        match crate::setup::handle_setup_key(key, state) {
                            crate::setup::SetupAction::Submit => {
                                let pairs: Vec<(&str, &str)> = state
                                    .fields
                                    .iter()
                                    .map(|f| (f.name, f.value.as_str()))
                                    .collect();

                                match common::env_writer::save_keys(&pairs) {
                                    Ok(()) => {
                                        use zeroize::Zeroize;
                                        for field in &mut state.fields {
                                            field.value.zeroize();
                                        }
                                        app.screen = crate::app::AppScreen::Dashboard;
                                    }
                                    Err(e) => {
                                        state.error_msg =
                                            Some(format!("Failed to save .env: {}", e));
                                    }
                                }
                            }
                            crate::setup::SetupAction::Quit => break,
                            crate::setup::SetupAction::Continue => {}
                        }
                    }
                    crate::app::AppScreen::Dashboard => {
                        event_handler::handle_key(&mut app, key);
                    }
                },
                Event::Mouse(mouse_event) => {
                    if let crate::app::AppScreen::Dashboard = &app.screen {
                        event_handler::handle_mouse(&mut app, mouse_event);
                    }
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// RENDERING
// ═══════════════════════════════════════════════════════════════════════════════

fn ui(f: &mut Frame, app: &App) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Top Bar
            Constraint::Min(0),    // Main Content
            Constraint::Length(1), // Bottom Bar
        ])
        .split(f.area());

    draw_top_bar(f, main_layout[0], app);
    draw_main_content(f, main_layout[1], app);
    draw_bottom_bar(f, main_layout[2], app);

    // ── Overlays (Z-ordered) ──────────────────────────────────────────────
    if app.show_help {
        crate::widgets::help_overlay::render_help_overlay(f);
    }

    if app.show_buy_dialog || app.show_sell_dialog {
        draw_dialog(f, app);
    }

    if app.kill_switch_active {
        draw_kill_switch_overlay(f, app);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// KILL SWITCH OVERLAY
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_kill_switch_overlay(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Clear, area);

    // Full-screen red background
    let bg_block = Block::default().style(Style::default().bg(RED_KILL));
    f.render_widget(bg_block, area);

    let center = centered_rect(70, 60, area);
    f.render_widget(Clear, center);

    let inner_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(RED).add_modifier(Modifier::BOLD))
        .border_type(BorderType::Double)
        .style(Style::default().bg(Color::Rgb(30, 0, 0)));

    let inner = inner_block.inner(center);
    f.render_widget(inner_block, center);

    let timestamp = app.kill_switch_timestamp.as_deref().unwrap_or("UNKNOWN");

    let text = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "  ██╗  ██╗██╗██╗     ██╗     ",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "  ██║ ██╔╝██║██║     ██║     ",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "  █████╔╝ ██║██║     ██║     ",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "  ██╔═██╗ ██║██║     ██║     ",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "  ██║  ██╗██║███████╗███████╗ ",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "  ╚═╝  ╚═╝╚═╝╚══════╝╚══════╝",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  !!!  EMERGENCY HALT -- ALL TRADING SUSPENDED  !!!  ",
            Style::default()
                .fg(YELLOW)
                .add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Timestamp:       ", Style::default().fg(TEXT_DIM)),
            Span::styled(timestamp, Style::default().fg(TEXT_PRIMARY)),
        ]),
        Line::from(vec![
            Span::styled("  Sequence ID:     ", Style::default().fg(TEXT_DIM)),
            Span::styled(
                format!("{}", app.sequence_id),
                Style::default().fg(TEXT_PRIMARY),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Orders Cancelled:", Style::default().fg(TEXT_DIM)),
            Span::styled(
                format!(" {} pending orders", app.kill_switch_orders_cancelled),
                Style::default().fg(RED),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Positions:       ", Style::default().fg(TEXT_DIM)),
            Span::styled(
                format!(" {} positions flagged", app.kill_switch_positions_closed),
                Style::default().fg(ORANGE),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Mode:            ", Style::default().fg(TEXT_DIM)),
            Span::styled(
                if app.paper_mode {
                    " PAPER (no real orders)"
                } else {
                    " >> LIVE"
                },
                Style::default().fg(if app.paper_mode { BLUE } else { RED }),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Gateway:         ", Style::default().fg(TEXT_DIM)),
            Span::styled(
                " HALTED ",
                Style::default()
                    .fg(Color::Black)
                    .bg(RED)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                " ALL VENUES DISCONNECTED ",
                Style::default()
                    .fg(Color::Black)
                    .bg(ORANGE)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  [K] Resume Trading    [Q] Quit Application    [Esc] Dismiss",
            Style::default().fg(TEXT_SECONDARY),
        )]),
    ];

    f.render_widget(
        Paragraph::new(text).style(Style::default().bg(Color::Rgb(30, 0, 0))),
        inner,
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// ENHANCED BUY/SELL DIALOG
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_dialog(f: &mut Frame, app: &App) {
    let is_buy = app.show_buy_dialog;
    let title = if is_buy {
        " BUY ORDER "
    } else {
        " SELL ORDER "
    };
    let border_color = if is_buy { GREEN } else { RED };

    let area = centered_rect(45, 45, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(title)
        .title_style(
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(BG_ELEVATED));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mode_text = if app.paper_mode { "PAPER" } else { ">> LIVE" };
    let mode_color = if app.paper_mode { BLUE } else { RED };

    // Build order type selector
    let ot = app.dialog_order_type;
    let type_spans: Vec<Span> = [
        app::DialogOrderType::Market,
        app::DialogOrderType::Limit,
        app::DialogOrderType::Stop,
        app::DialogOrderType::Ioc,
    ]
    .iter()
    .map(|t| {
        if *t == ot {
            Span::styled(
                format!(" {} ", t.label()),
                Style::default()
                    .fg(Color::Black)
                    .bg(border_color)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!(" {} ", t.label()), Style::default().fg(TEXT_DIM))
        }
    })
    .collect();

    let text = vec![
        Line::from(vec![
            Span::styled("  Mode: ", Style::default().fg(TEXT_DIM)),
            Span::styled(
                mode_text,
                Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Symbol:   ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                &app.active_symbol,
                Style::default()
                    .fg(TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Quantity: ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(
                &app.order_qty_input,
                Style::default().fg(TEXT_PRIMARY).bg(BORDER),
            ),
            Span::styled(
                "█",
                Style::default()
                    .fg(border_color)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]),
        Line::from(""),
        Line::from({
            let mut spans = vec![Span::styled(
                "  Type:     ",
                Style::default().fg(TEXT_SECONDARY),
            )];
            spans.extend(type_spans);
            spans
        }),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  ─── AI SUGGESTION ───",
            Style::default().fg(CYAN),
        )]),
        Line::from(vec![
            Span::styled("  Dexter says: ", Style::default().fg(TEXT_DIM)),
            Span::styled(
                app.dexter_recommendation.as_deref().unwrap_or("—"),
                Style::default()
                    .fg(if app.dexter_recommendation.as_deref() == Some("BUY") {
                        GREEN
                    } else {
                        RED
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  conf: {:.0}%", app.dexter_confidence * 100.0),
                Style::default().fg(TEXT_DIM),
            ),
        ]),
        Line::from(vec![Span::styled(
            format!(
                "  SL: {:.1}%  TP: {:.1}%  Size: {:.1}%",
                app.dexter_stop_loss_pct, app.dexter_take_profit_pct, app.dexter_position_size_pct
            ),
            Style::default().fg(TEXT_DIM),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  [Enter] CONFIRM   [Esc] CANCEL   [Tab] Type",
            Style::default().fg(TEXT_SECONDARY),
        )]),
    ];

    f.render_widget(Paragraph::new(text), inner);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helper
// ═══════════════════════════════════════════════════════════════════════════════

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn block_with_title<'a>(title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .border_type(BorderType::Plain)
        .title(title)
        .title_style(Style::default().fg(TEXT_SECONDARY))
        .style(Style::default().bg(BG))
}

fn active_block_with_title<'a>(title: &'a str, is_active: bool) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_active { BORDER_ACTIVE } else { BORDER }))
        .border_type(if is_active {
            BorderType::Rounded
        } else {
            BorderType::Plain
        })
        .title(title)
        .title_style(Style::default().fg(if is_active { BLUE } else { TEXT_SECONDARY }))
        .style(Style::default().bg(BG))
}

// ═══════════════════════════════════════════════════════════════════════════════
// TOP BAR — Live clock + exchange status + latency
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_top_bar(f: &mut Frame, area: Rect, app: &App) {
    let now = chrono::Local::now();
    let clock = now.format("%H:%M:%S").to_string();
    let date = now.format("%b %d").to_string();

    let mut spans = vec![
        Span::styled(
            " RUST TERMINAL ",
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", Style::default().fg(BORDER)),
    ];

    // Exchange status indicators
    for ex in &app.exchanges {
        let (color, icon) = match ex.status {
            ExchangeStatus::Connected => (GREEN, "●"),
            ExchangeStatus::Degraded => (ORANGE, "◐"),
            ExchangeStatus::Disconnected => (RED, "○"),
            ExchangeStatus::Disabled => (TEXT_DIM, "○"),
        };
        spans.push(Span::styled(
            format!("{}", ex.name),
            Style::default().fg(color),
        ));
        spans.push(Span::styled(
            format!("{} ", icon),
            Style::default().fg(color),
        ));
    }

    spans.push(Span::styled("│ ", Style::default().fg(BORDER)));

    // Paper/Live mode
    if app.paper_mode {
        spans.push(Span::styled(
            "PAPER ",
            Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(
            ">>LIVE ",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(Span::styled("│ ", Style::default().fg(BORDER)));

    // Live metrics
    spans.push(Span::styled("E2E: ", Style::default().fg(TEXT_DIM)));
    spans.push(Span::styled("1.8ms ", Style::default().fg(GREEN)));
    spans.push(Span::styled("│ FIX 4.4 │ ", Style::default().fg(BORDER)));
    spans.push(Span::styled(
        &app.connection_status,
        Style::default().fg(TEXT_SECONDARY),
    ));

    // Right-aligned clock
    let status_len: usize = spans.iter().map(|s| s.content.len()).sum();
    let clock_str = format!(" {} {} ", date, clock);
    let padding = (area.width as usize)
        .saturating_sub(status_len.min(area.width as usize))
        .saturating_sub(clock_str.len());
    if padding > 0 {
        spans.push(Span::raw(" ".repeat(padding)));
    }
    spans.push(Span::styled(clock_str, Style::default().fg(CYAN)));

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(BG)),
        area,
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// MAIN CONTENT — 3 columns
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_main_content(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(55),
            Constraint::Percentage(25),
        ])
        .split(area);

    draw_left_col(f, cols[0], app);
    draw_center_col(f, cols[1], app);
    draw_right_col(f, cols[2], app);
}

// ── LEFT COLUMN ──────────────────────────────────────────────────────────────

fn draw_left_col(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    draw_watchlist(f, chunks[0], app);
    draw_dexter_alerts(f, chunks[1], app);
}

fn draw_watchlist(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .watchlist
        .iter()
        .map(|item| {
            let color = if item.change_pct >= 0.0 { GREEN } else { RED };
            let sign = if item.change_pct >= 0.0 { "+" } else { "" };
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(
                        item.symbol.clone(),
                        Style::default()
                            .fg(TEXT_PRIMARY)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("\n{}", item.name), Style::default().fg(TEXT_DIM)),
                ])),
                Cell::from(Span::styled(
                    format!("{:.2}", item.price),
                    Style::default().fg(TEXT_PRIMARY),
                )),
                Cell::from(Span::styled(
                    format!("{}{:.2}%", sign, item.change_pct),
                    Style::default().fg(color),
                )),
            ])
            .height(2)
        })
        .collect();

    let widths = [
        Constraint::Length(15),
        Constraint::Length(8),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(Row::new(vec!["Symbol", "Price", "Change"]).style(Style::default().fg(TEXT_DIM)))
        .block(active_block_with_title(
            " Watchlist ",
            app.active_panel == 0,
        ));

    f.render_widget(table, area);
}

fn draw_dexter_alerts(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .alerts
        .iter()
        .map(|a| {
            let color = match a.severity {
                crate::app::AlertSeverity::Info => BLUE,
                crate::app::AlertSeverity::Warning => ORANGE,
                crate::app::AlertSeverity::Critical => RED,
            };
            ListItem::new(Line::from(vec![
                Span::styled("● ", Style::default().fg(color)),
                Span::styled(a.text.clone(), Style::default().fg(TEXT_PRIMARY)),
            ]))
        })
        .collect();

    let list = List::new(items).block(active_block_with_title(
        " Dexter Alerts ",
        app.active_panel == 1,
    ));
    f.render_widget(list, area);
}

// ── CENTER COLUMN ────────────────────────────────────────────────────────────

fn draw_center_col(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),      // Index Strip
            Constraint::Percentage(45), // Chart
            Constraint::Percentage(25), // Order Book
            Constraint::Percentage(25), // Dexter & Mirofish
            Constraint::Length(3),      // Order Entry
        ])
        .split(area);

    draw_index_strip(f, chunks[0]);
    render_candlestick_chart(
        f,
        chunks[1],
        &app.candles,
        &app.candle_state,
        &app.active_symbol,
    );
    draw_order_book(f, chunks[2], app);

    let bottom_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);

    draw_dexter_analyst(f, bottom_split[0], app);
    draw_mirofish_sim(f, bottom_split[1], app);
    draw_order_entry(f, chunks[4], app);
}

fn draw_index_strip(f: &mut Frame, area: Rect) {
    let p = Paragraph::new(Line::from(vec![
        Span::styled("S&P 500 ", Style::default().fg(RED)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(" Nasdaq-100 ", Style::default().fg(GREEN)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(" Dow Jones ", Style::default().fg(GREEN)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(" CRYPTO:/CBOP ", Style::default().fg(TEXT_PRIMARY)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(" Major Intra ", Style::default().fg(ORANGE)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(" LMIIc (UD) ", Style::default().fg(TEXT_DIM)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(" Major I ", Style::default().fg(TEXT_DIM)),
    ]))
    .block(block_with_title(" Market Index Strip "));
    f.render_widget(p, area);
}

fn draw_order_book(f: &mut Frame, area: Rect, app: &App) {
    let ob_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let max_ask: f64 = app
        .order_book
        .iter()
        .map(|r| r.ask_total)
        .fold(1.0, f64::max);
    let max_bid: f64 = app
        .order_book
        .iter()
        .map(|r| r.bid_total)
        .fold(1.0, f64::max);

    let t_rows: Vec<Row> = app
        .order_book
        .iter()
        .map(|row| {
            let ai = (row.ask_total / max_ask).min(1.0);
            let bi = (row.bid_total / max_bid).min(1.0);
            let ask_bg = Color::Rgb((50.0 * ai) as u8, (18.0 * ai) as u8, (18.0 * ai) as u8);
            let bid_bg = Color::Rgb((18.0 * bi) as u8, (50.0 * bi) as u8, (18.0 * bi) as u8);
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("${:.2}", row.ask_price),
                    Style::default().fg(RED),
                ))
                .style(Style::default().bg(ask_bg)),
                Cell::from(row.ask_size.to_string()).style(Style::default().bg(ask_bg)),
                Cell::from(Span::styled(
                    format!("{:.0}M", row.ask_total),
                    Style::default().fg(RED).add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(ask_bg)),
                Cell::from(Span::styled(
                    format!("${:.2}", row.bid_price),
                    Style::default().fg(GREEN),
                ))
                .style(Style::default().bg(bid_bg)),
                Cell::from(row.bid_size.to_string()).style(Style::default().bg(bid_bg)),
                Cell::from(Span::styled(
                    format!("{:.0}M", row.bid_total),
                    Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(bid_bg)),
            ])
        })
        .collect();

    let table = Table::new(t_rows, [Constraint::Percentage(16); 6])
        .header(
            Row::new(vec!["Asks", "Size", "Total", "Bids", "Size", "Total"])
                .style(Style::default().fg(TEXT_DIM)),
        )
        .block(active_block_with_title(
            " Order Book ",
            app.active_panel == 2,
        ));
    f.render_widget(table, ob_chunks[0]);

    let (spread, mid, imbal) =
        if let (Some(first), Some(last)) = (app.order_book.first(), app.order_book.last()) {
            let s = if last.bid_price > 0.0 {
                (first.ask_price - last.bid_price) / last.bid_price * 100.0
            } else {
                0.0
            };
            let m = (first.ask_price + first.bid_price) / 2.0;
            let ta: f64 = app.order_book.iter().map(|r| r.ask_total).sum();
            let tb: f64 = app.order_book.iter().map(|r| r.bid_total).sum();
            let i = if ta + tb > 0.0 {
                (tb - ta) / (ta + tb) * 100.0
            } else {
                0.0
            };
            (s, m, i)
        } else {
            (0.0, 0.0, 0.0)
        };

    let summary = Line::from(vec![
        Span::styled(" Spread ", Style::default().fg(TEXT_DIM)),
        Span::styled(format!("{:.2}%", spread), Style::default().fg(TEXT_PRIMARY)),
        Span::styled("      Mid price ", Style::default().fg(TEXT_DIM)),
        Span::styled(format!("{:.2}", mid), Style::default().fg(TEXT_PRIMARY)),
        Span::styled("      Buy/Sell imbal ", Style::default().fg(TEXT_DIM)),
        Span::styled(
            format!("{:+.0}%", imbal),
            Style::default().fg(if imbal > 0.0 { GREEN } else { RED }),
        ),
    ]);
    f.render_widget(Paragraph::new(summary), ob_chunks[1]);
}

// ═══════════════════════════════════════════════════════════════════════════════
// ENHANCED DEXTER PANEL — Full signal card
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_dexter_analyst(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.active_panel == 3 {
            BORDER_ACTIVE
        } else {
            BORDER
        }))
        .title(Line::from(vec![
            Span::styled("◉ ", Style::default().fg(CYAN)),
            Span::styled(
                "DEXTER — FINANCIAL ANALYST",
                Style::default().fg(if app.active_panel == 3 {
                    BLUE
                } else {
                    TEXT_SECONDARY
                }),
            ),
        ]))
        .style(Style::default().bg(BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.dexter_loading {
        let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let idx = (app.session_start.elapsed().as_millis() / 100) as usize % spinner_frames.len();
        f.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "  {} Analyzing {} ...",
                        spinner_frames[idx], app.active_symbol
                    ),
                    Style::default().fg(CYAN),
                )),
            ]),
            inner,
        );
        return;
    }

    let mut text: Vec<Line> = Vec::new();

    // Render valuation output lines
    for line in &app.dexter_output {
        text.push(Line::from(Span::styled(
            format!(" {}", line),
            Style::default().fg(TEXT_PRIMARY),
        )));
    }

    text.push(Line::from(""));

    // BUY / RISK / NEUTRAL buttons
    text.push(Line::from(vec![
        Span::styled(
            " BUY ",
            Style::default()
                .fg(Color::Black)
                .bg(GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            " RISK ",
            Style::default()
                .fg(Color::Black)
                .bg(RED)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            " NEUTRAL ",
            Style::default()
                .fg(Color::Black)
                .bg(AMBER)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    f.render_widget(Paragraph::new(text), inner);
}

// ═══════════════════════════════════════════════════════════════════════════════
// ENHANCED MIROFISH PANEL — Gauge + agent microstructure
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_mirofish_sim(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.active_panel == 4 {
            BORDER_ACTIVE
        } else {
            BORDER
        }))
        .title(Line::from(vec![
            Span::styled("◉ ", Style::default().fg(PURPLE)),
            Span::styled(
                "MIROFISH — SWARM SIMULATION",
                Style::default().fg(if app.active_panel == 4 {
                    BLUE
                } else {
                    TEXT_SECONDARY
                }),
            ),
        ]))
        .style(Style::default().bg(BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let status = if app.mirofish_running {
        format!("{} agent simulation running...", app.mirofish_agent_count)
    } else {
        "Idle. Press [F] to start.".to_string()
    };

    // Build gauge bars
    let rally_filled = (app.mirofish_rally_pct / 5.0) as usize;
    let rally_bar = format!(
        "{}{}",
        "█".repeat(rally_filled.min(20)),
        "░".repeat(20 - rally_filled.min(20))
    );

    let side_filled = (app.mirofish_sideways_pct / 5.0) as usize;
    let side_bar = format!(
        "{}{}",
        "█".repeat(side_filled.min(20)),
        "░".repeat(20 - side_filled.min(20))
    );

    let dip_filled = (app.mirofish_dip_pct / 5.0) as usize;
    let dip_bar = format!(
        "{}{}",
        "█".repeat(dip_filled.min(20)),
        "░".repeat(20 - dip_filled.min(20))
    );

    // Sum check
    let sum = app.mirofish_rally_pct + app.mirofish_sideways_pct + app.mirofish_dip_pct;
    let sum_ok = (sum - 100.0).abs() < 0.1;
    let sum_icon = if sum_ok { "[OK]" } else { "[!]" };

    // Bias detection
    let bias_text = if app.mirofish_bias_detected {
        Span::styled(
            " [!] BIAS >85% -- herding detected",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(" [OK] No herding bias detected", Style::default().fg(GREEN))
    };

    let text = vec![
        Line::from(Span::styled(
            format!(" {}", status),
            Style::default().fg(TEXT_SECONDARY),
        )),
        Line::from(Span::styled(
            " Scenario probability",
            Style::default()
                .fg(TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )),
        // Rally
        Line::from(vec![
            Span::styled(" Rally    ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(&rally_bar, Style::default().fg(GREEN)),
            Span::styled(
                format!(" {:.0}%", app.mirofish_rally_pct),
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
        ]),
        // Sideways
        Line::from(vec![
            Span::styled(" Sideways ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(&side_bar, Style::default().fg(AMBER)),
            Span::styled(
                format!(" {:.0}%", app.mirofish_sideways_pct),
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            ),
        ]),
        // Dip
        Line::from(vec![
            Span::styled(" Dip      ", Style::default().fg(TEXT_SECONDARY)),
            Span::styled(&dip_bar, Style::default().fg(PURPLE)),
            Span::styled(
                format!(" {:.0}%", app.mirofish_dip_pct),
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            ),
        ]),
        // Sum check
        Line::from(vec![Span::styled(
            format!(
                " {:.0} + {:.0} + {:.0} = {:.0}% {}",
                app.mirofish_rally_pct,
                app.mirofish_sideways_pct,
                app.mirofish_dip_pct,
                sum,
                sum_icon
            ),
            Style::default().fg(TEXT_DIM),
        )]),
        // Agent microstructure
        Line::from(vec![Span::styled(
            format!(
                " Agents: {}  Sim: {:.0}ms  OI: {:.2}  σ: {:.3}",
                app.mirofish_agent_count,
                app.mirofish_sim_time_ms,
                app.mirofish_order_imbalance,
                app.mirofish_simulated_vol
            ),
            Style::default().fg(TEXT_DIM),
        )]),
        // Agreement
        Line::from(vec![
            Span::styled(
                format!(" Agreement: {:.0}%  ", app.mirofish_agent_agreement),
                Style::default().fg(if app.mirofish_agent_agreement > 85.0 {
                    RED
                } else {
                    GREEN
                }),
            ),
            bias_text,
        ]),
    ];

    f.render_widget(Paragraph::new(text), inner);
}

// ═══════════════════════════════════════════════════════════════════════════════
// ORDER ENTRY STRIP
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_order_entry(f: &mut Frame, area: Rect, app: &App) {
    let price = app.watchlist.first().map(|w| w.price).unwrap_or(20.0);

    let text = Line::from(vec![
        Span::styled(" Symbol ", Style::default().fg(TEXT_DIM)),
        Span::styled(
            format!(" {} ", app.active_symbol),
            Style::default()
                .fg(TEXT_PRIMARY)
                .add_modifier(Modifier::REVERSED),
        ),
        Span::styled("  Qty ", Style::default().fg(TEXT_DIM)),
        Span::styled(
            " 1 ",
            Style::default()
                .fg(TEXT_PRIMARY)
                .add_modifier(Modifier::REVERSED),
        ),
        Span::styled("  Price ", Style::default().fg(TEXT_DIM)),
        Span::styled(
            format!(" ${:.2} ", price),
            Style::default()
                .fg(TEXT_PRIMARY)
                .add_modifier(Modifier::REVERSED),
        ),
        Span::raw("  "),
        Span::styled("LMT/MKT/STP/IOC ", Style::default().fg(TEXT_DIM)),
        Span::styled(
            " BUY ",
            Style::default()
                .fg(Color::Black)
                .bg(GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " SELL ",
            Style::default()
                .fg(Color::White)
                .bg(RED)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(
        Paragraph::new(text).block(block_with_title(" Order Entry Strip ")),
        area,
    );
}

// ── RIGHT COLUMN ─────────────────────────────────────────────────────────────

fn draw_right_col(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(65),
            Constraint::Percentage(10),
        ])
        .split(area);

    draw_open_positions(f, chunks[0], app);
    draw_news_feed(f, chunks[1], app);
    draw_day_pnl(f, chunks[2], app);
}

fn draw_open_positions(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .positions
        .iter()
        .map(|p| {
            let color = if p.pnl_pct >= 0.0 { GREEN } else { RED };
            let sign = if p.pnl_pct >= 0.0 { "+" } else { "" };
            let pnl_str = format!("{}{:.2}%", sign, p.pnl_pct);
            let holding_str = if p.holding > 0.0 {
                format!("+{:.2}", p.holding)
            } else {
                format!("{:.2}", p.holding)
            };

            Row::new(vec![p.symbol.clone(), holding_str, pnl_str]).style(Style::default().fg(color))
        })
        .collect();

    let table = Table::new(rows, [Constraint::Percentage(33); 3])
        .header(Row::new(vec!["Holding", "", "P&L"]).style(Style::default().fg(TEXT_DIM)))
        .block(active_block_with_title(
            " Open Positions ",
            app.active_panel == 5,
        ));
    f.render_widget(table, area);
}

fn draw_news_feed(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .news
        .iter()
        .map(|n| {
            ListItem::new(vec![
                Line::from(Span::styled(
                    format!("{} • {}", n.source, n.time_ago),
                    Style::default().fg(TEXT_DIM),
                )),
                Line::from(Span::raw(n.headline.clone())),
                Line::from(""),
            ])
        })
        .collect();

    f.render_widget(
        List::new(items).block(block_with_title(" News Feed ")),
        area,
    );
}

fn draw_day_pnl(f: &mut Frame, area: Rect, app: &App) {
    let color = if app.day_pnl >= 0.0 { GREEN } else { RED };
    let text = vec![
        Line::from(vec![
            Span::styled(" Day P&L:        ", Style::default().fg(TEXT_DIM)),
            Span::styled(
                format!("${:.2}K", app.day_pnl),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Buying power:   ", Style::default().fg(TEXT_DIM)),
            Span::raw(format!("{:.1}B", app.available_power)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::NONE)),
        area,
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// BOTTOM STATUS BAR — Live metrics
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_bottom_bar(f: &mut Frame, area: Rect, app: &App) {
    let uptime = app.session_uptime();
    let fill_ratio = app.fill_ratio_str();

    let text = Line::from(vec![
        Span::styled(" Rust v6.19 ", Style::default().fg(TEXT_DIM)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(" tokio ", Style::default().fg(TEXT_DIM)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(" Thread: 1 ", Style::default().fg(TEXT_DIM)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(" MPSC ", Style::default().fg(TEXT_DIM)),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(
            format!(" Dexter: {} ", app.dexter_call_count),
            Style::default().fg(CYAN),
        ),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(
            format!(" Swarm: {} ", app.mirofish_agent_count),
            Style::default().fg(PURPLE),
        ),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(
            format!(" Orders: {} ", app.orders_sent),
            Style::default().fg(TEXT_DIM),
        ),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(
            format!(" Fills: {} ", fill_ratio),
            Style::default().fg(TEXT_DIM),
        ),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(
            format!(" Seq: {} ", app.sequence_id),
            Style::default().fg(TEXT_DIM),
        ),
        Span::styled("│", Style::default().fg(BORDER)),
        Span::styled(format!(" Uptime: {} ", uptime), Style::default().fg(GREEN)),
    ]);
    f.render_widget(Paragraph::new(text).style(Style::default().bg(BG)), area);
}
