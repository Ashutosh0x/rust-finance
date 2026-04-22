use crate::app::App;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub fn handle_key(app: &mut App, key: KeyEvent) {
    // Only handle key PRESS events — ignore Release/Repeat (critical on Windows)
    if key.kind != KeyEventKind::Press {
        return;
    }

    // ── Kill switch overlay active — only allow K (resume) or Q (quit) ────
    if app.kill_switch_active {
        match key.code {
            KeyCode::Char('k') | KeyCode::Char('K') => {
                app.kill_switch_active = false;
                app.push_alert("Kill switch DISENGAGED — trading resumed.");
            }
            KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
            KeyCode::Esc => {
                app.kill_switch_active = false;
                app.push_alert("Kill switch DISENGAGED — trading resumed.");
            }
            _ => {}
        }
        return;
    }

    // ── Help overlay ─────────────────────────────────────────────────────
    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => app.show_help = false,
            _ => {}
        }
        return;
    }

    // ── Dialog hijack ────────────────────────────────────────────────────
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
            KeyCode::Tab => {
                // Cycle order type in dialog
                app.cycle_order_type();
            }
            _ => {}
        }
        return;
    }

    match (key.modifiers, key.code) {
        // ── Help ──────────────────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Char('?')) | (KeyModifiers::NONE, KeyCode::Char('h')) => {
            app.show_help = true
        }

        // ── Kill Switch (K) ───────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Char('k')) => {
            app.activate_kill_switch();
        }

        // ── Paper Mode (M) ───────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Char('m')) => {
            app.paper_mode = !app.paper_mode;
            if app.paper_mode {
                app.push_alert("Mode: PAPER — orders will be simulated.");
            } else {
                app.push_alert("Mode: LIVE — orders will route to broker.");
            }
        }

        // ── Quit ──────────────────────────────────────────────────────────
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => app.should_quit = true,
        (KeyModifiers::NONE, KeyCode::Char('q')) => app.should_quit = true,

        // ── Panel focus 1-6 ───────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Char('1')) => app.active_panel = 0,
        (KeyModifiers::NONE, KeyCode::Char('2')) => app.active_panel = 1,
        (KeyModifiers::NONE, KeyCode::Char('3')) => app.active_panel = 2,
        (KeyModifiers::NONE, KeyCode::Char('4')) => app.active_panel = 3,
        (KeyModifiers::NONE, KeyCode::Char('5')) => app.active_panel = 4,
        (KeyModifiers::NONE, KeyCode::Char('6')) => app.active_panel = 5,

        // ── Scroll ────────────────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Up) => app.scroll_up(),
        (KeyModifiers::NONE, KeyCode::Down) => app.scroll_down(),
        (KeyModifiers::NONE, KeyCode::Tab) => app.next_panel(),
        (_, KeyCode::BackTab) => app.prev_panel(),

        // ── Chart Controls ────────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Char('+')) | (KeyModifiers::NONE, KeyCode::Char('=')) => {
            app.chart_zoom_in();
        }
        (KeyModifiers::NONE, KeyCode::Char('-')) => {
            app.chart_zoom_out();
        }
        (KeyModifiers::SHIFT, KeyCode::Left) => {
            app.chart_scroll_left();
        }
        (KeyModifiers::SHIFT, KeyCode::Right) => {
            app.chart_scroll_right();
        }
        (KeyModifiers::NONE, KeyCode::Char('t')) => app.cycle_time_range(),

        // ── Trading ───────────────────────────────────────────────────────
        (KeyModifiers::CONTROL, KeyCode::Char('x')) => app.cancel_all(),
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => app.close_full_position(),
        (KeyModifiers::NONE, KeyCode::Char('b')) => app.open_buy_dialog(),
        (KeyModifiers::NONE, KeyCode::Char('s')) => app.open_sell_dialog(),
        (KeyModifiers::NONE, KeyCode::Char('x')) => app.cancel_selected(),
        (KeyModifiers::NONE, KeyCode::Enter) => {
            if app.show_buy_dialog || app.show_sell_dialog {
                app.confirm_order();
            }
        }
        (KeyModifiers::NONE, KeyCode::Esc) => {
            if app.show_buy_dialog || app.show_sell_dialog {
                app.dismiss_dialog();
            }
        }

        // ── AI Engine ─────────────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Char('d')) => app.trigger_dexter(), // D = Dexter
        (KeyModifiers::NONE, KeyCode::Char('f')) => app.trigger_mirofish(), // F = Mirofish
        (KeyModifiers::NONE, KeyCode::Char('c')) => app.cycle_confidence(),
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => app.toggle_auto_trade(),

        // ── Data ──────────────────────────────────────────────────────────
        (KeyModifiers::NONE, KeyCode::Char('e')) => app.export_csv(),
        (KeyModifiers::NONE, KeyCode::F(5)) => app.refresh_portfolio(),
        (KeyModifiers::CONTROL, KeyCode::Char('b')) => app.run_backtest(),

        // ── Vim-style scroll (j/k only when not triggering kill switch) ───
        (KeyModifiers::NONE, KeyCode::Char('j')) => app.scroll_down(),

        _ => {}
    }
}

pub fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent) {
    use crossterm::event::{MouseButton, MouseEventKind};

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            app.chart_zoom_in();
        }
        MouseEventKind::ScrollDown => {
            app.chart_zoom_out();
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            // Drag-to-scroll would need state tracking — deferred
        }
        _ => {}
    }
}
