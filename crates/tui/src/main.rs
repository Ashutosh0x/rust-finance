use std::{
    io,
    time::{Duration},
};
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{*},
    text::Line,
};
use tokio::net::TcpStream;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

// Custom Colors matching the image
const BG: Color = Color::Rgb(10, 12, 15);
const BORDER: Color = Color::Rgb(30, 37, 48);
const TEXT_PRIMARY: Color = Color::Rgb(226, 232, 240);
const TEXT_SECONDARY: Color = Color::Rgb(148, 163, 184);
const GREEN: Color = Color::Rgb(74, 222, 128);
const RED: Color = Color::Rgb(248, 113, 113);
const ORANGE: Color = Color::Rgb(249, 115, 22);
const BLUE: Color = Color::Rgb(96, 165, 250);
const PURPLE: Color = Color::Rgb(167, 139, 250);

mod app;
mod event_handler;
pub mod widgets;
pub mod layout;
pub mod state;
pub mod setup;

use app::App;
use common::models::exchange::ExchangeStatus;
use widgets::chart_widget::render_chart;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, crossterm::event::EnableMouseCapture, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Try loading existing .env
    let _ = dotenvy::dotenv();

    // Determine if setup is needed
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

    // Event Bus Connection Manager
    let (tx_status, mut rx_status) = mpsc::channel::<String>(100);
    let (tx_event, mut rx_event) = mpsc::channel::<common::events::BotEvent>(1000);
    
    // Clone channels for the async task
    let tx_status_clone = tx_status.clone();
    
    tokio::spawn(async move {
        loop {
            match TcpStream::connect("127.0.0.1:7001").await {
                Ok(stream) => {
                    let _ = tx_status_clone.send("Connected to Daemon (127.0.0.1:7001/binary)".to_string()).await;
                    let (mut reader, _writer) = tokio::io::split(stream);
                    let mut length_buf = [0u8; 4];
                    
                    loop {
                        if reader.read_exact(&mut length_buf).await.is_err() {
                            break; // EOF or Error
                        }
                        let len = u32::from_le_bytes(length_buf) as usize;
                        if len > 1024 * 1024 { break; } // Safety limit
                        
                        let mut buf = vec![0u8; len];
                        if reader.read_exact(&mut buf).await.is_err() {
                            break;
                        }
                        
                        // Parse event using Postcard
                        if let Ok(event) = postcard::from_bytes::<common::events::BotEvent>(&buf) {
                            let _ = tx_event.send(event).await;
                        }
                    }
                    let _ = tx_status_clone.send("Daemon Disconnected. Reconnecting...".to_string()).await;
                }
                Err(_) => {
                    let _ = tx_status_clone.send("Connection Failed. Retrying...".to_string()).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    loop {
        // Non-blocking UI update from network thread
        while let Ok(msg) = rx_status.try_recv() {
            app.connection_status = msg;
        }
        while let Ok(event) = rx_event.try_recv() {
            app.update_from_event(event);
        }

        terminal.draw(|f| {
            match &app.screen {
                crate::app::AppScreen::Setup(state) => crate::setup::render_setup(f, state),
                crate::app::AppScreen::Dashboard => ui(f, &app),
            }
        })?;

        if crossterm::event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    match &mut app.screen {
                        crate::app::AppScreen::Setup(state) => {
                            match crate::setup::handle_setup_key(key, state) {
                                crate::setup::SetupAction::Submit => {
                                    let pairs: Vec<(&str, &str)> = state.fields.iter()
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
                                            state.error_msg = Some(format!("Failed to save .env: {}", e));
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
                    }
                }
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
    execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
    Ok(())
}

fn ui(f: &mut Frame, app: &App) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Top Bar
            Constraint::Min(0),    // Main Content
            Constraint::Length(1), // Bottom Bar
        ])
        .split(f.size());

    draw_top_bar(f, main_layout[0], app);
    draw_main_content(f, main_layout[1], app);
    draw_bottom_bar(f, main_layout[2]);

    if app.show_help {
        crate::widgets::help_overlay::render_help_overlay(f);
    }
    
    if app.show_buy_dialog || app.show_sell_dialog {
        draw_dialog(f, app);
    }
}

fn draw_dialog(f: &mut Frame, app: &App) {
    let title = if app.show_buy_dialog { "BUY ORDER" } else { "SELL ORDER" };
    let border_color = if app.show_buy_dialog { GREEN } else { RED };
    
    // Create a 40x10 popup in the center of the screen
    let area = centered_rect(40, 15, f.size());
    
    f.render_widget(Clear, area); // Clear background
    
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(BG));
        
    let text = vec![
        Line::from(vec![Span::styled(format!("Symbol:   {}", app.active_symbol), Style::default().add_modifier(Modifier::BOLD))]),
        Line::from(""),
        Line::from(vec![Span::raw("Quantity: "), Span::styled(&app.order_qty_input, Style::default().fg(TEXT_PRIMARY).bg(BORDER)), Span::raw("█")]),
        Line::from(""),
        Line::from(vec![Span::raw("Type:     MARKET")]), // Hardcoded for simplified input right now
        Line::from(""),
        Line::from(vec![Span::styled("[ENTER] Confirm   [ESC] Cancel", Style::default().fg(TEXT_SECONDARY))]),
    ];
    
    f.render_widget(Paragraph::new(text).block(block).alignment(Alignment::Center), area);
}

// Helper function to center a rect
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

fn draw_top_bar(f: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(" RUST TERMINAL   ", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)),
    ];

    for ex in &app.exchanges {
        let (color, icon) = match ex.status {
            ExchangeStatus::Connected => (GREEN, "●"),
            ExchangeStatus::Degraded => (ORANGE, "◐"),
            ExchangeStatus::Disconnected => (RED, "○"),
            ExchangeStatus::Disabled => (TEXT_SECONDARY, "○"),
        };
        spans.push(Span::styled(format!("{} ", ex.name), Style::default().fg(color)));
        spans.push(Span::raw(format!("{}  ", icon)));
    }

    spans.push(Span::raw("                                        "));
    spans.push(Span::styled("● ", Style::default().fg(Color::Cyan)));
    spans.push(Span::styled(
        format!(" Live: E2E: 1.8ms | Status: {} | 11 2:30 EST", app.connection_status),
        Style::default().fg(TEXT_SECONDARY)
    ));

    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(BG)), area);
}

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

