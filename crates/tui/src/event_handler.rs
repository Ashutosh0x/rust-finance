use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::app::App;

pub fn handle_key(app: &mut App, key: KeyEvent) {
    // If help is open, ONLY Esc or ? closes it - nothing else fires
    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => app.show_help = false,
            _ => {}  // swallow all other keys while help is open
        }
        return;
    }

    // Normal key handling when help is closed
    match (key.modifiers, key.code) {
        // Open help
        (KeyModifiers::NONE, KeyCode::Char('?')) => app.show_help = true,

        // Kill switch - Ctrl+K
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
            app.show_help = false;
            // TODO: broadcast KillSwitchCmd to daemon
        }

        // Paper mode toggle - Ctrl+P
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            app.paper_mode = !app.paper_mode;
        }

        // Quit - q or Ctrl+C
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => app.should_quit = true,
        (KeyModifiers::NONE, KeyCode::Char('q'))    => app.should_quit = true,

        // Panel focus - 1 through 6
        (KeyModifiers::NONE, KeyCode::Char('1')) => app.active_panel = 0,
        (KeyModifiers::NONE, KeyCode::Char('2')) => app.active_panel = 1,
        (KeyModifiers::NONE, KeyCode::Char('3')) => app.active_panel = 2,
        (KeyModifiers::NONE, KeyCode::Char('4')) => app.active_panel = 3,
        (KeyModifiers::NONE, KeyCode::Char('5')) => app.active_panel = 4,
        (KeyModifiers::NONE, KeyCode::Char('6')) => app.active_panel = 5,

        // Scroll
        (KeyModifiers::NONE, KeyCode::Up)   | (KeyModifiers::NONE, KeyCode::Char('k')) => app.scroll_up(),
        (KeyModifiers::NONE, KeyCode::Down) | (KeyModifiers::NONE, KeyCode::Char('j')) => app.scroll_down(),
        (KeyModifiers::NONE, KeyCode::Tab)  => app.next_panel(),
        (_, KeyCode::BackTab)               => app.prev_panel(),

        // Trading
        (KeyModifiers::CONTROL, KeyCode::Char('x')) => app.cancel_all(),
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => app.close_full_position(),
        (KeyModifiers::NONE, KeyCode::Char('b')) => app.open_buy_dialog(),
        (KeyModifiers::NONE, KeyCode::Char('s')) => app.open_sell_dialog(),
        (KeyModifiers::NONE, KeyCode::Char('x')) => app.cancel_selected(),
        (KeyModifiers::NONE, KeyCode::Char('h')) => app.halve_position(),
        (KeyModifiers::NONE, KeyCode::Enter) => app.confirm_order(),
        (KeyModifiers::NONE, KeyCode::Esc)   => app.dismiss_dialog(),

        // AI
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => app.toggle_auto_trade(),
        (KeyModifiers::SHIFT, KeyCode::Char('A'))   => app.trigger_mirofish(),
        (KeyModifiers::NONE, KeyCode::Char('a'))    => app.trigger_dexter(),
        (KeyModifiers::NONE, KeyCode::Char('c'))    => app.cycle_confidence(),

        // Data
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => app.export_csv(),
        (KeyModifiers::CONTROL, KeyCode::Char('b')) => app.run_backtest(),
        (KeyModifiers::CONTROL, KeyCode::Char('m')) => app.toggle_data_source(),
        (KeyModifiers::NONE, KeyCode::F(5))         => app.refresh_portfolio(),

        _ => {}
    }
}
