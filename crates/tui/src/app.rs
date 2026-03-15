pub struct App {
    pub should_quit: bool,
    pub chart_data: Vec<(f64, f64)>,
    pub connection_status: String,
    pub show_help: bool,
    pub paper_mode: bool,
    pub active_panel: u8,
}

impl App {
    pub fn new() -> Self {
        let mut chart_data = Vec::new();
        let mut price = 1430.0;
        for i in 0..100 {
            let change = (f64::sin(i as f64 * 0.2) * 5.0) + (f64::cos(i as f64 * 0.1) * 3.0) + (i as f64 * 0.2);
            chart_data.push((i as f64, price + change));
        }
        
        Self {
            should_quit: false,
            chart_data,
            connection_status: "Connecting...".to_string(),
            show_help: false,
            paper_mode: false,
            active_panel: 0,
        }
    }

    pub fn scroll_up(&mut self) {}
    pub fn scroll_down(&mut self) {}
    pub fn next_panel(&mut self) {}
    pub fn prev_panel(&mut self) {}
    pub fn open_buy_dialog(&mut self) {}
    pub fn open_sell_dialog(&mut self) {}
    pub fn cancel_selected(&mut self) {}
    pub fn cancel_all(&mut self) {}
    pub fn halve_position(&mut self) {}
    pub fn close_full_position(&mut self) {}
    pub fn confirm_order(&mut self) {}
    pub fn dismiss_dialog(&mut self) {}
    pub fn trigger_dexter(&mut self) {}
    pub fn trigger_mirofish(&mut self) {}
    pub fn cycle_confidence(&mut self) {}
    pub fn toggle_auto_trade(&mut self) {}
    pub fn export_csv(&mut self) {}
    pub fn run_backtest(&mut self) {}
    pub fn toggle_data_source(&mut self) {}
    pub fn refresh_portfolio(&mut self) {}
}
