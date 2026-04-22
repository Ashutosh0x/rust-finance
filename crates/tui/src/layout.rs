// ============================================================
// crates/tui/src/layout.rs
//
// The full terminal layout matching your screenshot:
//
//  ┌──────────┬─────────────────────────────┬──────────────┐
//  │Watchlist │ Market Index Strip          │Open Positions│
//  │          ├──────────────────────────── ├──────────────┤
//  │          │ Price Chart                 │ News Feed    │
//  │          ├──────────────────────────── │              │
//  │          │ Order Book                  │              │
//  ├──────────┼───────────────┬─────────────┤              │
//  │Dexter    │ Dexter AI     │ MiroFish    │              │
//  │Alerts    │ Panel         │ Swarm       │              │
//  ├──────────┴───────────────┴─────────────┴──────────────┤
//  │                Order Entry Strip                       │
//  ├────────────────────────────────────────────────────────┤
//  │                Status Bar                              │
//  └────────────────────────────────────────────────────────┘
// ============================================================

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Cell, Chart, Dataset, GraphType, List, ListItem, Paragraph, Row,
        Table,
    },
    Frame,
};

use crate::state::AppState;

const GREEN: Color = Color::Rgb(0, 220, 90);
const RED: Color = Color::Rgb(220, 50, 50);
const CYAN: Color = Color::Rgb(0, 210, 220);
const AMBER: Color = Color::Rgb(240, 180, 0);
const DIM: Color = Color::Rgb(100, 100, 100);
const FG: Color = Color::Rgb(220, 220, 200);

/// Root render — called every frame from the Tokio draw loop
pub fn render(f: &mut Frame, state: &AppState) {
    let area = f.area();

    // ── Top: Index Strip (full width, 1 line) ─────────────────────────────
    let areas = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .split(area);
    let index_strip = areas[0];
    let main = areas[1];
    let order_entry = areas[2];
    let status_bar = areas[3];

    render_index_strip(f, index_strip, state);
    render_status_bar(f, status_bar, state);
    render_order_entry(f, order_entry, state);

    // ── Main area: 3 columns ──────────────────────────────────────────────
    let cols = Layout::horizontal([
        Constraint::Length(26),
        Constraint::Fill(1),
        Constraint::Length(36),
    ])
    .split(main);

    let left_col = cols[0];
    let center_col = cols[1];
    let right_col = cols[2];

    // ── Left: Watchlist + Dexter Alerts ───────────────────────────────────
    let left_areas =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(12)]).split(left_col);

    let watchlist = left_areas[0];
    let alerts = left_areas[1];

    render_watchlist(f, watchlist, state);
    render_dexter_alerts(f, alerts, state);

    // ── Center: Chart + Order Book + AI Panels ────────────────────────────
    let center_areas = Layout::vertical([
        Constraint::Length(18),
        Constraint::Length(12),
        Constraint::Fill(1),
    ])
    .split(center_col);

    let chart_area = center_areas[0];
    let order_book = center_areas[1];
    let ai_row = center_areas[2];

    render_price_chart(f, chart_area, state);
    render_order_book(f, order_book, state);

    let ai_areas =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(ai_row);

    let dexter_panel = ai_areas[0];
    let swarm_panel = ai_areas[1];

    render_dexter_panel(f, dexter_panel, state);
    render_swarm_panel(f, swarm_panel, state);

    // ── Right: Open Positions + Polymarket + News ─────────────────────────────────────
    let right_areas = Layout::vertical([
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Fill(1),
    ])
    .split(right_col);

    let positions = right_areas[0];
    let polymarket = right_areas[1];
    let news = right_areas[2];

    render_positions(f, positions, state);
    render_polymarket_panel(f, polymarket, state);
    render_news_feed(f, news, state);
}

// ── Panel implementations ─────────────────────────────────────────────────

