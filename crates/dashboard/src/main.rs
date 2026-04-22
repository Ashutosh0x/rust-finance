#![forbid(unsafe_code)]
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::{io, time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        tracing::error!("Dashboard error: {:?}", err);
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>) -> io::Result<()> {
    loop {
        terminal.draw(ui)?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if let KeyCode::Char('q') = key.code {
                    return Ok(());
                }
            }
        }
    }
}

fn ui(f: &mut Frame) {
    // Top Bar (1 row) + Main Content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3), // Top Bar
                Constraint::Min(0),    // Main 3-column Grid
                Constraint::Length(3), // Footer/Status Bar
            ]
            .as_ref(),
        )
        .split(f.size());

    // Top Bar
    let top_bar = Paragraph::new(
        "RUSTFORGE | NYSE ● NASDAQ ● CME ● CBOE ● CRYPTO ● | FIX 4.4 | LATENCY: 1.2ms",
    )
    .style(Style::default().fg(Color::Green))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(top_bar, chunks[0]);

    // Main 3-Column Grid
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Length(40),     // Left: Watchlist + Alerts
                Constraint::Percentage(60), // Center: Chart, Book, AI
                Constraint::Length(40),     // Right: Positions + News
            ]
            .as_ref(),
        )
        .split(chunks[1]);

    // Left Column: Watchlist (top) + Dexter Alerts (bottom)
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(main_chunks[0]);

    let watchlist = Block::default().title("Watchlist").borders(Borders::ALL);
    f.render_widget(watchlist, left_chunks[0]);
    let alerts = Block::default()
        .title("Dexter Alerts")
        .borders(Borders::ALL);
    f.render_widget(alerts, left_chunks[1]);

    // Center Column: Chart (top), AI/Book (middle), Order Entry (bottom)
    let center_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage(40), // Chart
                Constraint::Percentage(45), // AI Panels & Orderbook
                Constraint::Length(3),      // Order Entry
            ]
            .as_ref(),
        )
        .split(main_chunks[1]);

    let chart = Block::default()
        .title("AAPL Chart (mock)")
        .borders(Borders::ALL);
    f.render_widget(chart, center_chunks[0]);

    let middle_bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(center_chunks[1]);

    let ai_dexter = Block::default()
        .title("DEXTER - Analyst")
        .borders(Borders::ALL);
    f.render_widget(ai_dexter, middle_bottom_chunks[0]);
    let ai_mirofish = Block::default()
        .title("MIROFISH - Swarm")
        .borders(Borders::ALL);
    f.render_widget(ai_mirofish, middle_bottom_chunks[1]);

    let order_entry = Paragraph::new("Symbol: AAPL | Qty: 100 | LMT 150.00 [BUY] [SELL]")
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(order_entry, center_chunks[2]);

    // Right Column: Positions (top) + News (bottom)
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
        .split(main_chunks[2]);

    let positions = Block::default()
        .title("Positions & PNL")
        .borders(Borders::ALL);
    f.render_widget(positions, right_chunks[0]);
    let news = Block::default()
        .title("Live News Feed")
        .borders(Borders::ALL);
    f.render_widget(news, right_chunks[1]);

    // Footer
    let footer = Paragraph::new(
        "RustForge v0.1.0 | Engine: tokio | Threads: 16 | Orders: 0 | session uptime: 02:45:11",
    )
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}
