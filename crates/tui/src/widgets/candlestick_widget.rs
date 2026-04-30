// crates/tui/src/widgets/candlestick_widget.rs
//
// Professional candlestick chart inspired by scalper-rs:
// - Full OHLC candles (body + wick) with green/red coloring
// - Volume histogram below price chart
// - Price scale on the right edge
// - Current price crosshair line
// - Zoom (Shift+Up/Down) and scroll (Left/Right)
// - Auto-centering when price drifts off-screen

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

// ── Colors ────────────────────────────────────────────────────────────────────

const BG: Color = Color::Rgb(10, 12, 15);
const BORDER: Color = Color::Rgb(30, 37, 48);
const TEXT_PRI: Color = Color::Rgb(226, 232, 240);
const TEXT_SEC: Color = Color::Rgb(148, 163, 184);
const TEXT_DIM: Color = Color::Rgb(80, 90, 100);
const BULL: Color = Color::Rgb(34, 197, 94); // Green candle
const BEAR: Color = Color::Rgb(239, 68, 68); // Red candle
const BULL_DIM: Color = Color::Rgb(22, 101, 52); // Green volume
const BEAR_DIM: Color = Color::Rgb(127, 29, 29); // Red volume
const CROSSHAIR: Color = Color::Rgb(100, 116, 139);
const PRICE_TAG_BG: Color = Color::Rgb(59, 130, 246);

// ── Data Structures ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct Candle {
    pub time: f64, // x-axis index
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Candle {
    pub fn is_bullish(&self) -> bool {
        self.close >= self.open
    }
}

#[derive(Debug, Clone)]
pub struct CandlestickState {
    pub zoom: f64,
    pub scroll_offset: f64,
    pub candle_width: f64,
}

impl Default for CandlestickState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            scroll_offset: 0.0,
            candle_width: 3.0,
        }
    }
}

impl CandlestickState {
    pub fn zoom_in(&mut self) {
        self.zoom = (self.zoom * 1.25).min(8.0);
    }

    pub fn zoom_out(&mut self) {
        self.zoom = (self.zoom / 1.25).max(0.25);
    }

    pub fn scroll_left(&mut self) {
        self.scroll_offset = (self.scroll_offset + 3.0 / self.zoom).max(0.0);
    }