fn draw_left_col(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .split(area);

    draw_watchlist(f, chunks[0], app);
    draw_dexter_alerts(f, chunks[1], app);
}

fn draw_watchlist(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app.watchlist.iter().map(|item| {
        let color = if item.change_pct >= 0.0 { GREEN } else { RED };
        let sign = if item.change_pct >= 0.0 { "+" } else { "" };
        Row::new(vec![
            Cell::from(Line::from(vec![
                Span::styled(item.symbol.clone(), Style::default().fg(TEXT_PRIMARY)),
                Span::styled(format!("\n{}", item.name), Style::default().fg(TEXT_SECONDARY)),
            ])),
            Cell::from(Span::styled(format!("{:.2}", item.price), Style::default().fg(TEXT_PRIMARY))),
            Cell::from(Span::styled(format!("{}{:.2}%", sign, item.change_pct), Style::default().fg(color))),
        ]).height(2)
    }).collect();

    let widths = [
        Constraint::Length(15),
        Constraint::Length(8),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(Row::new(vec!["Symbol", "Price", "Change"]).style(Style::default().fg(TEXT_SECONDARY)))
        .block(block_with_title("Watchlist"));

    // Quick hack for scrolling - ratatui Table needs TableState for true scrolling,
    // so we'd normally pass a mutable reference. For now, we rely on the App state.
    
    f.render_widget(table, area);
}

fn draw_dexter_alerts(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app.alerts.iter().map(|a| {
        let color = match a.severity {
            crate::app::AlertSeverity::Info => BLUE,
            crate::app::AlertSeverity::Warning => ORANGE,
            crate::app::AlertSeverity::Critical => RED,
        };
        ListItem::new(Line::from(vec![
            Span::styled("● ", Style::default().fg(color)),
            Span::styled(a.text.clone(), Style::default().fg(TEXT_PRIMARY)),
        ]))
    }).collect();

    let list = List::new(items).block(block_with_title("Dexter Alerts"));
    f.render_widget(list, area);
}

fn draw_center_col(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Strip
            Constraint::Percentage(50), // Chart // Dynamically takes half instead of fixed 13
            Constraint::Percentage(25),     // Order Book
            Constraint::Percentage(17),  // Dexter & Mirofish
            Constraint::Percentage(5),  // Order Entry
        ])
        .split(area);

    draw_index_strip(f, chunks[0]);
    // Delegate to the new widget module
    render_chart(f, chunks[1], &app.chart_data, &app.volume_data, &app.chart_state, &app.chart_stats);
    draw_order_book(f, chunks[2], app);

    let bottom_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);
    
    draw_dexter_analyst(f, bottom_split[0], app);
    draw_mirofish_sim(f, bottom_split[1], app);
    draw_order_entry(f, chunks[4]);
}

