#![forbid(unsafe_code)]
//! Direct exchange connectivity, wired into RustForge's ingestion and execution traits.
//!
//! ```text
//!                 ┌──────────────── this crate ────────────────┐
//!   Nasdaq        │                                            │
//!   ITCH 5.0 ─────┤ NasdaqItchSource ──┐                       │
//!   (Mold/Soup)   │                    ├─ Normalizer ─ MarketEvent ─→ ingestion
//!   NYSE          │ NyseXdpSource ─────┘                       │
//!   Integrated ───┤                                            │
//!                 │                                            │
//!   Nasdaq OUCH ──┤ OuchGateway ───────┐                       │
//!   NYSE Pillar ──┤ PillarFixGateway   ├─ OrderEvent ──────────→ execution
//!   (FIX/binary)  │ PillarBinaryGateway┘                       │
//!                 └────────────────────────────────────────────┘
//! ```
//!
//! * [`config`] — session configuration and the fail-closed rules around it.
//! * [`normalize`] — the narrowing point from order-by-order depth to `MarketEvent`.
//! * [`nasdaq_source`] / [`nyse_source`] — [`ingestion::source::MarketDataSource`] adapters.
//! * [`ouch_gateway`] / [`pillar_gateway`] — [`execution::gateway::ExecutionGateway`] adapters.
//! * [`colo`] — preflight checks that measure the environment instead of assuming it.
//! * [`replay`] — drive the same handlers from a recorded capture, so a book builder can be
//!   validated without an entitlement.
//!
//! **What this does not do.** It does not synthesise market data, simulate an exchange
//! session, or supply defaults for values only an exchange can issue. Every connector needs
//! a real subscription, real credentials and real network reach; without them it fails to
//! start and names what is missing. That is deliberate — a connector that quietly produces
//! plausible-looking output is worse than one that does not run.

pub mod colo;
pub mod config;
pub mod nasdaq_source;
pub mod normalize;
pub mod nyse_source;
pub mod ouch_gateway;
pub mod pillar_gateway;
pub mod replay;

pub use config::{
    ConfigError, ItchTransport, LatencyBudget, NasdaqConfig, NyseConfig, OuchConfig,
    PillarBinaryConfig, PillarFixConfig, PillarOrderEntry, VenueConfig,
};
pub use nasdaq_source::NasdaqItchSource;
pub use normalize::Normalizer;
pub use nyse_source::NyseXdpSource;
pub use ouch_gateway::{OuchGateway, TokenAllocator};
pub use pillar_gateway::{PillarBinaryGateway, PillarFixGateway};

/// Venues this crate can connect to directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Venue {
    /// The Nasdaq Stock Market, via TotalView-ITCH and OUCH.
    Nasdaq,
    /// NYSE Group equities, via the Pillar Integrated Feed and the Pillar gateways.
    Nyse,
}

impl Venue {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nasdaq => "nasdaq",
            Self::Nyse => "nyse",
        }
    }

    /// The data centre a co-located session runs in. Relevant because inter-site latency
    /// (Carteret ↔ Mahwah) dominates everything else in a cross-venue strategy.
    pub const fn data_center(self) -> &'static str {
        match self {
            Self::Nasdaq => "Carteret, NJ",
            Self::Nyse => "Mahwah, NJ",
        }
    }
}

impl std::fmt::Display for Venue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Build the market data sources a configuration calls for.
///
/// Returns an empty vector when nothing is configured, which the caller should treat as an
/// error rather than as "run without market data".
pub fn market_data_sources(
    config: &VenueConfig,
) -> Result<Vec<Box<dyn ingestion::source::MarketDataSource>>, ConfigError> {
    config.validate()?;
    let mut sources: Vec<Box<dyn ingestion::source::MarketDataSource>> = Vec::new();
    if let Some(n) = &config.nasdaq {
        sources.push(Box::new(NasdaqItchSource::new(n.clone())));
    }
    if let Some(n) = &config.nyse {
        sources.push(Box::new(NyseXdpSource::new(n.clone())));
    }
    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venues_name_their_data_centres() {
        assert_eq!(Venue::Nasdaq.data_center(), "Carteret, NJ");
        assert_eq!(Venue::Nyse.data_center(), "Mahwah, NJ");
        assert_eq!(Venue::Nasdaq.to_string(), "nasdaq");
    }

    #[test]
    fn an_unconfigured_deployment_builds_no_sources() {
        let sources = market_data_sources(&VenueConfig::default()).unwrap();
        assert!(
            sources.is_empty(),
            "caller must treat this as a misconfiguration"
        );
    }

    #[test]
    fn a_configured_venue_produces_its_source() {
        use nasdaq::moldudp64::MoldConfig;
        let cfg = VenueConfig {
            nasdaq: Some(NasdaqConfig {
                itch: ItchTransport::MoldUdp64(MoldConfig::new(
                    "233.54.12.111:26477".parse().unwrap(),
                )),
                glimpse: None,
                ouch: None,
                session_midnight_epoch_nanos: config::session_midnight_epoch_nanos(),
            }),
            nyse: None,
        };
        let sources = market_data_sources(&cfg).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name(), "nasdaq-itch");
    }

    #[test]
    fn an_invalid_configuration_is_rejected_before_any_source_is_built() {
        use nasdaq::moldudp64::MoldConfig;
        let cfg = VenueConfig {
            nasdaq: Some(NasdaqConfig {
                // Unicast, which cannot carry a MoldUDP64 feed.
                itch: ItchTransport::MoldUdp64(MoldConfig::new("10.0.0.1:26477".parse().unwrap())),
                glimpse: None,
                ouch: None,
                session_midnight_epoch_nanos: config::session_midnight_epoch_nanos(),
            }),
            nyse: None,
        };
        assert!(market_data_sources(&cfg).is_err());
    }
}