    pub fn scroll_right(&mut self) {
        self.scroll_offset = (self.scroll_offset - 3.0 / self.zoom).max(0.0);
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

pub fn render_candlestick_chart(
    f: &mut Frame,
    area: Rect,
    candles: &[Candle],
    state: &CandlestickState,
    symbol: &str,
) {
    if area.height < 8 || area.width < 20 || candles.is_empty() {
        return;
    }

    // Outer block
    let current_price = candles.last().map(|c| c.close).unwrap_or(0.0);
    let prev_close = if candles.len() >= 2 {
        candles[candles.len() - 2].close
    } else {
        current_price
    };
    let change = current_price - prev_close;
    let change_pct = if prev_close != 0.0 {
        change / prev_close * 100.0
    } else {
        0.0
    };
    let title_color = if change >= 0.0 { BULL } else { BEAR };
    let sign = if change >= 0.0 { "+" } else { "" };

    let title = format!(
        " {} — {:.2}  {}{:.3} ({}{:.2}%) ",
        symbol,
        current_price,
        sign,
        change.abs(),
        sign,
        change_pct
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .border_type(BorderType::Plain)
        .title(Span::styled(
            title,
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Split: price chart (75%) | volume chart (25%)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(75), Constraint::Percentage(25)])
        .split(inner);

    let price_area = chunks[0];
    let volume_area = chunks[1];

    // Determine visible candles
    let total = candles.len() as f64;
    let visible_count = (total / state.zoom).max(5.0);
    let end_idx = (total - state.scroll_offset).max(visible_count);
    let start_idx = (end_idx - visible_count).max(0.0);

    let visible: Vec<Candle> = candles
        .iter()
        .filter(|c| c.time >= start_idx && c.time <= end_idx)
        .cloned()
        .collect();

    if visible.is_empty() {
        return;
    }

    // Price bounds
    let y_min = visible.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let y_max = visible
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let y_pad = (y_max - y_min).max(0.01) * 0.08;
    let y_lo = y_min - y_pad;
    let y_hi = y_max + y_pad;

    // Volume bounds
    let v_max = visible
        .iter()
        .map(|c| c.volume)
        .fold(f64::NEG_INFINITY, f64::max)
        .max(1.0);

    let x_lo = start_idx;
    let x_hi = end_idx;
    let cw = state.candle_width / state.zoom;
    let body_w = (cw * 0.6).max(0.3);

    // ── Price Chart Canvas ────────────────────────────────────────────────
    let vis_price = visible.clone();
    let price_canvas = Canvas::default()
        .marker(symbols::Marker::Block)
        .x_bounds([x_lo, x_hi])
        .y_bounds([y_lo, y_hi])
        .paint(move |ctx| {
            // Current price crosshair
            ctx.draw(&CanvasLine {
                x1: x_lo,
                y1: current_price,
                x2: x_hi,
                y2: current_price,
                color: CROSSHAIR,
            });

            // Draw each candle
            for candle in &vis_price {
                let color = if candle.is_bullish() { BULL } else { BEAR };

                // Wick (high to low)
                ctx.draw(&CanvasLine {
                    x1: candle.time,
                    y1: candle.low,
                    x2: candle.time,
                    y2: candle.high,
                    color,
                });

                // Body (open to close)
                let body_top = candle.open.max(candle.close);
                let body_bot = candle.open.min(candle.close);
                let body_h = (body_top - body_bot).max(0.001);

                ctx.draw(&Rectangle {
                    x: candle.time - body_w / 2.0,
                    y: body_bot,
                    width: body_w,
                    height: body_h,
                    color,
                });
            }
        });

    f.render_widget(price_canvas, price_area);

    // ── Price scale overlay (right edge) ──────────────────────────────────
    if price_area.width > 12 {
        let scale_w = 10u16;
        let scale_area = Rect {
            x: price_area.x + price_area.width.saturating_sub(scale_w),
            y: price_area.y,
            width: scale_w,
            height: price_area.height,
        };

        let steps = 5usize;
        let mut scale_lines: Vec<Line> = Vec::new();
        let line_spacing = price_area.height as usize / (steps + 1);
        for i in 0..=steps {
            let frac = 1.0 - (i as f64 / steps as f64);
            let price = y_lo + frac * (y_hi - y_lo);
            let target_row = (i * line_spacing).min(price_area.height.saturating_sub(1) as usize);
            while scale_lines.len() < target_row {
                scale_lines.push(Line::from(""));
            }
            scale_lines.push(Line::from(Span::styled(
                format!("{:>9.2}", price),
                Style::default().fg(TEXT_DIM),
            )));
        }
        f.render_widget(Paragraph::new(scale_lines), scale_area);

        // Current price tag
        let price_frac = (current_price - y_lo) / (y_hi - y_lo);
        let price_row = ((1.0 - price_frac) * price_area.height as f64) as u16;
        if price_row < price_area.height {
            let tag_area = Rect {
                x: price_area.x + price_area.width.saturating_sub(scale_w),
                y: price_area.y + price_row,
                width: scale_w,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!("{:>9.4}", current_price),
                    Style::default()
                        .fg(Color::White)
                        .bg(PRICE_TAG_BG)
                        .add_modifier(Modifier::BOLD),
                )),
                tag_area,
            );
        }
    }

    // ── Volume Histogram ──────────────────────────────────────────────────
    let vis_vol = visible;
    let vol_canvas = Canvas::default()
        .marker(symbols::Marker::Block)
        .x_bounds([x_lo, x_hi])
        .y_bounds([0.0, v_max * 1.15])
        .paint(move |ctx| {
            for candle in &vis_vol {
                let color = if candle.is_bullish() {
                    BULL_DIM
                } else {
                    BEAR_DIM
                };
                ctx.draw(&Rectangle {
                    x: candle.time - body_w / 2.0,
                    y: 0.0,
                    width: body_w,
                    height: candle.volume,
                    color,
                });
            }
        });

    // Volume header
    let vol_block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(BORDER))
        .title(Line::from(vec![
            Span::styled(" Vol ", Style::default().fg(TEXT_SEC)),
            Span::styled(
                format!("{:.0}", candles.last().map(|c| c.volume).unwrap_or(0.0)),
                Style::default().fg(TEXT_PRI),
            ),
        ]));

    let vol_inner = vol_block.inner(volume_area);
    f.render_widget(vol_block, volume_area);
    f.render_widget(vol_canvas, vol_inner);
}
