use common::Action;

pub struct MarketEnvironment {
    pub current_step: usize,
    pub max_steps: usize,
    pub inventory: f64,
    pub balance: f64,
}

impl MarketEnvironment {
    pub fn new(max_steps: usize, initial_balance: f64) -> Self {
        Self {
            current_step: 0,
            max_steps,
            inventory: 0.0,
            balance: initial_balance,
        }
    }

    pub fn reset(&mut self) -> Vec<f64> {
        self.current_step = 0;
        self.inventory = 0.0;
        self.get_state()
    }

    pub fn step(&mut self, action: Action, current_price: f64) -> (Vec<f64>, f64, bool) {
        let reward = match action {
            Action::Buy { size, .. } => {
                self.inventory += size;
                self.balance -= size * current_price;
                -0.01 // Trading cost penalty
            }
            Action::Sell { size, .. } => {
                self.inventory -= size;
                self.balance += size * current_price;
                -0.01
            }
            Action::Hold => {
                0.0 // No action, no immediate reward/penalty except time decay
            }
        };

        self.current_step += 1;
        let done = self.current_step >= self.max_steps;

        // Final reward based on PNL mark-to-market if done
        let final_reward = if done {
            (self.balance + (self.inventory * current_price)) - 10000.0 // Assuming 10k start
        } else {
            reward
        };

        (self.get_state(), final_reward, done)
    }

    fn get_state(&self) -> Vec<f64> {
        // [Price (mocked), Spread (mocked), Inventory, Step Ratio]
        vec![
            100.0,
            0.1,
            self.inventory,
            self.current_step as f64 / self.max_steps as f64,
        ]
    }
}
