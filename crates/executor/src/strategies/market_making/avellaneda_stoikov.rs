pub struct AsModel {
    pub risk_aversion: f64,
    pub inventory_penalty: f64,
    pub target_inventory: f64,
    pub current_inventory: f64,
    pub volatility: f64,
    pub time_to_close: f64,
}

impl AsModel {
    pub fn calculate_reservation_price(&self, mid_price: f64) -> f64 {
        mid_price
            - (self.current_inventory - self.target_inventory)
                * self.risk_aversion
                * self.volatility
                * self.time_to_close
    }

    pub fn optimal_spread(&self) -> f64 {
        self.risk_aversion * self.volatility * self.time_to_close
            + (2.0 / self.risk_aversion)
                * (1.0 + (self.risk_aversion / self.inventory_penalty)).ln()
    }

    pub fn get_quotes(&self, mid_price: f64) -> (f64, f64) {
        let res_price = self.calculate_reservation_price(mid_price);
        let spread = self.optimal_spread();

        // Return Bid, Ask
        (res_price - spread / 2.0, res_price + spread / 2.0)
    }
}