fn draw_index_strip(f: &mut Frame, area: Rect) {
    let p = Paragraph::new(Line::from(vec![
        Span::styled("S&P 500 ", Style::default().fg(RED)), Span::raw(" | "),
        Span::styled("Nasdaq-100 ", Style::default().fg(GREEN)), Span::raw(" | "),
        Span::styled("Dow Jones ", Style::default().fg(GREEN)), Span::raw(" | "),
        Span::styled("CRYPTO:/CBOP ", Style::default().fg(TEXT_PRIMARY)), Span::raw(" | "),
        Span::styled("Major Intra ", Style::default().fg(ORANGE)), Span::raw(" | "),
        Span::styled("LMIIc (UD) ", Style::default().fg(TEXT_SECONDARY)), Span::raw(" | "),
        Span::styled("Major I ", Style::default().fg(TEXT_SECONDARY)), 
    ])).block(block_with_title("Market Index Strip"));
    f.render_widget(p, area);
}

fn draw_order_book(f: &mut Frame, area: Rect, app: &App) {
    let t_rows: Vec<Row> = app.order_book.iter().map(|row| {
        Row::new(vec![
            Cell::from(Span::styled(format!("${:.2}", row.ask_price), Style::default().fg(RED))),
            Cell::from(row.ask_size.to_string()),
            Cell::from(Span::styled(format!("{:.0}M", row.ask_total), Style::default().fg(RED).add_modifier(Modifier::BOLD))),
            Cell::from(Span::styled(format!("${:.2}", row.bid_price), Style::default().fg(GREEN))),
            Cell::from(row.bid_size.to_string()),
            Cell::from(Span::styled(format!("{:.0}M", row.bid_total), Style::default().fg(GREEN).add_modifier(Modifier::BOLD))),
        ])
    }).collect();

    let table = Table::new(t_rows, [Constraint::Percentage(16); 6])
        .header(Row::new(vec!["Asks", "Size", "Total", "Bids", "Size", "Total"]).style(Style::default().fg(TEXT_SECONDARY)))
        .block(block_with_title("Order Book"));
    
    f.render_widget(table, area);
}

fn draw_dexter_analyst(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Line::from(vec![
            Span::styled("● ", Style::default().fg(BLUE)),
            Span::styled("DEXTER - FINANCIAL ANALYST", Style::default().fg(TEXT_SECONDARY))
        ]))
        .style(Style::default().bg(BG));
        
    let mut text: Vec<Line> = app.dexter_output.iter()
        .map(|s| Line::from(s.clone()))
        .collect();

    if let Some(rec) = &app.dexter_recommendation {
        text.push(Line::from(""));
        let buy_style = if rec == "BUY" { Style::default().fg(Color::Black).bg(GREEN).add_modifier(Modifier::BOLD) } else { Style::default().fg(TEXT_SECONDARY) };
        let risk_style = if rec == "RISK" { Style::default().fg(Color::Black).bg(ORANGE).add_modifier(Modifier::BOLD) } else { Style::default().fg(TEXT_SECONDARY) };
        let neutral_style = if rec == "NEUTRAL" { Style::default().fg(Color::Black).bg(TEXT_SECONDARY).add_modifier(Modifier::BOLD) } else { Style::default().fg(TEXT_SECONDARY) };
        
        text.push(Line::from(vec![
            Span::styled(" BUY ", buy_style),
            Span::raw(" "),
            Span::styled(" RISK ", risk_style),
            Span::raw(" "),
            Span::styled(" NEUTRAL ", neutral_style),
        ]));
    }
        
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn draw_mirofish_sim(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Line::from(vec![
            Span::styled("● ", Style::default().fg(PURPLE)),
            Span::styled("MIROFISH - SWARM SIMULATION", Style::default().fg(TEXT_SECONDARY))
        ]))
        .style(Style::default().bg(BG));
        
    let status = if app.mirofish_running { "5,000 agent simulation running..." } else { "Idle." };
    
    let rally_bar = "█".repeat((app.mirofish_rally_pct / 5.0) as usize);
    let rally_space = "░".repeat(20_usize.saturating_sub((app.mirofish_rally_pct / 5.0) as usize));
    
    let side_bar = "█".repeat((app.mirofish_sideways_pct / 5.0) as usize);
    let side_space = "░".repeat(20_usize.saturating_sub((app.mirofish_sideways_pct / 5.0) as usize));
    
    let dip_bar = "█".repeat((app.mirofish_dip_pct / 5.0) as usize);
    let dip_space = "░".repeat(20_usize.saturating_sub((app.mirofish_dip_pct / 5.0) as usize));

    let text = vec![
        Line::from(status),
        Line::from(""),
        Line::from("Scenario probability"),
        Line::from(vec![
            Span::raw("Rally    "),
            Span::styled(rally_bar, Style::default().fg(BLUE)),
            Span::styled(format!("{} {:.0}%", rally_space, app.mirofish_rally_pct), Style::default().fg(BORDER)),
        ]),
        Line::from(vec![
            Span::raw("Sideways "),
            Span::styled(side_bar, Style::default().fg(TEXT_SECONDARY)),
            Span::styled(format!("{} {:.0}%", side_space, app.mirofish_sideways_pct), Style::default().fg(BORDER)),
        ]),
        Line::from(vec![
            Span::raw("Dip      "),
            Span::styled(dip_bar, Style::default().fg(PURPLE)),
            Span::styled(format!("{} {:.0}%", dip_space, app.mirofish_dip_pct), Style::default().fg(BORDER)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("Institutional accumulation: "),
            Span::styled("Hitting 80%", Style::default().fg(GREEN))
        ]),
        Line::from(vec![
            Span::raw("Retail sentiment: "),
            Span::styled("50% accumulation", Style::default().fg(GREEN))
        ]),
    ];

    f.render_widget(Paragraph::new(text).block(block), area);
}

