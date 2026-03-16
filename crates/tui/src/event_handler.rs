use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::app::App;

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('?') => app.show_help = false,
            _ => {}
        }
        return;
    }
    
    // If a dialog is open, hijack input
    if app.show_buy_dialog || app.show_sell_dialog {
        match key.code {
            KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                app.order_qty_input.push(c);
            }
            KeyCode::Backspace => {
                app.order_qty_input.pop();
            }
            KeyCode::Enter => {
                app.confirm_order();
            }
            KeyCode::Esc => {
                app.dismiss_dialog();
            }
            _ => {}
        }
        return; // Don't process other hotkeys
    }

    match (key.modifiers, key.code) {
        // ── Help ──────────────────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Char('?')) | (KeyModifiers::NONE, KeyCode::Char('h')) => app.show_help = true,

        // ── System ────────────────────────────────────────────────────────
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
            // Kill switch - halt all strategies
            app.show_help = false;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            app.paper_mode = !app.paper_mode;
        }

        // ── Quit ──────────────────────────────────────────────────────────
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => app.should_quit = true,
        (KeyModifiers::NONE, KeyCode::Char('q'))    => app.should_quit = true,

        // ── Panel focus 1-6 ───────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Char('1')) => app.active_panel = 0,
        (KeyModifiers::NONE, KeyCode::Char('2')) => app.active_panel = 1,
        (KeyModifiers::NONE, KeyCode::Char('3')) => app.active_panel = 2,
        (KeyModifiers::NONE, KeyCode::Char('4')) => app.active_panel = 3,
        (KeyModifiers::NONE, KeyCode::Char('5')) => app.active_panel = 4,
        (KeyModifiers::NONE, KeyCode::Char('6')) => app.active_panel = 5,

        // ── Scroll ────────────────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Up)   | (KeyModifiers::NONE, KeyCode::Char('k')) => app.scroll_up(),
        (KeyModifiers::NONE, KeyCode::Down) | (KeyModifiers::NONE, KeyCode::Char('j')) => app.scroll_down(),
        (KeyModifiers::NONE, KeyCode::Tab)  => app.next_panel(),
        (_, KeyCode::BackTab)               => app.prev_panel(),

        // ── Chart Controls ────────────────────────────────────────────────
        // Zoom: +/- or Ctrl+Up/Down
        (KeyModifiers::NONE, KeyCode::Char('+')) | (KeyModifiers::NONE, KeyCode::Char('=')) => {
            app.chart_zoom_in();
        }
        (KeyModifiers::NONE, KeyCode::Char('-')) => {
            app.chart_zoom_out();
        }
        // Scroll chart: Shift+Left/Right or H/L
        (KeyModifiers::SHIFT, KeyCode::Left) | (KeyModifiers::SHIFT, KeyCode::Char('H')) => {
            app.chart_scroll_left();
        }
        (KeyModifiers::SHIFT, KeyCode::Right) | (KeyModifiers::SHIFT, KeyCode::Char('L')) => {
            app.chart_scroll_right();
        }
        // Time range: t
        (KeyModifiers::NONE, KeyCode::Char('t')) => app.cycle_time_range(),

        // ── Trading ───────────────────────────────────────────────────────
        (KeyModifiers::CONTROL, KeyCode::Char('x')) => app.cancel_all(),
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => app.close_full_position(),
        (KeyModifiers::NONE, KeyCode::Char('b')) => app.open_buy_dialog(),
        (KeyModifiers::NONE, KeyCode::Char('s')) => app.open_sell_dialog(),
        (KeyModifiers::NONE, KeyCode::Char('x')) => app.cancel_selected(),
        (KeyModifiers::SHIFT, KeyCode::Char('H')) => app.halve_position(),
        (KeyModifiers::NONE, KeyCode::Enter) => app.confirm_order(),
        (KeyModifiers::NONE, KeyCode::Esc)   => app.dismiss_dialog(),

        // ── AI ────────────────────────────────────────────────────────────
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => app.toggle_auto_trade(),
        (KeyModifiers::SHIFT, KeyCode::Char('A'))   => app.trigger_mirofish(),
        (KeyModifiers::NONE, KeyCode::Char('a'))    => app.trigger_dexter(),
        (KeyModifiers::NONE, KeyCode::Char('c'))    => app.cycle_confidence(),

        // ── Data ──────────────────────────────────────────────────────────
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => app.export_csv(),
        (KeyModifiers::CONTROL, KeyCode::Char('b')) => app.run_backtest(),
        (KeyModifiers::CONTROL, KeyCode::Char('m')) => app.toggle_data_source(),
        (KeyModifiers::NONE, KeyCode::F(5))         => app.refresh_portfolio(),

        _ => {}
    }
}

pub fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent) {
    use crossterm::event::{MouseEventKind, MouseButton};

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            // Zoom In
            app.chart_zoom_in();
        }
        MouseEventKind::ScrollDown => {
            // Zoom Out
            app.chart_zoom_out();
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            // Very simplistic drag tracking: 
            // In a real app we'd track the previous X/Y coords to delta scroll.
            // For now, if moving mouse, let's assume we want to trigger a scroll action.
            // We can check if column is moving left/right if we kept state,
            // but for a stateless approach we can't easily tell direction.
            // As a basic implementation, we just use hotkeys or precise logic.
            // Let's implement drag based on a cached last_mouse_x later if needed.
        }
        _ => {}
    }
}
