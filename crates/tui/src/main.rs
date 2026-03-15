use std::{
    io,
    time::{Duration},
};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{canvas::*, *},
    text::Line,
};
use tokio::net::TcpStream;
use tokio::io::{AsyncBufReadExt, BufReader};
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

use app::App;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    // Event Bus Connection Manager
    let (tx, mut rx) = mpsc::channel::<String>(100);
    
    tokio::spawn(async move {
        loop {
            match TcpStream::connect("127.0.0.1:7001").await {
                Ok(stream) => {
                    let _ = tx.send("Connected to Daemon (127.0.0.1:7001)".to_string()).await;
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    
                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) => break, // EOF
                            Ok(_) => {
                                // Real implementation would parse event and update App state
                            }
                            Err(_) => break, // Error
                        }
                    }
                    let _ = tx.send("Daemon Disconnected. Reconnecting...".to_string()).await;
                }
                Err(_) => {
                    let _ = tx.send("Connection Failed. Retrying...".to_string()).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    loop {
        // Non-blocking UI update from network thread
        while let Ok(msg) = rx.try_recv() {
            app.connection_status = msg;
        }

        terminal.draw(|f| ui(f, &app))?;

        if crossterm::event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                event_handler::handle_key(&mut app, key);
            }
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
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
    let text = Line::from(vec![
        Span::styled(" 🟧 RUST TERMINAL   ", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)),
        Span::styled("NYSE ", Style::default().fg(GREEN)), Span::raw("●  "),
        Span::styled("NASDAQ ", Style::default().fg(GREEN)), Span::raw("●  "),
        Span::styled("CME ", Style::default().fg(GREEN)), Span::raw("●  "),
        Span::styled("CBOE ", Style::default().fg(ORANGE)), Span::raw("●  "),
        Span::styled("LSE ", Style::default().fg(TEXT_SECONDARY)), Span::raw("◐  "),
        Span::styled("CRYPTO ", Style::default().fg(Color::Yellow)), Span::raw("●  "),
        Span::raw("                                        "),
        Span::styled("● ", Style::default().fg(Color::Cyan)),
        Span::styled(format!(" Live: E2E: 1.8ms | Status: {} | 11 2:30 EST", app.connection_status), Style::default().fg(TEXT_SECONDARY)),
    ]);
    f.render_widget(Paragraph::new(text).style(Style::default().bg(BG)), area);
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

    draw_left_col(f, cols[0]);
    draw_center_col(f, cols[1], app);
    draw_right_col(f, cols[2]);
}

fn draw_left_col(f: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .split(area);

    draw_watchlist(f, chunks[0]);
    draw_dexter_alerts(f, chunks[1]);
}

fn draw_watchlist(f: &mut Frame, area: Rect) {
    let list = vec![
        ("AAPL", "AAPL Inc.", "322.50", "+1.58%", GREEN),
        ("NVDA", "NVDA Commun...", "297.75", "+0.32%", GREEN),
        ("TSLA", "Corporates, Inc...", "103.35", "-0.31%", RED),
        ("AAPL", "Marketsion-Am...", "83.50", "-0.17%", RED),
        ("NVDA", "Company, Inc.", "119.50", "-1.27%", RED),
        ("NVDA", "Elveratrin Corp...", "223.90", "-0.79%", RED),
        ("AAPL", "Bustein Corp.", "52.55", "-0.33%", RED),
        ("TSLA", "Apple Corporat...", "308.83", "-0.32%", RED),
        ("NVDA", "Apple Corporat...", "111.93", "+1.22%", GREEN),
        ("TSLA", "Thnancial Fina...", "52.27", "+0.12%", GREEN),
        ("NVDA", "Nantwender S...", "15.97", "-0.12%", RED),
        ("TSLA", "Chsco Amergia ...", "275.19", "+0.12%", GREEN),
        ("AAPL", "Simpers, Inc.", "38.20", "-0.30%", RED),
        ("TSLA", "Twereen", "135.15", "-0.38%", RED),
    ];

    let rows: Vec<Row> = list.into_iter().map(|(sym, desc, price, chg, color)| {
        Row::new(vec![
            Cell::from(Line::from(vec![
                Span::styled(format!("{}", sym), Style::default().fg(TEXT_PRIMARY)),
                Span::styled(format!("\n{}", desc), Style::default().fg(TEXT_SECONDARY)),
            ])),
            Cell::from(Span::styled(price.to_string(), Style::default().fg(TEXT_PRIMARY))),
            Cell::from(Span::styled(chg.to_string(), Style::default().fg(color))),
        ]).height(2)
    }).collect();

    let widths = [
        Constraint::Length(15),   // symbol + description
        Constraint::Length(8),    // price
        Constraint::Length(8),    // change
    ];

    let table = Table::new(rows, widths)
        .header(Row::new(vec!["Symbol", "Price", "Change"]).style(Style::default().fg(TEXT_SECONDARY)))
        .block(block_with_title("Watchlist"));

    f.render_widget(table, area);
}

