//! Configuration for direct exchange sessions, and the fail-closed rules around it.
//!
//! Every value here is issued by an exchange under a subscription or connectivity agreement:
//! multicast groups, Soup credentials, OUCH accounts, XDP product and channel identifiers,
//! Request Server source ids, Pillar MPIDs. None of them can be guessed, and a plausible
//! wrong value is worse than a missing one — a feed handler pointed at the wrong multicast
//! group simply receives nothing, and an order sent with the wrong MPID is rejected at best.
//!
//! So configuration is loaded strictly:
//!
//! * every required field must be present and syntactically valid;
//! * [`VenueConfig::from_env`] returns `NotConfigured` naming the missing variable rather
//!   than substituting a default;
//! * nothing in this crate constructs a session from partial configuration.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use nasdaq::moldudp64::MoldConfig;
use nasdaq::soupbintcp::SoupConfig;
use nyse::xdp::receiver::{ChannelConfig, FeedLine};
use nyse::xdp::request_server::RequestServerConfig;

/// Configuration errors, all of which mean "do not start".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is not set; it is issued by the exchange and cannot be defaulted")]
    Missing(&'static str),
    #[error("{name} is invalid: {reason}")]
    Invalid { name: &'static str, reason: String },
}

type Result<T> = std::result::Result<T, ConfigError>;

fn env(name: &'static str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or(ConfigError::Missing(name))
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn parse_socket(name: &'static str, value: &str) -> Result<SocketAddrV4> {
    value.parse().map_err(|e| ConfigError::Invalid {
        name,
        reason: format!("{value:?} is not a valid host:port ({e})"),
    })
}

fn parse_u8(name: &'static str, value: &str) -> Result<u8> {
    value.trim().parse().map_err(|e| ConfigError::Invalid {
        name,
        reason: format!("{value:?} is not a valid 0..=255 value ({e})"),
    })
}

/// How ITCH is delivered.
///
/// MoldUDP64 is the co-location path: unshaped multicast, and the only option for the FPGA
/// feed. SoupBinTCP is a point-to-point TCP stream with sequence-resumed reconnect, which is
/// what an extranet or disaster-recovery connection uses.
#[derive(Debug, Clone)]
pub enum ItchTransport {
    MoldUdp64(MoldConfig),
    SoupBinTcp(SoupConfig),
}

/// Nasdaq market data and order entry.
#[derive(Debug, Clone)]
pub struct NasdaqConfig {
    pub itch: ItchTransport,
    /// GLIMPSE snapshot service, used to obtain the current book state on a mid-day start.
    /// GLIMPSE speaks ITCH over SoupBinTCP, so it reuses the same session type.
    pub glimpse: Option<SoupConfig>,
    /// OUCH order entry over SoupBinTCP. `None` disables order entry entirely, which is the
    /// right setting for a market-data-only deployment.
    pub ouch: Option<OuchConfig>,
    /// UNIX epoch nanoseconds of the session's local midnight in US/Eastern. ITCH timestamps
    /// are relative to it.
    pub session_midnight_epoch_nanos: u64,
}

/// OUCH order-entry settings.
#[derive(Debug, Clone)]
pub struct OuchConfig {
    pub session: SoupConfig,
    /// Firm identifier sent on each order. Blank uses the OUCH account's default firm; a
    /// non-blank value requires a Service Bureau agreement covering that firm.
    pub firm: String,
    /// Prefix for generated order tokens. Tokens must be day-unique per OUCH account and
    /// are only 14 bytes, so the prefix has to be short.
    pub token_prefix: String,
}

impl OuchConfig {
    pub fn validate(&self) -> Result<()> {
        if self.firm.len() > 4 {
            return Err(ConfigError::Invalid {
                name: "NASDAQ_OUCH_FIRM",
                reason: format!("{:?} exceeds the 4-byte Firm field", self.firm),
            });
        }
        // Leave room for a monotonic counter in the 14-byte token.
        if self.token_prefix.len() > 6 {
            return Err(ConfigError::Invalid {
                name: "NASDAQ_OUCH_TOKEN_PREFIX",
                reason: format!(
                    "{:?} leaves too little of the 14-byte token for a unique suffix",
                    self.token_prefix
                ),
            });
        }
        if !self.token_prefix.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(ConfigError::Invalid {
                name: "NASDAQ_OUCH_TOKEN_PREFIX",
                reason: "OUCH tokens accept only letters, digits and spaces".into(),
            });
        }
        Ok(())
    }
}

/// Which Pillar order-entry protocol to use.
#[derive(Debug, Clone)]
pub enum PillarOrderEntry {
    Binary(PillarBinaryConfig),
    Fix(PillarFixConfig),
}

#[derive(Debug, Clone)]
pub struct PillarBinaryConfig {
    /// `host:port` of the gateway.
    pub addr: String,
    /// Firm identifier.
    pub mpid: String,
    /// Integer market-maker identifier, or 0 when not applicable.
    pub mmid: u32,
    /// Desk identifier within the firm.
    pub mp_sub_id: char,
    /// The TG (Trader to Gateway) stream this session writes to.
    pub stream_id: u64,
    pub market: nyse::pillar::Market,
}

#[derive(Debug, Clone)]
pub struct PillarFixConfig {
    pub addr: String,
    pub session: nyse::pillar::fix::FixSessionConfig,
    pub market: nyse::pillar::Market,
}

/// NYSE market data and order entry.
#[derive(Debug, Clone)]
pub struct NyseConfig {
    /// One entry per Integrated Feed multicast channel subscribed to.
    pub channels: Vec<ChannelConfig>,
    /// Request Server, for retransmission and refresh. Optional only because a firm may
    /// choose to run without recovery; the receiver then reports gaps it cannot close.
    pub request_server: Option<RequestServerConfig>,
    pub order_entry: Option<PillarOrderEntry>,
}

/// Everything needed to bring up direct connectivity.
#[derive(Debug, Clone, Default)]
pub struct VenueConfig {
    pub nasdaq: Option<NasdaqConfig>,
    pub nyse: Option<NyseConfig>,
}

impl VenueConfig {
    /// Load from the environment.
    ///
    /// A venue is configured only if its primary market-data variable is present; otherwise
    /// it is left out entirely rather than half-built. Order entry is opt-in on top of that,
    /// so a market-data-only deployment cannot accidentally acquire the ability to trade.
    ///
    /// Nasdaq:
    /// * `NASDAQ_ITCH_MOLD_GROUP` — multicast `host:port` (co-location path), **or**
    /// * `NASDAQ_ITCH_SOUP_ADDR` + `NASDAQ_ITCH_SOUP_USER` + `NASDAQ_ITCH_SOUP_PASS`
    /// * `NASDAQ_ITCH_INTERFACE`, `NASDAQ_MOLD_REREQUEST_SERVERS` (comma separated)
    /// * `NASDAQ_OUCH_ADDR`, `NASDAQ_OUCH_USER`, `NASDAQ_OUCH_PASS`, `NASDAQ_OUCH_FIRM`,
    ///   `NASDAQ_OUCH_TOKEN_PREFIX`
    ///
    /// NYSE:
    /// * `NYSE_XDP_PRODUCT_ID`, `NYSE_XDP_CHANNEL_ID`, `NYSE_XDP_GROUP_A`
    /// * `NYSE_XDP_GROUP_B`, `NYSE_XDP_INTERFACE`
    /// * `NYSE_REQUEST_SERVER_ADDR`, `NYSE_REQUEST_SERVER_SOURCE_ID`
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            nasdaq: Self::nasdaq_from_env()?,
            nyse: Self::nyse_from_env()?,
        })
    }

    fn nasdaq_from_env() -> Result<Option<NasdaqConfig>> {
        let itch = if let Some(group) = env_opt("NASDAQ_ITCH_MOLD_GROUP") {
            let mut mold = MoldConfig::new(parse_socket("NASDAQ_ITCH_MOLD_GROUP", &group)?);
            if let Some(iface) = env_opt("NASDAQ_ITCH_INTERFACE") {
                mold.interface = iface.parse().map_err(|e| ConfigError::Invalid {
                    name: "NASDAQ_ITCH_INTERFACE",
                    reason: format!("{iface:?} is not an IPv4 address ({e})"),
                })?;
            }
            if let Some(servers) = env_opt("NASDAQ_MOLD_REREQUEST_SERVERS") {
                for s in servers.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    mold.request_servers
                        .push(s.parse().map_err(|e| ConfigError::Invalid {
                            name: "NASDAQ_MOLD_REREQUEST_SERVERS",
                            reason: format!("{s:?} is not a valid host:port ({e})"),
                        })?);
                }
            }
            ItchTransport::MoldUdp64(mold)
        } else if let Some(addr) = env_opt("NASDAQ_ITCH_SOUP_ADDR") {
            ItchTransport::SoupBinTcp(SoupConfig::new(
                addr,
                env("NASDAQ_ITCH_SOUP_USER")?,
                env("NASDAQ_ITCH_SOUP_PASS")?,
            ))
        } else {
            return Ok(None);
        };

        let glimpse = match env_opt("NASDAQ_GLIMPSE_ADDR") {
            Some(addr) => Some(SoupConfig::new(
                addr,
                env("NASDAQ_GLIMPSE_USER")?,
                env("NASDAQ_GLIMPSE_PASS")?,
            )),
            None => None,
        };

        let ouch = match env_opt("NASDAQ_OUCH_ADDR") {
            Some(addr) => {
                let cfg = OuchConfig {
                    session: SoupConfig::new(
                        addr,
                        env("NASDAQ_OUCH_USER")?,
                        env("NASDAQ_OUCH_PASS")?,
                    ),
                    firm: env_opt("NASDAQ_OUCH_FIRM").unwrap_or_default(),
                    token_prefix: env_opt("NASDAQ_OUCH_TOKEN_PREFIX")
                        .unwrap_or_else(|| "RF".into()),
                };
                cfg.validate()?;
                Some(cfg)
            }
            None => None,
        };

        Ok(Some(NasdaqConfig {
            itch,
            glimpse,
            ouch,
            session_midnight_epoch_nanos: session_midnight_epoch_nanos(),
        }))
    }

    fn nyse_from_env() -> Result<Option<NyseConfig>> {
        let Some(group_a) = env_opt("NYSE_XDP_GROUP_A") else {
            return Ok(None);
        };

        let product_id = parse_u8("NYSE_XDP_PRODUCT_ID", &env("NYSE_XDP_PRODUCT_ID")?)?;
        let channel_id = parse_u8("NYSE_XDP_CHANNEL_ID", &env("NYSE_XDP_CHANNEL_ID")?)?;
        let interface: Ipv4Addr = match env_opt("NYSE_XDP_INTERFACE") {
            Some(v) => v.parse().map_err(|e| ConfigError::Invalid {
                name: "NYSE_XDP_INTERFACE",
                reason: format!("{v:?} is not an IPv4 address ({e})"),
            })?,
            None => Ipv4Addr::UNSPECIFIED,
        };

        let line_a = FeedLine::on_interface(parse_socket("NYSE_XDP_GROUP_A", &group_a)?, interface);
        let mut channel = ChannelConfig::new(product_id, channel_id, line_a);
        if let Some(group_b) = env_opt("NYSE_XDP_GROUP_B") {
            channel = channel.with_line_b(FeedLine::on_interface(
                parse_socket("NYSE_XDP_GROUP_B", &group_b)?,
                interface,
            ));
        }

        let request_server = match env_opt("NYSE_REQUEST_SERVER_ADDR") {
            Some(addr) => Some(RequestServerConfig::new(
                addr,
                env("NYSE_REQUEST_SERVER_SOURCE_ID")?,
                product_id,
                channel_id,
            )),
            None => None,
        };

        Ok(Some(NyseConfig {
            channels: vec![channel],
            request_server,
            order_entry: None,
        }))
    }

    /// Reject a configuration that cannot work, before any socket is opened.
    pub fn validate(&self) -> Result<()> {
        if let Some(n) = &self.nasdaq {
            match &n.itch {
                ItchTransport::MoldUdp64(m) => m.validate().map_err(|e| ConfigError::Invalid {
                    name: "NASDAQ_ITCH_MOLD_GROUP",
                    reason: e.to_string(),
                })?,
                ItchTransport::SoupBinTcp(s) => s.validate().map_err(|e| ConfigError::Invalid {
                    name: "NASDAQ_ITCH_SOUP_ADDR",
                    reason: e.to_string(),
                })?,
            }
            if let Some(o) = &n.ouch {
                o.validate()?;
                o.session.validate().map_err(|e| ConfigError::Invalid {
                    name: "NASDAQ_OUCH_ADDR",
                    reason: e.to_string(),
                })?;
            }
            if n.session_midnight_epoch_nanos == 0 {
                return Err(ConfigError::Invalid {
                    name: "session_midnight_epoch_nanos",
                    reason: "ITCH timestamps are relative to it; 0 would date every event to 1970"
                        .into(),
                });
            }
        }

        if let Some(n) = &self.nyse {
            if n.channels.is_empty() {
                return Err(ConfigError::Invalid {
                    name: "NYSE_XDP_GROUP_A",
                    reason: "no Integrated Feed channels configured".into(),
                });
            }
            for c in &n.channels {
                c.validate().map_err(|e| ConfigError::Invalid {
                    name: "NYSE_XDP_GROUP_A",
                    reason: e.to_string(),
                })?;
            }
            if let Some(rs) = &n.request_server {
                rs.validate().map_err(|e| ConfigError::Invalid {
                    name: "NYSE_REQUEST_SERVER_ADDR",
                    reason: e.to_string(),
                })?;
            }
        }

        Ok(())
    }

    /// True when at least one venue is configured. A deployment with neither is a
    /// misconfiguration, not a quiet no-op.
    pub fn any_configured(&self) -> bool {
        self.nasdaq.is_some() || self.nyse.is_some()
    }
}

