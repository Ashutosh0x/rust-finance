use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render_help_overlay(f: &mut Frame) {
    // Center the box - 60% wide, 80% tall
    let area = centered_rect(60, 80, f.size());

    // Clear the background behind the overlay first
    f.render_widget(Clear, area);

    let lines = vec![
        Line::from(vec![
            Span::styled("  SYSTEM", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  [Ctrl+K]   Kill switch - halt all strategies"),
        Line::from("  [Ctrl+P]   Toggle paper trading mode"),
        Line::from("  [Ctrl+L]   Reload config"),
        Line::from("  [q/Ctrl+C] Quit"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  NAVIGATION", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  [1-6]      Focus panel"),
        Line::from("  [↑/k]      Scroll up"),
        Line::from("  [↓/j]      Scroll down"),
        Line::from("  [Tab]      Next panel"),
        Line::from("  [Shift+Tab] Prev panel"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  TRADING", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  [b]        Open buy dialog"),
        Line::from("  [s]        Open sell dialog"),
        Line::from("  [x]        Cancel selected order"),
        Line::from("  [Ctrl+X]   Cancel ALL orders"),
        Line::from("  [h]        Halve position"),
        Line::from("  [Ctrl+W]   Close full position"),
        Line::from("  [Enter]    Confirm order"),
        Line::from("  [Esc]      Dismiss dialog"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  AI ENGINE", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  [a]        Trigger Dexter analysis"),
        Line::from("  [Shift+A]  Trigger MiroFish swarm"),
        Line::from("  [c]        Cycle confidence (60/75/90%)"),
        Line::from("  [Ctrl+A]   Toggle auto-trade (confirm!)"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  DATA", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
        ]),
        Line::from("  [Ctrl+E]   Export CSV"),
        Line::from("  [Ctrl+B]   Run backtest"),
        Line::from("  [Ctrl+M]   Toggle data source"),
        Line::from("  [F5]       Refresh portfolio"),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [Esc] or [?]  Dismiss this dialog",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    let block = Block::default()
        .title(" COMMAND CHEAT SHEET - RustForge v0.1 ")
        .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(Color::White));

    f.render_widget(paragraph, area);
}

// Helper: carve out a centered rect at (percent_x, percent_y) of the terminal
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_width  = r.width  * percent_x / 100;
    let popup_height = r.height * percent_y / 100;
    Rect {
        x: r.x + (r.width  - popup_width)  / 2,
        y: r.y + (r.height - popup_height) / 2,
        width:  popup_width,
        height: popup_height,
    }
}
