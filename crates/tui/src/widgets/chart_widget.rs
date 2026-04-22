// crates/tui/src/widgets/chart_widget.rs
//
// Bloomberg-style interactive price chart with:
// - Gradient-shaded area chart
// - Volume histogram below price
// - Stats legend overlay (Day Session, Last, High, Low, Average)
// - Zoom and scroll support
// - Crosshair cursor support
// - Time range cycling (1D / 1W / 1M / 1Y / ALL)

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Line as CanvasLine, Rectangle},
        Block, BorderType, Borders, Paragraph,
    },
    Frame,
};

// ── Color palette ─────────────────────────────────────────────────────────────

const BG: Color = Color::Rgb(10, 12, 15);
const BORDER: Color = Color::Rgb(30, 37, 48);
const TEXT_PRIMARY: Color = Color::Rgb(226, 232, 240);
const TEXT_SECONDARY: Color = Color::Rgb(148, 163, 184);
const TEXT_DIM: Color = Color::Rgb(80, 90, 100);
const CHART_GREEN: Color = Color::Rgb(0, 200, 180); // Teal line
const CHART_FILL_TOP: Color = Color::Rgb(0, 80, 70); // Area fill darker
#[allow(dead_code)]
const CHART_FILL_BOT: Color = Color::Rgb(0, 40, 35); // Area fill darkest
const VOLUME_BAR: Color = Color::Rgb(60, 70, 80); // Volume bar gray
const VOLUME_AVG: Color = Color::Rgb(74, 222, 128); // Volume SMAVG green
const GREEN: Color = Color::Rgb(74, 222, 128);
const RED: Color = Color::Rgb(248, 113, 113);
#[allow(dead_code)]
const BLUE_ACCENT: Color = Color::Rgb(0, 150, 220);
#[allow(dead_code)]
const ORANGE: Color = Color::Rgb(249, 115, 22);

// ── Time Ranges ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeRange {
    Day1,
    Week1,
    Month1,
    Month6,
    Year1,
    All,
}

impl TimeRange {
    pub fn label(&self) -> &str {
        match self {
            TimeRange::Day1 => "1D",
            TimeRange::Week1 => "1W",
            TimeRange::Month1 => "1M",
            TimeRange::Month6 => "6M",
            TimeRange::Year1 => "1Y",
            TimeRange::All => "ALL",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            TimeRange::Day1 => TimeRange::Week1,
            TimeRange::Week1 => TimeRange::Month1,
            TimeRange::Month1 => TimeRange::Month6,
            TimeRange::Month6 => TimeRange::Year1,
            TimeRange::Year1 => TimeRange::All,
            TimeRange::All => TimeRange::Day1,
        }
    }
}

// ── Chart State ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChartState {
    /// Current zoom level (1.0 = no zoom, 2.0 = 2x zoomed in)
    pub zoom: f64,
    /// Horizontal scroll offset (0.0 = rightmost / latest data)
    pub scroll_offset: f64,
    /// Current time range view
    pub time_range: TimeRange,
    /// Crosshair X position (None = no crosshair)
    pub crosshair_x: Option<f64>,
}

impl Default for ChartState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            scroll_offset: 0.0,
            time_range: TimeRange::Year1,
            crosshair_x: None,
        }
    }
}

impl ChartState {
    pub fn zoom_in(&mut self) {
        self.zoom = (self.zoom * 1.3).min(10.0);
    }

    pub fn zoom_out(&mut self) {
        self.zoom = (self.zoom / 1.3).max(0.2);
    }

    pub fn scroll_left(&mut self) {
        let step = 5.0 / self.zoom;
        self.scroll_offset = (self.scroll_offset + step).max(0.0);
    }

    pub fn scroll_right(&mut self) {
        let step = 5.0 / self.zoom;
        self.scroll_offset = (self.scroll_offset - step).max(0.0);
    }

    pub fn cycle_time_range(&mut self) {
        self.time_range = self.time_range.next();
        self.zoom = 1.0;
        self.scroll_offset = 0.0;
    }
}

// ── Chart Stats ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChartStats {
    pub last_price: f64,
    pub high_price: f64,
    pub high_date: String,
    pub low_price: f64,
    pub low_date: String,
    pub average: f64,
    pub volume: f64,
    pub volume_smavg: f64,
    pub market_cap: f64,
    pub price_change: f64,
    pub price_change_pct: f64,
}

