#![forbid(unsafe_code)]
//! NYSE (ICE) Pillar direct connectivity.
//!
//! ```text
//!   market data                              order entry
//!   ───────────                              ───────────
//!   XDP Integrated Feed  (UDP multicast)     Pillar Binary Gateway  (native)
//!   XDP control messages (types 1,2,3,32,34) Pillar FIX Gateway     (FIX 4.2)
//!   Pillar Request Server (TCP recovery)
//! ```
//!
//! * [`xdp`] — packet framing, control and referential messages, the Integrated Feed message
//!   set, an A/B-arbitrating multicast receiver and the Request Server client.
//! * [`pillar`] — order entry over the binary gateway or FIX 4.2.
//!
//! Everything here needs an NYSE data agreement, assigned product and channel identifiers,
//! a Source ID for the Request Server and network reach to the Mahwah data centre. Nothing
//! in this crate invents a session or a price when those are missing: an unconfigured
//! connector refuses to start and names what it lacks.

pub mod pillar;
pub mod xdp;

/// Errors from the NYSE session and recovery layers.
#[derive(Debug, thiserror::Error)]
pub enum NyseError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("wire decode failed: {0}")]
    Wire(#[from] exchange_core::WireError),

    #[error("protocol violation: {0}")]
    Protocol(String),

    #[error("session ended: {0}")]
    SessionEnded(String),

    #[error("request rejected: {0}")]
    RequestRejected(xdp::request_server::RequestStatus),

    #[error("not configured: {0}")]
    NotConfigured(String),

    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
}

pub type Result<T> = std::result::Result<T, NyseError>;
