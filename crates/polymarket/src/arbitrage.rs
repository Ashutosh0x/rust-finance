// crates/polymarket/src/arbitrage.rs
//
// Polymarket Arbitrage Engine — Sum-to-One & Cross-Market
//
// Detects and exploits:
//   1. Sum-to-one: YES + NO < $1.00 on the same market
//   2. Cross-market: Same event priced differently across Polymarket markets
//   3. Cross-platform: Polymarket vs. Kalshi spread detection
//
// Reference: QuantVPS (2025) "$40M in arb profits April 2024–2025"

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Data Types ──────────────────────────────────────────────────

/// Normalized prediction market snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSnapshot {
    /// Unique market identifier (slug or condition_id)
    pub market_id: String,
    /// Human-readable question
    pub question: String,
    /// Platform source
    pub platform: Platform,
    /// YES token price [0, 1]
    pub yes_price: f64,
    /// NO token price [0, 1]
    pub no_price: f64,
    /// YES token best ask (what you'd actually pay to buy YES)
    pub yes_ask: f64,
    /// NO token best ask
    pub no_ask: f64,
    /// YES token best bid (what you'd get selling YES)
    pub yes_bid: f64,
    /// NO token best bid
    pub no_bid: f64,
    /// 24h volume in USD
    pub volume_24h: f64,
    /// Available liquidity (depth at best price)
    pub liquidity_usd: f64,
    /// Resolution deadline (Unix timestamp)
    pub resolution_ts: i64,
    /// Category tags for semantic matching
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Platform {
    Polymarket,
    Kalshi,
    Metaculus,
    Other(String),
}

/// Detected arbitrage opportunity.
#[derive(Debug, Clone)]
pub struct ArbOpportunity {
    pub arb_type: ArbType,
    /// Gross profit before fees (per $1 risked)
    pub gross_spread: f64,
    /// Net profit after fees
    pub net_spread: f64,
    /// Maximum executable size in USD
    pub max_size_usd: f64,
    /// Expected P&L at max size
    pub expected_pnl: f64,
    /// Markets involved
    pub legs: Vec<ArbLeg>,
    /// Confidence score [0, 1]
    pub confidence: f64,
    /// Urgency: seconds before opportunity likely closes
    pub urgency_secs: f64,
}

#[derive(Debug, Clone)]
pub enum ArbType {
    /// YES + NO < 1.00 on same market
    SumToOne,
    /// Same event, different prices on same platform
    CrossMarket,
    /// Same event, different platforms (e.g., Polymarket vs Kalshi)
    CrossPlatform,
}

#[derive(Debug, Clone)]
pub struct ArbLeg {
    pub market_id: String,
    pub platform: Platform,
    pub side: ArbSide,
    pub token: ArbToken,
    pub price: f64,
    pub quantity_usd: f64,
}

#[derive(Debug, Clone)]
pub enum ArbSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub enum ArbToken {
    Yes,
    No,
}

// ─── Fee Structure ───────────────────────────────────────────────

/// Platform fee schedule.
#[derive(Debug, Clone)]
pub struct FeeSchedule {
    /// Taker fee as fraction (e.g., 0.02 = 2%)
    pub taker_fee: f64,
    /// Maker rebate as fraction (negative = rebate)
    pub maker_rebate: f64,
    /// Settlement/withdrawal fee
    pub settlement_fee: f64,
    /// Gas cost per transaction (for on-chain platforms)
    pub gas_cost_usd: f64,
}

impl FeeSchedule {
    pub fn polymarket() -> Self {
        Self {
            taker_fee: 0.02,   // 2% taker
            maker_rebate: 0.0, // no maker rebate currently
            settlement_fee: 0.0,
            gas_cost_usd: 0.05, // Polygon gas is cheap
        }
    }

    pub fn kalshi() -> Self {
        Self {
            taker_fee: 0.02,
            maker_rebate: 0.0,
            settlement_fee: 0.0,
            gas_cost_usd: 0.0, // No gas (centralized)
        }
    }
}

// ─── Arbitrage Scanner ───────────────────────────────────────────

/// Scans for arbitrage opportunities across prediction markets.
pub struct ArbScanner {
    /// Known markets indexed by market_id
    markets: HashMap<String, MarketSnapshot>,
    /// Fee schedules per platform
    fees: HashMap<String, FeeSchedule>,
    /// Minimum net spread to trigger (after all fees)
    min_net_spread: f64,
    /// Minimum liquidity to consider
    min_liquidity_usd: f64,
    /// Semantic matching pairs: (market_id_1, market_id_2) that represent the same event
    matched_pairs: Vec<(String, String)>,
}

impl ArbScanner {
    pub fn new(min_net_spread: f64, min_liquidity_usd: f64) -> Self {
        let mut fees = HashMap::new();
        fees.insert("polymarket".into(), FeeSchedule::polymarket());
        fees.insert("kalshi".into(), FeeSchedule::kalshi());

        Self {
            markets: HashMap::new(),
            fees,
            min_net_spread,
            min_liquidity_usd,
            matched_pairs: Vec::new(),
        }
    }

