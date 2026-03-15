use anyhow::Result;
use common::events::BotEvent;
use common::Action;
use tracing::info;
use execution::tca::{TcaEngine, FillRecord, TcaReport};

pub struct BacktestEngine {
    initial_capital: f64,
    current_capital: f64,
    position_size: f64,
    trade_history: Vec<f64>, // Stores PnLs
    tca: TcaEngine,
}

impl BacktestEngine {
    pub fn new(initial_capital: f64) -> Self {
        Self {
            initial_capital,
            current_capital: initial_capital,
            position_size: 0.0,
            trade_history: Vec::new(),
            tca: TcaEngine::new(),
        }
    }

    /// Simulate an execution
    pub fn execute(&mut self, action: Action, price: f64, ts: u64) {
        match action {
            Action::Buy { token, size, .. } => {
                let cost = size * price;
                if self.current_capital >= cost {
                    self.current_capital -= cost;
                    self.position_size += size;
                    
                    self.tca.record_fill(FillRecord {
                        order_id: "idx-bt-buy".into(),
                        symbol: token,
                        strategy: "PPO_RL".into(),
                        decision_price: price * 0.999,
                        arrival_vwap: price,
                        fill_price: price,
                        side_sign: 1.0,
                        quantity: size as u64,
                        commission_usd: 1.50,
                        decision_ts: ts.saturating_sub(10), // mock 10ms latency
                        fill_ts: ts,
                        day_vwap: price * 0.99,
                        interval_twap: price,
                    });
                }
            }
            Action::Sell { token, size, .. } => {
                if self.position_size >= size {
                    let revenue = size * price;
                    self.current_capital += revenue;
                    self.position_size -= size;
                    // Mock PnL
                    self.trade_history.push(revenue * 0.01);
                    
                    self.tca.record_fill(FillRecord {
                        order_id: "idx-bt-sell".into(),
                        symbol: token,
                        strategy: "PPO_RL".into(),
                        decision_price: price * 1.001,
                        arrival_vwap: price,
                        fill_price: price,
                        side_sign: -1.0,
                        quantity: size as u64,
                        commission_usd: 1.50,
                        decision_ts: ts.saturating_sub(10),
                        fill_ts: ts,
                        day_vwap: price * 1.01,
                        interval_twap: price,
                    });
                }
            }
            Action::Hold => {}
        }
    }

    pub fn process_events(&mut self, events: Vec<BotEvent>) -> Result<()> {
        info!("Processing {} historical events...", events.len());
        // Simple mock loop for structure. In reality, we'd feed this into a strategy.
        let mut sim_ts = 1700000000000;
        for event in events {
            if let BotEvent::MarketEvent { price, .. } = event {
                sim_ts += 1000; // 1 second intervals mock
                // Mock strategy trigger
                if price < 50000.0 {
                    self.execute(Action::Buy { token: "SOL".into(), size: 1.0, confidence: 0.9 }, price, sim_ts);
                } else if price > 60000.0 {
                    self.execute(Action::Sell { token: "SOL".into(), size: 1.0, confidence: 0.9 }, price, sim_ts);
                }
            }
        }
        Ok(())
    }

    pub fn get_equity(&self, current_price: f64) -> f64 {
        self.current_capital + (self.position_size * current_price)
    }
    
    pub fn get_trade_history(&self) -> &Vec<f64> {
        &self.trade_history
    }
    
    pub fn get_tca_report(&self) -> TcaReport {
        self.tca.generate_report()
    }
}
