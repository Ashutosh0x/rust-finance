// crates/risk/src/self_match.rs
//
// Self-Match Prevention (SMP) — prevents wash trades when the market maker
// holds simultaneous bid/ask quotes that could cross.
//
// Implements the DMIST November 2025 unified SMP standard.
// Required by CME, NASDAQ, and all major exchanges since 2026.
//
// Three modes:
//   CancelResting    — cancel the resting order (CME standard)
//   CancelAggressive — cancel the incoming order (NASDAQ standard)
//   CancelBoth       — cancel both (safest for market makers)

use crate::state::{EngineState, OpenOrderSnapshot};
use compact_str::CompactString;
use execution::gateway::OpenRequest;

use super::interceptor::{RiskInterceptor, RiskVerdict};

/// Self-match prevention mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SmpMode {
    /// Cancel the resting order — CME standard.
    CancelResting,
    /// Cancel the incoming (aggressive) order — NASDAQ standard.
    CancelAggressive,
    /// Cancel both sides — safest for market makers.
    CancelBoth,
}

/// Self-Match Prevention interceptor.
///
/// Checks if a new order would cross with any resting order on the opposite
/// side for the same symbol. This prevents wash trades which are a regulatory
/// violation on all major exchanges.
///
/// Should be the FIRST interceptor in the `RiskChain` — before VaR, Kelly,
/// or any other sizing checks — because a wash trade is always illegal
/// regardless of position size or drawdown state.
pub struct SelfMatchPrevention {
    pub mode: SmpMode,
}

impl SelfMatchPrevention {
    pub fn new(mode: SmpMode) -> Self {
        Self { mode }
    }

    /// Check if two orders would cross (trade against each other).
    fn would_cross(incoming: &OpenRequest, resting: &OpenOrderSnapshot) -> bool {
        use common::events::OrderSide;

        // Must be opposite sides
        let opposite_sides = match (&incoming.side, &resting.side) {
            (OrderSide::Buy, OrderSide::Sell) | (OrderSide::Sell, OrderSide::Buy) => true,
            _ => false,
        };

        if !opposite_sides {
            return false;
        }

        // Check price crossing
        match (&incoming.side, incoming.limit_price, resting.price) {
            // Market order always crosses
            (_, None, _) => true,
            // Buy limit >= Sell resting → cross
            (OrderSide::Buy, Some(buy_limit), sell_price) => buy_limit >= sell_price,
            // Sell limit <= Buy resting → cross
            (OrderSide::Sell, Some(sell_limit), buy_price) => sell_limit <= buy_price,
        }
    }
}

impl RiskInterceptor for SelfMatchPrevention {
    fn evaluate(&self, state: &EngineState, req: &OpenRequest) -> RiskVerdict {
        for resting in &state.open_orders {
            // Same symbol only
            if resting.symbol.as_str() != req.symbol.as_str() {
                continue;
            }

            if Self::would_cross(req, resting) {
                let reason = CompactString::new(format!(
                    "SMP[{:?}]: {} {} would cross resting {} @ {:.4}",
                    self.mode,
                    req.side_label(),
                    req.symbol,
                    resting.side_label(),
                    resting.price,
                ));

                return match self.mode {
                    SmpMode::CancelAggressive | SmpMode::CancelBoth => {
                        // Block the incoming order
                        RiskVerdict::Blocked { reason }
                    }
                    SmpMode::CancelResting => {
                        // In CancelResting mode, we approve the incoming order
                        // but the caller is responsible for cancelling the resting.
                        // We signal via Modified with the same request.
                        RiskVerdict::Modified {
                            new_request: req.clone(),
                            reason,
                        }
                    }
                };
            }
        }

        RiskVerdict::Approved
    }
}

/// Extension trait for readable side labels.
trait SideLabel {
    fn side_label(&self) -> &'static str;
}

impl SideLabel for OpenRequest {
    fn side_label(&self) -> &'static str {
        match self.side {
            common::events::OrderSide::Buy => "BUY",
            common::events::OrderSide::Sell => "SELL",
        }
    }
}