    /// Update or insert a market snapshot.
    pub fn update_market(&mut self, snapshot: MarketSnapshot) {
        self.markets.insert(snapshot.market_id.clone(), snapshot);
    }

    /// Register two markets as semantically equivalent (same event).
    pub fn register_pair(&mut self, market_a: &str, market_b: &str) {
        self.matched_pairs
            .push((market_a.to_string(), market_b.to_string()));
    }

    /// Scan all markets for arbitrage opportunities.
    pub fn scan(&self) -> Vec<ArbOpportunity> {
        let mut opportunities = Vec::new();

        // 1. Sum-to-one arbitrage (single market)
        for market in self.markets.values() {
            if let Some(opp) = self.check_sum_to_one(market) {
                opportunities.push(opp);
            }
        }

        // 2. Cross-market / cross-platform arbitrage (matched pairs)
        for (id_a, id_b) in &self.matched_pairs {
            if let (Some(a), Some(b)) = (self.markets.get(id_a), self.markets.get(id_b)) {
                if let Some(opp) = self.check_cross_market(a, b) {
                    opportunities.push(opp);
                }
            }
        }

        // Sort by net spread descending
        opportunities.sort_by(|a, b| {
            b.net_spread
                .partial_cmp(&a.net_spread)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        opportunities
    }

    /// Check for sum-to-one arbitrage: buy YES + buy NO < $1.00
    fn check_sum_to_one(&self, market: &MarketSnapshot) -> Option<ArbOpportunity> {
        // Use ask prices (what we'd actually pay)
        let total_cost = market.yes_ask + market.no_ask;

        if total_cost >= 1.0 {
            return None; // No arb
        }

        let gross_spread = 1.0 - total_cost;

        // Calculate fees
        let platform_key = match &market.platform {
            Platform::Polymarket => "polymarket",
            Platform::Kalshi => "kalshi",
            _ => return None,
        };
        let fees = self.fees.get(platform_key)?;

        // Fee on both legs
        let total_fee = fees.taker_fee * 2.0 + fees.gas_cost_usd * 2.0 / market.liquidity_usd.max(1.0);
        let net_spread = gross_spread - total_fee;

        if net_spread < self.min_net_spread {
            return None;
        }

        if market.liquidity_usd < self.min_liquidity_usd {
            return None;
        }

        let max_size = market.liquidity_usd * 0.5; // Don't take more than 50% of liquidity
        let expected_pnl = max_size * net_spread;

        Some(ArbOpportunity {
            arb_type: ArbType::SumToOne,
            gross_spread,
            net_spread,
            max_size_usd: max_size,
            expected_pnl,
            legs: vec![
                ArbLeg {
                    market_id: market.market_id.clone(),
                    platform: market.platform.clone(),
                    side: ArbSide::Buy,
                    token: ArbToken::Yes,
                    price: market.yes_ask,
                    quantity_usd: max_size * market.yes_ask,
                },
                ArbLeg {
                    market_id: market.market_id.clone(),
                    platform: market.platform.clone(),
                    side: ArbSide::Buy,
                    token: ArbToken::No,
                    price: market.no_ask,
                    quantity_usd: max_size * market.no_ask,
                },
            ],
            confidence: (net_spread / gross_spread).clamp(0.0, 1.0),
            urgency_secs: 30.0, // Arbs close fast
        })
    }

    /// Check for cross-market/cross-platform arbitrage.
    fn check_cross_market(
        &self,
        market_a: &MarketSnapshot,
        market_b: &MarketSnapshot,
    ) -> Option<ArbOpportunity> {
        // Strategy: Buy YES on cheaper platform, buy NO on more expensive platform
        // If YES_a + NO_b < 1.00, there's an arb
        let combo_1 = market_a.yes_ask + market_b.no_ask; // Buy YES@A + NO@B
        let combo_2 = market_b.yes_ask + market_a.no_ask; // Buy YES@B + NO@A

        let (best_cost, buy_yes_on, buy_no_on) = if combo_1 < combo_2 {
            (combo_1, market_a, market_b)
        } else {
            (combo_2, market_b, market_a)
        };

        if best_cost >= 1.0 {
            return None;
        }

        let gross_spread = 1.0 - best_cost;

        // Fees on both platforms
        let fee_a = self.platform_fee(&buy_yes_on.platform);
        let fee_b = self.platform_fee(&buy_no_on.platform);
        let total_fee = fee_a + fee_b;
        let net_spread = gross_spread - total_fee;

        if net_spread < self.min_net_spread {
            return None;
        }

        let min_liquidity = buy_yes_on.liquidity_usd.min(buy_no_on.liquidity_usd);
        if min_liquidity < self.min_liquidity_usd {
            return None;
        }

        let max_size = min_liquidity * 0.3; // Conservative: 30% of thinner side
        let arb_type = if buy_yes_on.platform == buy_no_on.platform {
            ArbType::CrossMarket
        } else {
            ArbType::CrossPlatform
        };

        Some(ArbOpportunity {
            arb_type,
            gross_spread,
            net_spread,
            max_size_usd: max_size,
            expected_pnl: max_size * net_spread,
            legs: vec![
                ArbLeg {
                    market_id: buy_yes_on.market_id.clone(),
                    platform: buy_yes_on.platform.clone(),
                    side: ArbSide::Buy,
                    token: ArbToken::Yes,
                    price: buy_yes_on.yes_ask,
                    quantity_usd: max_size * buy_yes_on.yes_ask,
                },
                ArbLeg {
                    market_id: buy_no_on.market_id.clone(),
                    platform: buy_no_on.platform.clone(),
                    side: ArbSide::Buy,
                    token: ArbToken::No,
                    price: buy_no_on.no_ask,
                    quantity_usd: max_size * buy_no_on.no_ask,
                },
            ],
            confidence: (net_spread / gross_spread).clamp(0.0, 1.0),
            urgency_secs: 15.0,
        })
    }

    fn platform_fee(&self, platform: &Platform) -> f64 {
        let key = match platform {
            Platform::Polymarket => "polymarket",
            Platform::Kalshi => "kalshi",
            _ => return 0.05, // Default 5% for unknown
        };
        self.fees
            .get(key)
            .map(|f| f.taker_fee + f.gas_cost_usd / 100.0)
            .unwrap_or(0.05)
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_market(id: &str, yes_ask: f64, no_ask: f64, platform: Platform) -> MarketSnapshot {
        MarketSnapshot {
            market_id: id.into(),
            question: format!("Will {} happen?", id),
            platform,
            yes_price: yes_ask - 0.01,
            no_price: no_ask - 0.01,
            yes_ask,
            no_ask,
            yes_bid: yes_ask - 0.02,
            no_bid: no_ask - 0.02,
            volume_24h: 50_000.0,
            liquidity_usd: 10_000.0,
            resolution_ts: 1735689600,
            tags: vec!["politics".into()],
        }
    }

    #[test]
    fn test_sum_to_one_arb_detected() {
        let mut scanner = ArbScanner::new(0.001, 100.0);

        // YES=0.48, NO=0.47 → total=0.95 → 5% gross spread
        let market = make_market("btc-100k", 0.48, 0.47, Platform::Polymarket);
        scanner.update_market(market);

        let opps = scanner.scan();
        assert!(!opps.is_empty(), "Should detect sum-to-one arb");

        let opp = &opps[0];
        assert!(matches!(opp.arb_type, ArbType::SumToOne));
        assert!(opp.gross_spread > 0.04, "Gross spread should be ~5%");
        assert!(opp.net_spread > 0.0, "Net spread should be positive after fees");
    }

    #[test]
    fn test_no_arb_when_sum_equals_one() {
        let mut scanner = ArbScanner::new(0.001, 100.0);

        // YES=0.55, NO=0.47 → total=1.02 → no arb
        let market = make_market("btc-100k", 0.55, 0.47, Platform::Polymarket);
        scanner.update_market(market);

        let opps = scanner.scan();
        let sum_to_one: Vec<_> = opps.iter().filter(|o| matches!(o.arb_type, ArbType::SumToOne)).collect();
        assert!(sum_to_one.is_empty(), "No arb when YES+NO >= 1.0");
    }

    #[test]
    fn test_cross_platform_arb() {
        let mut scanner = ArbScanner::new(0.001, 100.0);

        // Polymarket: YES=0.60
        let poly = make_market("poly-election", 0.60, 0.45, Platform::Polymarket);
        // Kalshi: NO=0.35 (implying YES=0.65 — spread exists)
        let kalshi = make_market("kalshi-election", 0.65, 0.35, Platform::Kalshi);

        scanner.update_market(poly);
        scanner.update_market(kalshi);
        scanner.register_pair("poly-election", "kalshi-election");

        let opps = scanner.scan();
        let cross: Vec<_> = opps
            .iter()
            .filter(|o| matches!(o.arb_type, ArbType::CrossPlatform))
            .collect();

        // YES@Poly(0.60) + NO@Kalshi(0.35) = 0.95 → 5% gross arb
        assert!(!cross.is_empty(), "Should detect cross-platform arb");
        assert!(cross[0].gross_spread > 0.04);
    }

    #[test]
    fn test_low_liquidity_filtered() {
        let mut scanner = ArbScanner::new(0.001, 5000.0); // Min $5000 liquidity

        let mut market = make_market("low-liq", 0.40, 0.40, Platform::Polymarket);
        market.liquidity_usd = 100.0; // Too low
        scanner.update_market(market);

        let opps = scanner.scan();
        assert!(opps.is_empty(), "Low liquidity should be filtered");
    }

    #[test]
    fn test_opportunities_sorted_by_spread() {
        let mut scanner = ArbScanner::new(0.001, 100.0);

        scanner.update_market(make_market("small-arb", 0.48, 0.49, Platform::Polymarket));
        scanner.update_market(make_market("big-arb", 0.40, 0.40, Platform::Polymarket));

        let opps = scanner.scan();
        if opps.len() >= 2 {
            assert!(opps[0].net_spread >= opps[1].net_spread);
        }
    }
}
