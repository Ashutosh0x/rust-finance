use crate::app::{KeyField, SetupState};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

impl SetupState {
    pub fn new() -> Self {
        Self {
            fields: vec![
                KeyField {
                    name: "FINNHUB_API_KEY",
                    label: "Finnhub API Key",
                    value: String::new(),
                    required: true,
                    masked: true,
                    hint: "https://finnhub.io -> Dashboard -> API Key",
                },
                KeyField {
                    name: "ALPACA_API_KEY",
                    label: "Alpaca Key ID",
                    value: String::new(),
                    required: true,
                    masked: false, // Key ID is not secret
                    hint: "https://app.alpaca.markets -> API Keys",
                },
                KeyField {
                    name: "ALPACA_SECRET_KEY",
                    label: "Alpaca Secret Key",
                    value: String::new(),
                    required: true,
                    masked: true,
                    hint: "Shown once on generation -- paste carefully",
                },
                KeyField {
                    name: "ANTHROPIC_API_KEY",
                    label: "Anthropic API Key (optional)",
                    value: String::new(),
                    required: false,
                    masked: true,
                    hint: "Enables AI analysis -- console.anthropic.com",
                },
                KeyField {
                    name: "SOL_PRIVATE_KEY",
                    label: "Solana Private Key (optional)",
                    value: String::new(),
                    required: false,
                    masked: true,
                    hint: "Base58 encoded -- experimental feature",
                },
            ],
            active_field: 0,
            error_msg: None,
            show_confirmation: false,
        }
    }

    pub fn required_satisfied(&self) -> bool {
        self.fields
            .iter()
            .all(|f| !f.required || !f.value.trim().is_empty())
    }
}

pub fn render_setup(f: &mut Frame, state: &SetupState) {
    // Center the setup panel
    let area = centered_rect(60, 70, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" [ RustForge ] -> First Run Setup ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Split inner area: header + fields + footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            std::iter::once(Constraint::Length(3)) // header
                .chain(state.fields.iter().map(|_| Constraint::Length(3)))
                .chain(std::iter::once(Constraint::Length(3))) // error/status
                .chain(std::iter::once(Constraint::Min(0))) // footer
                .collect::<Vec<_>>(),
        )
        .split(inner);

    // Header
    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            "Enter your API keys below. Required fields marked with *",
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::styled(
            "Tab/Up/Down = navigate  |  Enter = submit  |  Esc = skip optional",
            Style::default().fg(Color::DarkGray),
        )),
    ]);
    f.render_widget(header, chunks[0]);

    // Render each field
    for (i, field) in state.fields.iter().enumerate() {
        let is_active = i == state.active_field;
        let chunk = chunks[i + 1];

        let label = format!(
            "{}{}: ",
            if field.required { "* " } else { "  " },
            field.label,
        );

        let display_value = if field.masked && !field.value.is_empty() && !is_active {
            "*".repeat(field.value.len().min(32))
        } else if field.masked && !field.value.is_empty() && is_active {
            // Show last 4 chars while typing
            let v = &field.value;
            if v.len() > 4 {
                format!("{}{}", "*".repeat(v.len() - 4), &v[v.len() - 4..])
            } else {
                v.clone()
            }
        } else if field.value.is_empty() {
            field.hint.to_string()
        } else {
            field.value.clone()
        };

        let style = if is_active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if !field.required && field.value.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else if field.required && field.value.is_empty() {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::Green)
        };

        let value_style = if field.value.is_empty() {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC)
        } else {
            style
        };

        let line = Line::from(vec![
            Span::styled(label, style),
            Span::styled(display_value, value_style),
            if is_active {
                Span::styled("|", Style::default().fg(Color::Yellow)) // cursor
            } else {
                Span::raw("")
            },
        ]);

        let input_block = Block::default()
            .borders(if is_active {
                Borders::ALL
            } else {
                Borders::BOTTOM
            })
            .border_style(if is_active {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            });

        let paragraph = Paragraph::new(line).block(input_block);
        f.render_widget(paragraph, chunk);
    }

    // Error message
    let error_idx = state.fields.len() + 1;
    if let Some(ref err) = state.error_msg {
        let err_widget = Paragraph::new(Span::styled(
            format!("! {}", err),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
        f.render_widget(err_widget, chunks[error_idx]);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub enum SetupAction {
    Continue, // Stay on setup screen
    Submit,   // Keys ready, transition to dashboard
    Quit,
}

pub fn handle_setup_key(key: KeyEvent, state: &mut SetupState) -> SetupAction {
    match key.code {
        // Navigate fields
        KeyCode::Tab | KeyCode::Down => {
            state.active_field = (state.active_field + 1) % state.fields.len();
            state.error_msg = None;
            SetupAction::Continue
        }
        KeyCode::BackTab | KeyCode::Up => {
            state.active_field = if state.active_field == 0 {
                state.fields.len() - 1
            } else {
                state.active_field - 1
            };
            state.error_msg = None;
            SetupAction::Continue
        }

        // Type characters into active field
        KeyCode::Char(c) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                state.fields[state.active_field].value.push(c);
                state.error_msg = None;
            } else if c == 'u' || c == 'w' {
                state.fields[state.active_field].value.clear();
            }
            SetupAction::Continue
        }

        // Delete
        KeyCode::Backspace => {
            state.fields[state.active_field].value.pop();
            SetupAction::Continue
        }

        // Submit
        KeyCode::Enter => {
            if state.required_satisfied() {
                SetupAction::Submit
            } else {
                let missing: Vec<&str> = state
                    .fields
                    .iter()
                    .filter(|f| f.required && f.value.trim().is_empty())
                    .map(|f| f.label)
                    .collect();
                state.error_msg = Some(format!("Missing: {}", missing.join(", ")));
                SetupAction::Continue
            }
        }

        KeyCode::Esc => SetupAction::Quit,

        _ => SetupAction::Continue,
    }
}