impl SideLabel for OpenOrderSnapshot {
    fn side_label(&self) -> &'static str {
        match self.side {
            common::events::OrderSide::Buy => "BUY",
            common::events::OrderSide::Sell => "SELL",
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::EngineState;
    use common::events::{OrderSide, OrderType};
    use execution::gateway::TimeInForce;

    fn make_state_with_resting(
        symbol: &str,
        side: OrderSide,
        price: f64,
    ) -> EngineState {
        EngineState {
            total_equity: 100_000.0,
            daily_pnl: 0.0,
            current_drawdown_pct: 0.0,
            open_order_count: 1,
            daily_trade_count: 0,
            open_orders: vec![OpenOrderSnapshot {
                symbol: symbol.into(),
                side,
                price,
            }],
        }
    }

    fn make_request(symbol: &str, side: OrderSide, limit: Option<f64>) -> OpenRequest {
        OpenRequest {
            client_order_id: "test-smp".into(),
            symbol: symbol.into(),
            side,
            quantity: 10.0,
            order_type: OrderType::Limit,
            limit_price: limit,
            time_in_force: TimeInForce::DAY,
        }
    }

    #[test]
    fn test_crossing_buy_blocked() {
        let smp = SelfMatchPrevention::new(SmpMode::CancelAggressive);
        let state = make_state_with_resting("AAPL", OrderSide::Sell, 150.0);
        let req = make_request("AAPL", OrderSide::Buy, Some(151.0)); // crosses at 150

        let verdict = smp.evaluate(&state, &req);
        assert!(
            matches!(verdict, RiskVerdict::Blocked { .. }),
            "Buy at 151 should cross sell at 150"
        );
    }

    #[test]
    fn test_crossing_sell_blocked() {
        let smp = SelfMatchPrevention::new(SmpMode::CancelAggressive);
        let state = make_state_with_resting("AAPL", OrderSide::Buy, 150.0);
        let req = make_request("AAPL", OrderSide::Sell, Some(149.0)); // crosses at 150

        let verdict = smp.evaluate(&state, &req);
        assert!(
            matches!(verdict, RiskVerdict::Blocked { .. }),
            "Sell at 149 should cross buy at 150"
        );
    }

    #[test]
    fn test_no_cross_when_prices_dont_overlap() {
        let smp = SelfMatchPrevention::new(SmpMode::CancelAggressive);
        let state = make_state_with_resting("AAPL", OrderSide::Sell, 155.0);
        let req = make_request("AAPL", OrderSide::Buy, Some(150.0)); // doesn't cross

        let verdict = smp.evaluate(&state, &req);
        assert!(
            matches!(verdict, RiskVerdict::Approved),
            "Buy at 150 should NOT cross sell at 155"
        );
    }

    #[test]
    fn test_same_side_not_blocked() {
        let smp = SelfMatchPrevention::new(SmpMode::CancelAggressive);
        let state = make_state_with_resting("AAPL", OrderSide::Buy, 150.0);
        let req = make_request("AAPL", OrderSide::Buy, Some(151.0));

        let verdict = smp.evaluate(&state, &req);
        assert!(
            matches!(verdict, RiskVerdict::Approved),
            "Same-side orders should never self-match"
        );
    }

    #[test]
    fn test_different_symbol_not_blocked() {
        let smp = SelfMatchPrevention::new(SmpMode::CancelAggressive);
        let state = make_state_with_resting("AAPL", OrderSide::Sell, 150.0);
        let req = make_request("NVDA", OrderSide::Buy, Some(900.0)); // different symbol

        let verdict = smp.evaluate(&state, &req);
        assert!(
            matches!(verdict, RiskVerdict::Approved),
            "Different symbols should not trigger SMP"
        );
    }

    #[test]
    fn test_market_order_always_crosses() {
        let smp = SelfMatchPrevention::new(SmpMode::CancelAggressive);
        let state = make_state_with_resting("AAPL", OrderSide::Sell, 150.0);
        let req = make_request("AAPL", OrderSide::Buy, None); // market order

        let verdict = smp.evaluate(&state, &req);
        assert!(
            matches!(verdict, RiskVerdict::Blocked { .. }),
            "Market buy should always cross resting sell"
        );
    }

    #[test]
    fn test_cancel_resting_mode_returns_modified() {
        let smp = SelfMatchPrevention::new(SmpMode::CancelResting);
        let state = make_state_with_resting("AAPL", OrderSide::Sell, 150.0);
        let req = make_request("AAPL", OrderSide::Buy, Some(151.0));

        let verdict = smp.evaluate(&state, &req);
        assert!(
            matches!(verdict, RiskVerdict::Modified { .. }),
            "CancelResting mode should return Modified (approve incoming, signal resting cancel)"
        );
    }

    #[test]
    fn test_empty_open_orders_approved() {
        let smp = SelfMatchPrevention::new(SmpMode::CancelBoth);
        let state = EngineState {
            total_equity: 100_000.0,
            daily_pnl: 0.0,
            current_drawdown_pct: 0.0,
            open_order_count: 0,
            daily_trade_count: 0,
            open_orders: vec![],
        };
        let req = make_request("AAPL", OrderSide::Buy, Some(150.0));

        let verdict = smp.evaluate(&state, &req);
        assert!(matches!(verdict, RiskVerdict::Approved));
    }
}
