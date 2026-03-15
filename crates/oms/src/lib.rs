// crates/oms/src/lib.rs
//
// Order Management System (OMS)
// Manages the order lifecycle, position tracking, and pre-trade compliance checks.

pub mod order;
pub mod position;
pub mod blotter;
pub mod sebi;

pub use order::{Order, OrderStatus, OrderEvent, OrderType, Side, TimeInForce};
pub use position::{Position, PositionManager};
pub use blotter::{OrderBlotter, ComplianceLimits, ComplianceError};
pub use sebi::{SebiCompliance, SebiConfig, OrderVariety, SebiViolation};
