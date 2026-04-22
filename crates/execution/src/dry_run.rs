use chrono::Utc;
use rand::Rng;

pub struct Order {
    pub symbol: String,
    pub qty: f64,
    pub price: f64,
}

pub struct Fill {
    pub fill_price: f64,
    pub timestamp: chrono::DateTime<Utc>,
}

pub fn simulate_fill(order: &Order) -> Fill {
    let mut rng = rand::thread_rng();
    let slippage: f64 = rng.gen_range(-0.01..0.01);

    Fill {
        fill_price: order.price + slippage,
        timestamp: Utc::now(),
    }
}
