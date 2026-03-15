/// Scaffolding for ETF NAV Arbitrage framework.
/// Identifies premiums/discounts between an ETF's traded price and its underlying basket NAV.

pub struct EtfNavArbitrage {
    pub min_discount_threshold: f64,
}

impl EtfNavArbitrage {
    pub fn new(min_discount_threshold: f64) -> Self {
        Self {
            min_discount_threshold,
        }
    }

    /// Calculates if the ETF is trading at a significant discount/premium to its basket
    pub fn analyze_arbitrage_opportunity(
        &self,
        etf_price: f64,
        computed_nav: f64,
    ) -> Option<String> {
        if computed_nav == 0.0 {
            return None;
        }

        let diff = etf_price - computed_nav;
        let diff_pct = diff / computed_nav;

        if diff_pct < -self.min_discount_threshold {
            Some("BUY_ETF_SELL_BASKET".to_string())
        } else if diff_pct > self.min_discount_threshold {
            Some("SELL_ETF_BUY_BASKET".to_string())
        } else {
            None
        }
    }
}