/// UNIX epoch nanoseconds of today's midnight in US/Eastern.
///
/// ITCH publishes nanoseconds since local midnight, so this is the offset that turns those
/// into absolute time. Eastern is UTC−5, or UTC−4 while daylight saving is in effect; the
/// US rule since 2007 is the second Sunday in March to the first Sunday in November.
pub fn session_midnight_epoch_nanos() -> u64 {
    use chrono::{Datelike, Utc};
    let now = Utc::now();
    let date = now.date_naive();
    let offset_hours = if is_us_eastern_dst(date) { 4 } else { 5 };
    // Local midnight is `offset_hours` after UTC midnight of the same date.
    let utc_midnight = date.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
    let epoch_secs = utc_midnight.timestamp() + offset_hours * 3600;
    debug_assert!(
        date.year() >= 2007,
        "the DST rule below assumes the post-2007 dates"
    );
    (epoch_secs.max(0) as u64) * 1_000_000_000
}

/// Whether US Eastern daylight time is in effect on `date`.
///
/// Approximate at the two boundary days, where the change happens at 02:00 local rather
/// than at midnight; a session's ITCH timestamps are all after 04:00 ET in practice, so the
/// date-level answer is the one that matters.
fn is_us_eastern_dst(date: chrono::NaiveDate) -> bool {
    use chrono::{Datelike, NaiveDate, Weekday};

    let year = date.year();
    let nth_weekday = |month: u32, weekday: Weekday, n: u32| -> NaiveDate {
        let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
        let offset = (7 + weekday.num_days_from_sunday() - first.weekday().num_days_from_sunday())
            % 7
            + (n - 1) * 7;
        first + chrono::Duration::days(offset as i64)
    };

    let start = nth_weekday(3, Weekday::Sun, 2); // second Sunday in March
    let end = nth_weekday(11, Weekday::Sun, 1); // first Sunday in November
    date >= start && date < end
}

