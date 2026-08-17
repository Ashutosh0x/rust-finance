#![forbid(unsafe_code)]
//! Nasdaq direct connectivity.
//!
//! Four protocols, in the layers Nasdaq stacks them:
//!
//! ```text
//!   application   ITCH 5.0 (market data)      OUCH 4.2 (order entry)
//!                        │                            │
//!   session       ┌──────┴──────┐              ┌──────┴──────┐
//!                 │ SoupBinTCP  │  MoldUDP64   │ SoupBinTCP  │
//!                 │  (TCP 1:1)  │ (UDP 1:many) │  (TCP 1:1)  │
//!   transport     └─────────────┴──────────────┴─────────────┘
//! ```
//!
//! * [`itch`] — TotalView-ITCH 5.0 decoder, symbol directory and book mapping.
//! * [`ouch`] — OUCH 4.2 order entry: inbound encoders, outbound decoders.
//! * [`soupbintcp`] — SoupBinTCP 3.00 session layer with sequence-resumed reconnect.
//! * [`moldudp64`] — MoldUDP64 multicast receiver with gap detection and re-request.
//!
//! Both feed and order-entry sessions require a Nasdaq subscription, an assigned OUCH
//! account or ITCH login, and network access to the Carteret data centre (direct connect,
//! extranet or co-location). Nothing here fabricates a session when those are absent: an
//! unconfigured connector fails to start and says why.

pub mod itch;
pub mod moldudp64;
pub mod ouch;
pub mod soupbintcp;

/// Errors from the Nasdaq session layers.
#[derive(Debug, thiserror::Error)]
pub enum NasdaqError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("wire decode failed: {0}")]
    Wire(#[from] exchange_core::WireError),

    #[error("SoupBinTCP login rejected: {0}")]
    LoginRejected(soupbintcp::LoginRejectReason),

    #[error("session ended: {0}")]
    SessionEnded(String),

    #[error("protocol violation: {0}")]
    Protocol(String),

    #[error("not configured: {0}")]
    NotConfigured(String),

    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
}

pub type Result<T> = std::result::Result<T, NasdaqError>;
