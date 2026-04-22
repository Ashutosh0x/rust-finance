// crates/tui/src/widgets/options_chain.rs
// Interactive Ratatui widget rendering full options boards + GEX surface

use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

// We will mock the external structs here for pure UI widget separation
#[derive(Clone, Debug)]
pub struct MockStrikeRef {
    pub strike: f64,
    pub call_gex: f64,
    pub put_gex: f64,
    pub is_itm: bool,
    pub is_spot: bool,
}

pub struct OptionsChainWidget {
    pub symbol: String,
    pub current_spot: f64,
    pub strikes: Vec<MockStrikeRef>,
    pub selected_index: usize,
}

impl OptionsChainWidget {
    pub fn new(symbol: &str, spot: f64, strikes: Vec<MockStrikeRef>) -> Self {
        Self {
            symbol: symbol.to_string(),
            current_spot: spot,
            strikes,
            selected_index: 0,
        }
    }

    pub fn unselect(&mut self) {
        self.selected_index = 0;
    }

    pub fn next(&mut self) {
        if self.strikes.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1).min(self.strikes.len() - 1);
    }

    pub fn previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let header = Row::new(vec![
            "Call GEX ($M)".to_string(),
            "Strike".to_string(),
            "Put GEX ($M)".to_string(),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);

        let rows = self.strikes.iter().enumerate().map(|(i, strike)| {
            let mut style = Style::default();

            // Highlight selected row
            if i == self.selected_index {
                style = style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
            }
            // Highlight At-The-Money row
            if strike.is_spot {
                style = style.bg(Color::DarkGray);
            }

            let call_cell = Cell::from(format!("{:.1}", strike.call_gex / 1_000_000.0)).style(
                Style::default().fg(if strike.call_gex > 0.0 {
                    Color::Green
                } else {
                    Color::White
                }),
            );

            let strike_cell = Cell::from(format!("{:.2}", strike.strike))
                .style(Style::default().add_modifier(Modifier::BOLD));

            let put_cell = Cell::from(format!("{:.1}", strike.put_gex / 1_000_000.0)).style(
                Style::default().fg(if strike.put_gex < 0.0 {
                    Color::Red
                } else {
                    Color::White
                }),
            );

            Row::new(vec![call_cell, strike_cell, put_cell]).style(style)
        });

        let widths = [
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ];

        let table_block = Block::default().borders(Borders::ALL).title(format!(
            " {} Options Chain (GEX) — Spot: {:.2} ",
            self.symbol, self.current_spot
        ));

        let table = Table::new(rows, widths)
            .header(header)
            .block(table_block)
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol(">> ");

        frame.render_widget(table, area);
    }
}