/// Latency and reliability expectations for a session, used by the preflight checks.
#[derive(Debug, Clone, Copy)]
pub struct LatencyBudget {
    /// Warn above this round-trip to the venue.
    pub max_rtt: Duration,
    /// Warn above this wire-to-book latency at the 99th percentile.
    pub max_p99_book: Duration,
}

impl Default for LatencyBudget {
    /// Defaults sized for a co-located deployment: exchange documentation puts
    /// order-to-acknowledgement and order-to-tick inside the Carteret and Mahwah data
    /// centres under 50 microseconds, so anything into the milliseconds means the session is
    /// not running where it is supposed to be.
    fn default() -> Self {
        Self {
            max_rtt: Duration::from_micros(500),
            max_p99_book: Duration::from_micros(50),
        }
    }
}

impl LatencyBudget {
    /// A budget appropriate for an extranet or cloud connection, where milliseconds are
    /// expected and a microsecond target would be noise.
    pub fn extranet() -> Self {
        Self {
            max_rtt: Duration::from_millis(20),
            max_p99_book: Duration::from_millis(2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn dst_boundaries_follow_the_post_2007_us_rule() {
        // 2026: DST runs 8 March to 1 November.
        assert!(!is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2026, 3, 7).unwrap()
        ));
        assert!(is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2026, 3, 8).unwrap()
        ));
        assert!(is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2026, 8, 17).unwrap()
        ));
        assert!(is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
        ));
        assert!(!is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        ));
        assert!(!is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2026, 12, 25).unwrap()
        ));
    }

    #[test]
    fn dst_boundaries_are_recomputed_per_year() {
        // 2027: second Sunday in March is the 14th, first Sunday in November is the 7th.
        assert!(!is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2027, 3, 13).unwrap()
        ));
        assert!(is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2027, 3, 14).unwrap()
        ));
        assert!(is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2027, 11, 6).unwrap()
        ));
        assert!(!is_us_eastern_dst(
            NaiveDate::from_ymd_opt(2027, 11, 7).unwrap()
        ));
    }

    #[test]
    fn session_midnight_is_a_plausible_recent_timestamp() {
        let ns = session_midnight_epoch_nanos();
        assert!(ns > 1_700_000_000_000_000_000, "after 2023");
        assert_eq!(ns % 1_000_000_000, 0, "midnight lands on a whole second");
    }

    #[test]
    fn an_empty_config_is_reported_as_unconfigured_rather_than_valid() {
        let cfg = VenueConfig::default();
        assert!(!cfg.any_configured());
        // An empty config passes validation but must not be mistaken for a working one.
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn a_zero_session_midnight_is_rejected() {
        let cfg = VenueConfig {
            nasdaq: Some(NasdaqConfig {
                itch: ItchTransport::MoldUdp64(MoldConfig::new(
                    "233.54.12.111:26477".parse().unwrap(),
                )),
                glimpse: None,
                ouch: None,
                session_midnight_epoch_nanos: 0,
            }),
            nyse: None,
        };
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid { .. })));
    }

    #[test]
    fn a_unicast_itch_group_is_rejected() {
        let cfg = VenueConfig {
            nasdaq: Some(NasdaqConfig {
                itch: ItchTransport::MoldUdp64(MoldConfig::new("10.0.0.1:26477".parse().unwrap())),
                glimpse: None,
                ouch: None,
                session_midnight_epoch_nanos: session_midnight_epoch_nanos(),
            }),
            nyse: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn ouch_settings_are_checked_against_the_wire_field_widths() {
        let mut o = OuchConfig {
            session: SoupConfig::new("host:1", "user", "pass"),
            firm: "ABCDE".into(),
            token_prefix: "RF".into(),
        };
        assert!(o.validate().is_err(), "firm over 4 bytes");

        o.firm = "ABCD".into();
        o.token_prefix = "TOOLONGPREFIX".into();
        assert!(o.validate().is_err(), "prefix leaves no room for a counter");

        o.token_prefix = "RF-".into();
        assert!(o.validate().is_err(), "hyphen is not allowed in a token");

        o.token_prefix = "RF".into();
        assert!(o.validate().is_ok());
    }

    #[test]
    fn nyse_config_requires_at_least_one_channel() {
        let cfg = VenueConfig {
            nasdaq: None,
            nyse: Some(NyseConfig {
                channels: vec![],
                request_server: None,
                order_entry: None,
            }),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn latency_budgets_reflect_where_the_session_actually_runs() {
        let colo = LatencyBudget::default();
        let extranet = LatencyBudget::extranet();
        assert!(colo.max_rtt < extranet.max_rtt);
        assert!(colo.max_p99_book < extranet.max_p99_book);
        assert!(colo.max_rtt <= Duration::from_micros(500));
    }
}
