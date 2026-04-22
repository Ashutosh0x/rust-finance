use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

pub fn render_help_overlay(f: &mut Frame) {
    let area = centered_rect(60, 80, f.area());
    f.render_widget(Clear, area);

    let lines = vec![
        Line::from(vec![Span::styled(
            "  SYSTEM",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  [K]        Kill switch — halt all trading"),
        Line::from("  [M]        Toggle paper / live mode"),
        Line::from("  [q/Ctrl+C] Quit application"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  NAVIGATION",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  [1-6]        Focus panel"),
        Line::from("  [↑]          Scroll up"),
        Line::from("  [↓/j]        Scroll down"),
        Line::from("  [Tab]        Next panel"),
        Line::from("  [Shift+Tab]  Prev panel"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  TRADING",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  [B]        Open buy dialog"),
        Line::from("  [S]        Open sell dialog"),
        Line::from("  [x]        Cancel selected order"),
        Line::from("  [Ctrl+X]   Cancel ALL orders"),
        Line::from("  [Ctrl+W]   Close full position"),
        Line::from("  [Enter]    Confirm order"),
        Line::from("  [Esc]      Dismiss dialog"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  AI ENGINE",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  [D]        Trigger Dexter AI analysis"),
        Line::from("  [F]        Run Mirofish swarm simulation"),
        Line::from("  [C]        Cycle confidence (60/75/90%)"),
        Line::from("  [Ctrl+A]   Toggle auto-trade (confirm!)"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  CHART",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  [+/-]      Zoom in/out"),
        Line::from("  [Shift+←/→] Scroll chart"),
        Line::from("  [T]        Cycle time range (1D/1W/1M/1Y)"),
        Line::from("  [Scroll]   Mouse wheel zoom"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  DATA",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  [E]        Export CSV"),
        Line::from("  [Ctrl+B]   Run backtest"),
        Line::from("  [F5]       Refresh portfolio"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  [Esc] or [?]  Dismiss this dialog",
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    let block = Block::default()
        .title(" COMMAND CHEAT SHEET — RustForge v0.5 ")
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .border_type(BorderType::Rounded);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(Color::White).bg(Color::Rgb(10, 12, 15)));

    f.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let popup_height = r.height * percent_y / 100;
    Rect {
        x: r.x + (r.width - popup_width) / 2,
        y: r.y + (r.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    }
}
