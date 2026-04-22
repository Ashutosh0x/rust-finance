#![forbid(unsafe_code)]
// crates/oms/src/lib.rs
//
// Order Management System (OMS)
// Manages the order lifecycle, position tracking, and pre-trade compliance checks.

pub mod blotter;
pub mod order;
pub mod position;
pub mod sebi;

pub use blotter::{ComplianceError, ComplianceLimits, OrderBlotter};
pub use order::{Order, OrderEvent, OrderStatus, OrderType, Side, TimeInForce};
pub use position::{Position, PositionManager};
pub use sebi::{OrderVariety, SebiCompliance, SebiConfig, SebiViolation};