fn draw_dexter_alerts(f: &mut Frame, area: Rect) {
    let alerts = vec![
        "EV subsidy catalyst detected - TSLA",
        "EV subsidy catalyst detected - TSLA",
        "EV subsidy catalyst detected - TSLA",
        "EV subsidy catalyst detected - TSLA",
        "Overbought RSI 78.4 - NVDA",
        "Overbought RSI 78.4 - NVDA",
        "Overbought RSI 78.4 - NVDA",
        "Overbougy catalyst detected - TSLA",
    ];

    let items: Vec<ListItem> = alerts.into_iter().map(|a| {
        ListItem::new(Line::from(vec![
            Span::styled("● ", Style::default().fg(BLUE)),
            Span::styled(a, Style::default().fg(TEXT_PRIMARY)),
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
            Constraint::Length(13), // Chart
            Constraint::Min(10),     // Order Book
            Constraint::Length(12),  // Dexter & Mirofish
            Constraint::Length(3),  // Order Entry
        ])
        .split(area);

    draw_index_strip(f, chunks[0]);
    draw_price_chart(f, chunks[1], app);
    draw_order_book(f, chunks[2]);

    let bottom_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);
    
    draw_dexter_analyst(f, bottom_split[0]);
    draw_mirofish_sim(f, bottom_split[1]);
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

fn draw_price_chart(f: &mut Frame, area: Rect, app: &App) {
    let block = block_with_title("Price Chart");
    
    // Header overlay inside the chart area
    let header_area = Rect { x: area.x + 1, y: area.y + 1, width: area.width - 2, height: 2 };
    let chart_area = Rect { x: area.x + 1, y: area.y + 3, width: area.width - 2, height: area.height.saturating_sub(4) };

    f.render_widget(block, area);

    // Chart header Text
    let header_text = Line::from(vec![
        Span::styled("1461.98  ", Style::default().fg(TEXT_PRIMARY).add_modifier(Modifier::BOLD)),
        Span::styled("+$0.031 (+2.92%)", Style::default().fg(GREEN)),
        Span::styled("     SVG polyline #8", Style::default().fg(TEXT_SECONDARY)),
        Span::raw("                                           "),
        Span::styled("Volume:       11,502.2B", Style::default().fg(TEXT_SECONDARY)),
    ]);
    f.render_widget(Paragraph::new(header_text), header_area);

    // Actual Chart
    let datasets = vec![
        Dataset::default()
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(GREEN))
            .graph_type(GraphType::Line)
            .data(&app.chart_data),
    ];
    let chart = Chart::new(datasets)
        .x_axis(Axis::default().bounds([0.0, 100.0]).labels(vec![Span::raw("10:00"), Span::raw("08:00"), Span::raw("12:00"), Span::raw("18:00"), Span::raw("12:00"), Span::raw("18:00"), Span::raw("03:00")]).style(Style::default().fg(TEXT_SECONDARY)))
        .y_axis(Axis::default().bounds([1380.0, 1480.0]).style(Style::default().fg(BG))); // Hide Y axis numbers by using BG color
    f.render_widget(chart, chart_area);
}

fn draw_order_book(f: &mut Frame, area: Rect) {
    let rows = vec![
        vec!["$7871.71", "100", "2382M", "$7871.70", "300", "10033M"],
        vec!["$7871.70", "100", "2543M", "$7871.69", "200", "9893M"],
        vec!["$7871.70", "120", "1592M", "$7871.68", "300", "4083M"],
        vec!["$7871.70", "100", "1193M", "$7871.68", "200", "3283M"],
        vec!["$7871.80", "100", "1213M", "$7871.67", "1,000", "3132M"],
        vec!["$7871.80", "360", "2133M", "$7871.66", "1,000", "4282M"],
        vec!["$7871.90", "80",  "593M",  "$7871.65", "400",   "3282M"],
    ];

    let t_rows: Vec<Row> = rows.into_iter().map(|cols| {
        Row::new(vec![
            Cell::from(Span::styled(cols[0], Style::default().fg(RED))),
            Cell::from(cols[1]),
            Cell::from(Span::styled(cols[2], Style::default().fg(RED).add_modifier(Modifier::BOLD))),
            Cell::from(Span::styled(cols[3], Style::default().fg(GREEN))),
            Cell::from(cols[4]),
            Cell::from(Span::styled(cols[5], Style::default().fg(GREEN).add_modifier(Modifier::BOLD))),
        ])
    }).collect();

    let table = Table::new(t_rows, [Constraint::Percentage(16); 6])
        .header(Row::new(vec!["Asks", "Size", "Total", "Bids", "Size", "Total"]).style(Style::default().fg(TEXT_SECONDARY)))
        .block(block_with_title("Order Book"));
    
    f.render_widget(table, area);
}

fn draw_dexter_analyst(f: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Line::from(vec![
            Span::styled("● ", Style::default().fg(BLUE)),
            Span::styled("DEXTER - FINANCIAL ANALYST", Style::default().fg(TEXT_SECONDARY))
        ]))
        .style(Style::default().bg(BG));
        
    let text = vec![
        Line::from("Revenue impact estimates - $44 mi in"),
        Line::from("showing revenue coperates. +35% revenue"),
        Line::from("margin insent scanue, moast L, 42% on"),
        Line::from("margin comparison 50% ≈ 34% on margin."),
        Line::from(""),
        Line::from("Key valuation multiples:"),
        Line::from("- P/E: 10.53"),
        Line::from("- P/S: 2.98"),
        Line::from("- EV/EBITDA: 2.99"),
        Line::from("- DCF fair value range: $3.30 - $7.6B"),
        Line::from(""),
        Line::from(vec![
            Span::styled(" BUY ", Style::default().fg(Color::Black).bg(GREEN).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(" RISK ", Style::default().fg(Color::Black).bg(ORANGE).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(" NEUTRAL ", Style::default().fg(Color::Black).bg(TEXT_SECONDARY).add_modifier(Modifier::BOLD)),
        ]),
    ];
        
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn draw_mirofish_sim(f: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Line::from(vec![
            Span::styled("● ", Style::default().fg(PURPLE)),
            Span::styled("MIROFISH - SWARM SIMULATION", Style::default().fg(TEXT_SECONDARY))
        ]))
        .style(Style::default().bg(BG));
        
    let text = vec![
        Line::from("5,000 agent simulation running..."),
        Line::from(""),
        Line::from("Scenario probability"),
        Line::from(vec![
            Span::raw("Rally    "),
            Span::styled("█████████████████", Style::default().fg(BLUE)),
            Span::styled("░░░░ 77%", Style::default().fg(BORDER)),
        ]),
        Line::from(vec![
            Span::raw("Sideways "),
            Span::styled("███████", Style::default().fg(TEXT_SECONDARY)),
            Span::styled("░░░░░░░░░░░░░░ 30%", Style::default().fg(BORDER)),
        ]),
        Line::from(vec![
            Span::raw("Dip      "),
            Span::styled("██", Style::default().fg(PURPLE)),
            Span::styled("░░░░░░░░░░░░░░░░░░░ 0%", Style::default().fg(BORDER)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("Institutional accumulation: "),
            Span::styled("Hiriting -fl0%", Style::default().fg(GREEN))
        ]),
        Line::from(vec![
            Span::raw("Retail sentiment: "),
            Span::styled("50% accumulaty", Style::default().fg(GREEN))
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

fn draw_right_col(f: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(70),
            Constraint::Length(5),
        ])
        .split(area);

    draw_open_positions(f, chunks[0]);
    draw_news_feed(f, chunks[1]);
    draw_day_pnl(f, chunks[2]);
}

fn draw_open_positions(f: &mut Frame, area: Rect) {
    let rows = vec![
        Row::new(vec!["AAPL", "+222.50", "+1.72%"]).style(Style::default().fg(GREEN)),
        Row::new(vec!["NVDA", "+100.00", " 1.53%"]).style(Style::default().fg(GREEN)),
        Row::new(vec!["NVDA", " +50.00", " 1.15%"]).style(Style::default().fg(GREEN)),
        Row::new(vec!["TSLA", "  $0.00", "-0.23%"]).style(Style::default().fg(RED)),
        Row::new(vec!["CNBC", " -10.00", "-0.27%"]).style(Style::default().fg(RED)),
    ];
    let table = Table::new(rows, [Constraint::Percentage(33); 3])
        .header(Row::new(vec!["Holding", "", "P&L"]).style(Style::default().fg(TEXT_SECONDARY)))
        .block(block_with_title("Open Positions"));
    f.render_widget(table, area);
}

fn draw_news_feed(f: &mut Frame, area: Rect) {
    let news = vec![
        "Reuters • 25m ago",
        "Annols nrex oostionns A EV catalyst signals emitrater to intraday stamite writ informatio...",
        "",
        "Bloomberg • 2m ago",
        "NVDAs nenture 7seceptatons of High status rai (Bloomberg) sponehetots used in the fire hine...",
        "",
        "Bloomberg • 19m ago",
        "Reuters new ha seen hynlcaniks to costrase the news markets",
        "",
        "WSJ • 19m ago",
        "The first news pemptsed the US president in the Boodsimr hanta in matrjrx brands.",
        "",
        "WSJ • 5m ago",
        "WSJ Fish eats a highs choming on new-rother wish to retail accumulato of flattwining at the...",
        "",
        "CNBC • Bloomberg",
        "CNBC Reuters sinteresteds fundnig unroar nalidites on the most chamig search and wits sor...",
    ];
    
    let items: Vec<ListItem> = news.into_iter().map(|n| {
        let style = if n.contains("ago") || n.contains("Bloomberg") { 
            Style::default().fg(TEXT_SECONDARY) 
        } else { 
            Style::default().fg(TEXT_PRIMARY) 
        };
        ListItem::new(Span::styled(n, style))
    }).collect();
    
    f.render_widget(List::new(items).block(block_with_title("News Feed")), area);
}

fn draw_day_pnl(f: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(vec![Span::raw("Day P&L:           "), Span::styled("$10.9GL", Style::default().fg(GREEN))]),
        Line::from(vec![Span::raw("Available power:  "), Span::raw("1,729.8B")]),
    ];
    f.render_widget(Paragraph::new(text).block(Block::default().borders(Borders::NONE)), area);
}

fn draw_bottom_bar(f: &mut Frame, area: Rect) {
    let text = Line::from(vec![
        Span::styled("Rust version: 6.19 | async runtime: tokio | Active thread: 1 | Feed protocol: MPSC | Dexter: 0 | MiroFish active agent: 6 | Orders sent: 15 | Fills vs rejections: 0 | Session uptime: 12:05:32", Style::default().fg(TEXT_SECONDARY))
    ]);
    f.render_widget(Paragraph::new(text).style(Style::default().bg(BG)), area);
}