fn draw_order_entry(f: &mut Frame, area: Rect) {
    let text = Line::from(vec![
        Span::raw("Symbol "),
        Span::styled(" AAPL  ", Style::default().fg(TEXT_PRIMARY).add_modifier(Modifier::REVERSED)),
        Span::raw("   Quantity "),
        Span::styled(" 1     ", Style::default().fg(TEXT_PRIMARY).add_modifier(Modifier::REVERSED)),
        Span::raw("   Price "),
        Span::styled(" $20.00 ", Style::default().fg(TEXT_PRIMARY).add_modifier(Modifier::REVERSED)),
        Span::raw("   "),
        Span::styled("LMT / MKT / STP / IOC   ", Style::default().fg(TEXT_SECONDARY)),
        Span::styled(" BUY ", Style::default().fg(Color::Black).bg(GREEN).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" SELL ", Style::default().fg(Color::White).bg(RED).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(Paragraph::new(text).block(block_with_title("Order Entry Strip")), area);
}

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
    let rows: Vec<Row> = app.positions.iter().map(|p| {
        let color = if p.pnl_pct >= 0.0 { GREEN } else { RED };
        let sign = if p.pnl_pct >= 0.0 { "+" } else { "" };
        let pnl_str = format!("{}{:.2}%", sign, p.pnl_pct);
        let holding_str = if p.holding > 0.0 { format!("+{:.2}", p.holding) } else { format!("{:.2}", p.holding) };
        
        Row::new(vec![p.symbol.clone(), holding_str, pnl_str]).style(Style::default().fg(color))
    }).collect();

    let table = Table::new(rows, [Constraint::Percentage(33); 3])
        .header(Row::new(vec!["Holding", "", "P&L"]).style(Style::default().fg(TEXT_SECONDARY)))
        .block(block_with_title("Open Positions"));
    f.render_widget(table, area);
}

fn draw_news_feed(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app.news.iter().map(|n| {
        ListItem::new(vec![
            Line::from(Span::styled(format!("{} • {}", n.source, n.time_ago), Style::default().fg(TEXT_SECONDARY))),
            Line::from(Span::raw(n.headline.clone())),
            Line::from(""), // spacing
        ])
    }).collect();
    
    f.render_widget(List::new(items).block(block_with_title("News Feed")), area);
}

fn draw_day_pnl(f: &mut Frame, area: Rect, app: &App) {
    let color = if app.day_pnl >= 0.0 { GREEN } else { RED };
    let text = vec![
        Line::from(vec![Span::raw("Day P&L:           "), Span::styled(format!("${:.2}K", app.day_pnl), Style::default().fg(color))]),
        Line::from(vec![Span::raw("Available power:  "), Span::raw(format!("{:.1}B", app.available_power))]),
    ];
    f.render_widget(Paragraph::new(text).block(Block::default().borders(Borders::NONE)), area);
}

fn draw_bottom_bar(f: &mut Frame, area: Rect) {
    let text = Line::from(vec![
        Span::styled("Rust version: 6.19 | async runtime: tokio | Active thread: 1 | Feed protocol: MPSC | Dexter: 0 | MiroFish active agent: 6 | Orders sent: 15 | Fills vs rejections: 0 | Session uptime: 12:05:32", Style::default().fg(TEXT_SECONDARY))
    ]);
    f.render_widget(Paragraph::new(text).style(Style::default().bg(BG)), area);
}