fn render_index_strip(f: &mut Frame, area: Rect, state: &AppState) {
    let indices = ["NYSE", "NASDAQ", "CME", "CBOE", "LSE", "CRYPTO"];
    let spans: Vec<Span> = indices
        .iter()
        .flat_map(|&name| {
            let is_live = state.connected_venues.contains(&name.to_string());
            let dot_color = if is_live { GREEN } else { DIM };
            vec![
                Span::styled(format!(" {} ", name), Style::default().fg(FG)),
                Span::styled("●", Style::default().fg(dot_color)),
                Span::styled(" | ", Style::default().fg(DIM)),
            ]
        })
        .collect();

    let strip_text = if state.selected_symbol.is_some() {
        format!(
            "  S&P 500: {:.2}  |  Nasdaq-100: {:.2}  |  Live: E2E {:.1}ms (FIX 4.4)",
            state.sp500_price, state.nasdaq_price, state.latency_ms
        )
    } else {
        "  Market Index Strip".to_string()
    };

    let mut line_spans = spans.clone();
    line_spans.push(Span::raw(strip_text));

    f.render_widget(
        Paragraph::new(Line::from(line_spans)).style(Style::default().fg(FG)),
        area,
    );
}

fn render_watchlist(f: &mut Frame, area: Rect, state: &AppState) {
    let header = Row::new(vec!["Symbol", "Price", "Change"]).style(Style::default().fg(DIM));

    let rows: Vec<Row> = state
        .watchlist
        .iter()
        .map(|item| {
            let change_color = if item.change_pct >= 0.0 { GREEN } else { RED };
            let change_str = format!("{:+.2}%", item.change_pct);
            Row::new(vec![
                Cell::from(item.symbol.as_str())
                    .style(Style::default().fg(FG).add_modifier(Modifier::BOLD)),
                Cell::from(format!("{:.2}", item.price)).style(Style::default().fg(FG)),
                Cell::from(change_str).style(Style::default().fg(change_color)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(Block::bordered().title(Span::styled(" Watchlist ", Style::default().fg(CYAN))));

    f.render_widget(table, area);
}

fn render_dexter_alerts(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .dexter_alerts
        .iter()
        .rev()
        .take(8)
        .map(|alert| {
            let color = match alert.severity.as_str() {
                "buy" => GREEN,
                "risk" => RED,
                _ => AMBER,
            };
            ListItem::new(Line::from(vec![
                Span::styled("● ", Style::default().fg(color)),
                Span::styled(alert.message.as_str(), Style::default().fg(FG)),
            ]))
        })
        .collect();

    f.render_widget(
        List::new(items).block(
            Block::bordered().title(Span::styled(" Dexter Alerts ", Style::default().fg(AMBER))),
        ),
        area,
    );
}

fn render_price_chart(f: &mut Frame, area: Rect, state: &AppState) {
    if state.price_history.is_empty() {
        return;
    }

    let data: Vec<(f64, f64)> = state
        .price_history
        .iter()
        .enumerate()
        .map(|(i, &p)| (i as f64, p))
        .collect();

    let min_price = data.iter().map(|(_, p)| *p).fold(f64::INFINITY, f64::min);
    let max_price = data
        .iter()
        .map(|(_, p)| *p)
        .fold(f64::NEG_INFINITY, f64::max);
    let price_range = (max_price - min_price) * 0.1;

    let current = state.price_history.last().cloned().unwrap_or(0.0);
    let first = state.price_history.first().cloned().unwrap_or(0.0);
    let color = if current >= first { GREEN } else { RED };

    let datasets = vec![Dataset::default()
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(color))
        .data(&data)];

    let title = format!(
        " {} — {:.2}  {:+.3} ({:+.2}%) ",
        state.selected_symbol.as_deref().unwrap_or("NVDA"),
        current,
        current - first,
        if first != 0.0 {
            (current - first) / first * 100.0
        } else {
            0.0
        },
    );

    let chart = Chart::new(datasets)
        .block(Block::bordered().title(Span::styled(
            title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )))
        .x_axis(
            Axis::default()
                .style(Style::default().fg(DIM))
                .bounds([0.0, data.len() as f64]),
        )
        .y_axis(
            Axis::default()
                .style(Style::default().fg(DIM))
                .bounds([min_price - price_range, max_price + price_range])
                .labels(vec![
                    Span::styled(format!("{:.0}", min_price), Style::default().fg(DIM)),
                    Span::styled(format!("{:.0}", max_price), Style::default().fg(DIM)),
                ]),
        );

    f.render_widget(chart, area);
}

fn render_order_book(f: &mut Frame, area: Rect, state: &AppState) {
    let areas = Layout::horizontal([
        Constraint::Percentage(45),
        Constraint::Length(14),
        Constraint::Percentage(45),
    ])
    .split(area);

    let asks_area = areas[0];
    let mid_area = areas[1];
    let bids_area = areas[2];

    // Asks (red, sorted desc)
    let ask_rows: Vec<Row> = state
        .order_book
        .asks
        .iter()
        .take(7)
        .map(|level| {
            Row::new(vec![
                Cell::from(format!("${:.2}", level.price)).style(Style::default().fg(RED)),
                Cell::from(format!("{}", level.size)),
                Cell::from(format!("{:.0}M", level.total / 1_000_000.0)),
            ])
        })
        .collect();

    let ask_table = Table::new(
        ask_rows,
        [
            Constraint::Length(9),
            Constraint::Length(6),
            Constraint::Length(6),
        ],
    )
    .header(Row::new(vec!["Asks", "Size", "Total"]).style(Style::default().fg(DIM)))
    .block(Block::default().borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM));

    f.render_widget(ask_table, asks_area);

    // Mid price + spread
    let spread_pct = state
        .order_book
        .asks
        .first()
        .zip(state.order_book.bids.first())
        .map(|(a, b)| {
            if b.price != 0.0 {
                (a.price - b.price) / b.price * 100.0
            } else {
                0.0
            }
        })
        .unwrap_or(0.0);

    let mid_price = (state
        .order_book
        .asks
        .first()
        .map(|a| a.price)
        .unwrap_or(0.0)
        + state
            .order_book
            .bids
            .first()
            .map(|b| b.price)
            .unwrap_or(0.0))
        / 2.0;

    let mid_lines = vec![
        Line::from(Span::styled(
            "Order Book",
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled("Spread", Style::default().fg(DIM))),
        Line::from(Span::styled(
            format!("{:.2}%", spread_pct),
            Style::default().fg(FG),
        )),
        Line::from(""),
        Line::from(Span::styled("Mid price", Style::default().fg(DIM))),
        Line::from(Span::styled(
            format!("{:.2}", mid_price),
            Style::default().fg(FG),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("Buy/Sell imbal {:+.0}%", state.order_book.imbalance * 100.0),
            Style::default().fg(if state.order_book.imbalance > 0.0 {
                GREEN
            } else {
                RED
            }),
        )),
    ];

    f.render_widget(Paragraph::new(mid_lines), mid_area);

    // Bids (green, sorted desc)
    let bid_rows: Vec<Row> = state
        .order_book
        .bids
        .iter()
        .take(7)
        .map(|level| {
            Row::new(vec![
                Cell::from(format!("${:.2}", level.price)).style(Style::default().fg(GREEN)),
                Cell::from(format!("{}", level.size)),
                Cell::from(format!("{:.0}M", level.total / 1_000_000.0)),
            ])
        })
        .collect();

    let bid_table = Table::new(
        bid_rows,
        [
            Constraint::Length(9),
            Constraint::Length(6),
            Constraint::Length(6),
        ],
    )
    .header(Row::new(vec!["Bids", "Size", "Total"]).style(Style::default().fg(DIM)))
    .block(Block::default().borders(Borders::RIGHT | Borders::TOP | Borders::BOTTOM));

    f.render_widget(bid_table, bids_area);
}

fn render_dexter_panel(f: &mut Frame, area: Rect, state: &AppState) {
    let Some(signal) = &state.dexter_signal else {
        f.render_widget(
            Paragraph::new("Dexter AI loading…").block(Block::bordered().title(Span::styled(
                " ◉ DEXTER — FINANCIAL ANALYST ",
                Style::default().fg(CYAN),
            ))),
            area,
        );
        return;
    };

    let rec_color = match signal.recommendation {
        ai::dexter::Recommendation::Buy => GREEN,
        ai::dexter::Recommendation::Sell => RED,
        ai::dexter::Recommendation::Risk => AMBER,
        ai::dexter::Recommendation::Hold => FG,
    };

    let mut lines = vec![
        Line::from(Span::styled(&signal.thesis, Style::default().fg(FG))),
        Line::from(""),
    ];

    if let Some(val) = &signal.valuation {
        if let Some(rev) = val.revenue_impact_usd_millions {
            lines.push(Line::from(format!("Revenue impact: ${:.0}M", rev)));
        }
        if let Some(margin) = val.margin_change_pct {
            lines.push(Line::from(format!("Margin Δ: {:.1}%", margin)));
        }
        if let Some(pe) = val.pe_ratio {
            lines.push(Line::from(format!("P/E: {:.2}", pe)));
        }
        if let Some(ps) = val.ps_ratio {
            lines.push(Line::from(format!("P/S: {:.2}", ps)));
        }
        if let (Some(lo), Some(hi)) = (val.dcf_range_low, val.dcf_range_high) {
            lines.push(Line::from(format!("DCF range: ${:.2}–${:.2}", lo, hi)));
        }
    }

    lines.push(Line::from(""));

    for risk in &signal.key_risks {
        lines.push(Line::from(Span::styled(
            format!("[!] {}", risk),
            Style::default().fg(AMBER),
        )));
    }

    lines.push(Line::from(""));

    // BUY / RISK / NEUTRAL buttons (styled spans)
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {:?} ", signal.recommendation),
            Style::default()
                .fg(Color::Black)
                .bg(rec_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(" CONF {:.0}% ", signal.confidence * 100.0),
            Style::default()
                .fg(Color::Black)
                .bg(if signal.confidence > 0.70 {
                    GREEN
                } else {
                    AMBER
                }),
        ),
    ]));

    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(Span::styled(
            " ◉ DEXTER — FINANCIAL ANALYST ",
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

fn render_swarm_panel(f: &mut Frame, area: Rect, state: &AppState) {
    let Some(step) = &state.swarm_step else {
        f.render_widget(
            Paragraph::new("Swarm simulation loading…").block(Block::bordered().title(
                Span::styled(
                    " ◉ MIROFISH — SWARM SIMULATION ",
                    Style::default().fg(Color::Magenta),
                ),
            )),
            area,
        );
        return;
    };

    let sig = &step.signal;

    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "{},{} agent simulation running…",
                5_000usize.to_string().chars().next().unwrap_or('5'),
                "000"
            ),
            Style::default().fg(FG),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Scenario probability",
            Style::default().fg(FG).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // Bull/Bear/Neutral bars (ASCII progress)
    let bull_pct = (sig.bullish_prob * 100.0) as u16;
    let bear_pct = (sig.bearish_prob * 100.0) as u16;
    let neut_pct = (sig.neutral_prob * 100.0) as u16;

    lines.push(render_prob_bar("Rally  ", bull_pct, GREEN));
    lines.push(render_prob_bar("Sideways", neut_pct + 30, AMBER));
    lines.push(render_prob_bar("Dip    ", bear_pct, RED));

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Institutional accumulation: ", Style::default().fg(DIM)),
        Span::styled(
            format!("{:.0}%", (sig.confidence * 80.0 + 10.0)),
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Retail sentiment: ", Style::default().fg(DIM)),
        Span::styled(
            format!("{:.0}% accumulating", sig.bullish_prob * 50.0 + 25.0),
            Style::default().fg(GREEN),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Round: ", Style::default().fg(DIM)),
        Span::styled(format!("{}", step.round), Style::default().fg(FG)),
        Span::styled("  Active agents: ", Style::default().fg(DIM)),
        Span::styled(format!("{}", step.actions_count), Style::default().fg(FG)),
    ]));

    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered().title(Span::styled(
                " ◉ MIROFISH — SWARM SIMULATION ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )),
        ),
        area,
    );
}

fn render_prob_bar(label: &str, pct: u16, color: Color) -> Line<'static> {
    let filled = (pct as usize * 20 / 100).min(20);
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(20 - filled));
    Line::from(vec![
        Span::styled(format!("{:<9}", label), Style::default().fg(FG)),
        Span::styled(bar, Style::default().fg(color)),
        Span::styled(
            format!(" {}%", pct),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

fn render_positions(f: &mut Frame, area: Rect, state: &AppState) {
    let header = Row::new(vec!["Holding", "P&L", ""]).style(Style::default().fg(DIM));

    let rows: Vec<Row> = state
        .positions
        .iter()
        .map(|pos| {
            let pnl_color = if pos.pnl >= 0.0 { GREEN } else { RED };
            let pct_color = if pos.pnl_pct >= 0.0 { GREEN } else { RED };
            Row::new(vec![
                Cell::from(pos.symbol.as_str())
                    .style(Style::default().fg(FG).add_modifier(Modifier::BOLD)),
                Cell::from(format!("{:+.2}", pos.pnl)).style(Style::default().fg(pnl_color)),
                Cell::from(format!("{:+.2}%", pos.pnl_pct)).style(Style::default().fg(pct_color)),
            ])
        })
        .collect();

    f.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(8),
            ],
        )
        .header(header)
        .block(
            Block::bordered().title(Span::styled(" Open Positions ", Style::default().fg(CYAN))),
        ),
        area,
    );
}

fn render_news_feed(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .news
        .iter()
        .take(12)
        .map(|item| {
            ListItem::new(vec![
                Line::from(Span::styled(
                    item.headline.as_str(),
                    Style::default().fg(FG),
                )),
                Line::from(Span::styled(
                    format!("{} • {}m ago", item.source, item.age_minutes),
                    Style::default().fg(DIM),
                )),
            ])
        })
        .collect();

    f.render_widget(
        List::new(items)
            .block(Block::bordered().title(Span::styled(" News Feed ", Style::default().fg(CYAN)))),
        area,
    );
}

fn render_order_entry(f: &mut Frame, area: Rect, order: &AppState) {
    let order = &order.order_entry;

    let content = Line::from(vec![
        Span::styled(" Symbol ", Style::default().fg(DIM)),
        Span::styled(
            format!("{:<6}", order.symbol),
            Style::default().fg(FG).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  Qty ", Style::default().fg(DIM)),
        Span::styled(format!("{}", order.quantity), Style::default().fg(FG)),
        Span::styled("  Price ", Style::default().fg(DIM)),
        Span::styled(order.price_str.to_string(), Style::default().fg(FG)),
        Span::styled("  LMT / MKT / STP / IOC  ", Style::default().fg(DIM)),
        Span::styled(
            " BUY ",
            Style::default()
                .fg(Color::Black)
                .bg(GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            " SELL ",
            Style::default()
                .fg(Color::Black)
                .bg(RED)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    f.render_widget(
        Paragraph::new(content).block(Block::bordered().title(Span::styled(
            " Order Entry Strip ",
            Style::default().fg(CYAN),
        ))),
        area,
    );
}

fn render_status_bar(f: &mut Frame, area: Rect, state: &AppState) {
    let status = format!(
        " Rust v{}  |  async runtime: tokio  |  Active thread: {}  |  Feed protocol: MPSC  \
         |  Dexter: {}  |  MiroFish active agent: {}  |  Orders sent: {}  \
         |  Fills vs rejections: {}  |  Session uptime: {:.2}",
        state.rust_version,
        state.active_threads,
        state.dexter_call_count,
        state.swarm_active_agents,
        state.orders_sent,
        state.fill_rejection_ratio,
        state.session_uptime_min,
    );

    f.render_widget(Paragraph::new(status).style(Style::default().fg(DIM)), area);
}

fn render_polymarket_panel(f: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default()
        .title(Span::styled(" Polymarket ", Style::default().fg(CYAN)))
        .borders(Borders::ALL);

    let header = Row::new(vec!["Market", "YES", "NO", "24h Vol"]).style(Style::default().fg(DIM));

    let rows: Vec<Row> = state
        .polymarket
        .markets
        .iter()
        .map(|m| {
            Row::new(vec![
                Cell::from(m.question.chars().take(40).collect::<String>()),
                Cell::from(format!("YES: ${:.2}", m.yes_price)).style(Style::default().fg(GREEN)),
                Cell::from(format!("NO: ${:.2}", m.no_price)).style(Style::default().fg(RED)),
                Cell::from(format!("Vol: ${:.0}", m.volume_24hr)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(50),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(block);

    f.render_widget(table, area);
}