impl Default for ChartStats {
    fn default() -> Self {
        Self {
            last_price: 1461.98,
            high_price: 1461.98,
            high_date: "02/09/12".to_string(),
            low_price: 1400.0,
            low_date: "06/10/11".to_string(),
            average: 1430.0,
            volume: 11502.2,
            volume_smavg: 48.048,
            market_cap: 74392.0,
            price_change: 0.031,
            price_change_pct: 2.92,
        }
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Render the full Bloomberg-style chart with price area + volume histogram
pub fn render_chart(
    f: &mut Frame,
    area: Rect,
    price_data: &[(f64, f64)],
    volume_data: &[(f64, f64)],
    chart_state: &ChartState,
    stats: &ChartStats,
) {
    if area.height < 6 || area.width < 20 {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .border_type(BorderType::Plain)
        .title(" Price Chart ")
        .title_style(Style::default().fg(TEXT_SECONDARY))
        .style(Style::default().bg(BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Split: stats legend (2 lines) | price chart (70%) | volume chart (30%)
    let chart_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),      // Stats legend
            Constraint::Percentage(65), // Price chart
            Constraint::Percentage(35), // Volume histogram
        ])
        .split(inner);

    render_stats_legend(f, chart_chunks[0], stats, chart_state);
    render_price_area(f, chart_chunks[1], price_data, chart_state, stats);
    render_volume_area(f, chart_chunks[2], volume_data, chart_state, stats);
}

/// The stats/legend overlay matching Bloomberg style
fn render_stats_legend(f: &mut Frame, area: Rect, stats: &ChartStats, _state: &ChartState) {
    let change_color = if stats.price_change >= 0.0 {
        GREEN
    } else {
        RED
    };
    let change_sign = if stats.price_change >= 0.0 { "+" } else { "" };

    if area.width < 40 {
        let line = Line::from(vec![
            Span::styled(
                format!("{:.2} ", stats.last_price),
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "{}${:.3} ({}{:.2}%)",
                    change_sign,
                    stats.price_change.abs(),
                    change_sign,
                    stats.price_change_pct
                ),
                Style::default().fg(change_color),
            ),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    // Right-side info: compute padding to push Volume/MarketCap toward right
    let vol_str = format_number_with_commas(stats.volume);
    let mcap_str = format_number_with_commas(stats.market_cap);

    let price_text = format!(" {:.2}", stats.last_price);
    let change_text = format!(
        " {}${:.3} ({}{:.2}%)",
        change_sign,
        stats.price_change.abs(),
        change_sign,
        stats.price_change_pct
    );
    let vol_label = format!("Volume:   {}B", vol_str);
    let svgline = " SVG polyline #8";
    let mcap_label = format!("Market Cap: ${} MB", mcap_str);

    let used_line1 = price_text.len() + change_text.len() + vol_label.len();
    let pad1 = if (area.width as usize) > used_line1 + 4 {
        area.width as usize - used_line1 - 4
    } else {
        2
    };
    let used_line2 = svgline.len() + mcap_label.len();
    let pad2 = if (area.width as usize) > used_line2 + 4 {
        area.width as usize - used_line2 - 4
    } else {
        2
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                price_text,
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(change_text, Style::default().fg(change_color)),
            Span::raw(" ".repeat(pad1)),
            Span::styled(vol_label, Style::default().fg(TEXT_PRIMARY)),
        ]),
        Line::from(vec![
            Span::styled(svgline.to_string(), Style::default().fg(TEXT_DIM)),
            Span::raw(" ".repeat(pad2)),
            Span::styled(mcap_label, Style::default().fg(TEXT_PRIMARY)),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

/// Render the main price area chart (line with Braille markers)
fn render_price_area(
    f: &mut Frame,
    area: Rect,
    data: &[(f64, f64)],
    state: &ChartState,
    _stats: &ChartStats,
) {
    if data.is_empty() || area.height < 3 {
        return;
    }

    // Calculate visible window based on zoom and scroll
    let total_points = data.len() as f64;
    let visible_width = total_points / state.zoom;
    let x_max = total_points - state.scroll_offset;
    let x_min = (x_max - visible_width).max(0.0);

    // Filter data to visible range
    let visible_data: Vec<(f64, f64)> = data
        .iter()
        .filter(|(x, _)| *x >= x_min && *x <= x_max)
        .cloned()
        .collect();

    if visible_data.is_empty() {
        return;
    }

    // Compute Y bounds with padding
    let y_min = visible_data
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::INFINITY, f64::min);
    let y_max = visible_data
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);
    let y_padding = (y_max - y_min) * 0.1;
    let y_low = y_min - y_padding;
    let y_high = y_max + y_padding;

    let canvas = Canvas::default()
        .marker(symbols::Marker::Braille)
        .x_bounds([x_min, x_max])
        .y_bounds([y_low, y_high])
        .paint(move |ctx| {
            // Fill area (vertical lines down to y_low)
            for p in &visible_data {
                ctx.draw(&CanvasLine {
                    x1: p.0,
                    y1: y_low,
                    x2: p.0,
                    y2: y_low + (p.1 - y_low) * 0.3,
                    color: CHART_FILL_TOP,
                });
            }
            // Main price line
            for i in 0..visible_data.len().saturating_sub(1) {
                let p1 = visible_data[i];
                let p2 = visible_data[i + 1];
                ctx.draw(&CanvasLine {
                    x1: p1.0,
                    y1: p1.1,
                    x2: p2.0,
                    y2: p2.1,
                    color: CHART_GREEN,
                });
            }
        });

    f.render_widget(canvas, area);
}

/// Render the volume histogram area
fn render_volume_area(
    f: &mut Frame,
    area: Rect,
    volume_data: &[(f64, f64)],
    state: &ChartState,
    stats: &ChartStats,
) {
    if volume_data.is_empty() || area.height < 3 {
        return;
    }

    // Volume header
    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let chart_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    let header = Line::from(vec![
        Span::styled(" ░ ", Style::default().fg(VOLUME_BAR)),
        Span::styled("Volume       ", Style::default().fg(TEXT_SECONDARY)),
        Span::styled(
            format!("{:.3}m   ", stats.volume),
            Style::default().fg(TEXT_PRIMARY),
        ),
        Span::styled(" █ ", Style::default().fg(VOLUME_AVG)),
        Span::styled(
            "SMAVG Volume Histogram(15) ",
            Style::default().fg(TEXT_SECONDARY),
        ),
        Span::styled(
            format!("{:.3}m", stats.volume_smavg),
            Style::default().fg(TEXT_PRIMARY),
        ),
    ]);
    f.render_widget(Paragraph::new(header), header_area);

    // Calculate visible window
    let total_points = volume_data.len() as f64;
    let visible_width = total_points / state.zoom;
    let x_max = total_points - state.scroll_offset;
    let x_min = (x_max - visible_width).max(0.0);

    let visible_vol: Vec<(f64, f64)> = volume_data
        .iter()
        .filter(|(x, _)| *x >= x_min && *x <= x_max)
        .cloned()
        .collect();

    // Volume SMAVG line (15-period simple moving average)
    let smavg_data: Vec<(f64, f64)> = compute_smavg(&visible_vol, 15);

    if visible_vol.is_empty() {
        return;
    }

    let v_max = visible_vol
        .iter()
        .map(|(_, v)| *v)
        .fold(f64::NEG_INFINITY, f64::max);

    let canvas = Canvas::default()
        .marker(symbols::Marker::Block)
        .x_bounds([x_min, x_max])
        .y_bounds([0.0, v_max * 1.1])
        .paint(move |ctx| {
            // Bars
            for p in &visible_vol {
                ctx.draw(&Rectangle {
                    x: p.0,
                    y: 0.0,
                    width: 1.0,
                    height: p.1,
                    color: VOLUME_BAR,
                });
            }
            // SMAVG line
            for i in 0..smavg_data.len().saturating_sub(1) {
                let p1 = smavg_data[i];
                let p2 = smavg_data[i + 1];
                ctx.draw(&CanvasLine {
                    x1: p1.0,
                    y1: p1.1,
                    x2: p2.0,
                    y2: p2.1,
                    color: VOLUME_AVG,
                });
            }
        });

    f.render_widget(canvas, chart_area);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[allow(dead_code)]
fn generate_time_labels(
    _x_min: f64,
    _x_max: f64,
    width: u16,
    time_range: TimeRange,
) -> Vec<Span<'static>> {
    let months = match time_range {
        TimeRange::Day1 => vec![
            "9:30", "10:00", "11:00", "12:00", "13:00", "14:00", "15:00", "16:00",
        ],
        TimeRange::Week1 => vec!["Mon", "Tue", "Wed", "Thu", "Fri"],
        TimeRange::Month1 => vec!["W1", "W2", "W3", "W4"],
        TimeRange::Month6 => vec!["Jan", "Feb", "Mar", "Apr", "May", "Jun"],
        TimeRange::Year1 => vec![
            "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec", "Jan", "Feb",
        ],
        TimeRange::All => vec!["2020", "2021", "2022", "2023", "2024", "2025"],
    };

    let max_labels = (width as usize / 10).max(3).min(months.len());
    let step = months.len() / max_labels.max(1);

    months
        .iter()
        .enumerate()
        .filter(|(i, _)| *i % step == 0)
        .map(|(_, m)| Span::styled(m.to_string(), Style::default().fg(TEXT_DIM)))
        .take(max_labels)
        .collect()
}

fn compute_smavg(data: &[(f64, f64)], period: usize) -> Vec<(f64, f64)> {
    if data.len() < period {
        return data.to_vec();
    }

    let mut result = Vec::with_capacity(data.len());
    for i in 0..data.len() {
        if i < period - 1 {
            result.push(data[i]);
        } else {
            let sum: f64 = data[i + 1 - period..=i].iter().map(|(_, v)| v).sum();
            result.push((data[i].0, sum / period as f64));
        }
    }
    result
}

fn format_number_with_commas(n: f64) -> String {
    let whole = n.trunc() as i64;
    let frac_part = ((n - n.trunc()).abs() * 10.0).round() as u64;
    let s = whole.abs().to_string();
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    if whole < 0 {
        result = format!("-{}", result);
    }
    if frac_part > 0 {
        format!("{}.{}", result, frac_part)
    } else {
        result
    }
}

#[allow(dead_code)]
fn format_volume(v: f64) -> String {
    if v >= 1_000_000.0 {
        format!("{:.0}m", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("{:.0}k", v / 1_000.0)
    } else {
        format!("{:.0}", v)
    }
}
