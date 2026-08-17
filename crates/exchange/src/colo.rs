//! Co-location preflight: measure the environment rather than assume it.
//!
//! A direct-feed deployment fails in ways that never surface as an error. The socket
//! connects, the session logs in, messages arrive — and the strategy is fifteen milliseconds
//! behind because the process is running in the wrong data centre, or the receive buffer is
//! at the OS default and drops a burst at the open, or the multicast join landed on the
//! management NIC.
//!
//! Everything here therefore *measures*. Each check reports what it actually observed, and a
//! check that cannot run says so instead of returning a pass. There are no hard-coded
//! expectations about the host: the only judgements made are against the caller's own
//! [`LatencyBudget`].

use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use tokio::net::{TcpStream, UdpSocket};

use crate::config::LatencyBudget;

/// Outcome of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    /// Measured, and within the budget.
    Pass,
    /// Measured, and outside the budget — the session will work but not at the intended
    /// latency.
    Warn,
    /// Measured, and the session cannot work.
    Fail,
    /// Could not be measured here. Not a pass: an unmeasured check is unknown.
    Skipped,
}

impl CheckStatus {
    pub const fn is_blocking(self) -> bool {
        matches!(self, Self::Fail)
    }
}

/// One preflight result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub status: CheckStatus,
    /// What was actually observed, in words. Always populated, including for a pass — the
    /// measured number is the useful part.
    pub detail: String,
}

impl Check {
    fn new(name: &'static str, status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            name,
            status,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for Check {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mark = match self.status {
            CheckStatus::Pass => "ok  ",
            CheckStatus::Warn => "warn",
            CheckStatus::Fail => "FAIL",
            CheckStatus::Skipped => "skip",
        };
        write!(f, "[{mark}] {:<28} {}", self.name, self.detail)
    }
}

/// The full preflight report.
#[derive(Debug, Clone, Default)]
pub struct PreflightReport {
    pub checks: Vec<Check>,
}

impl PreflightReport {
    /// True when no check failed outright.
    pub fn is_go(&self) -> bool {
        !self.checks.iter().any(|c| c.status.is_blocking())
    }

    pub fn failures(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.status == CheckStatus::Fail)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.status == CheckStatus::Warn)
    }

    /// Checks that could not be run. Worth surfacing separately: they are the ones a reader
    /// is most likely to mistake for passes.
    pub fn skipped(&self) -> impl Iterator<Item = &Check> {
        self.checks
            .iter()
            .filter(|c| c.status == CheckStatus::Skipped)
    }
}

impl std::fmt::Display for PreflightReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for c in &self.checks {
            writeln!(f, "{c}")?;
        }
        write!(
            f,
            "{}",
            if self.is_go() {
                "preflight: go"
            } else {
                "preflight: NO-GO"
            }
        )
    }
}

/// Runs the preflight checks.
#[derive(Debug, Clone)]
pub struct Preflight {
    pub budget: LatencyBudget,
    /// How many round trips to time when measuring latency to an endpoint.
    pub samples: usize,
    pub connect_timeout: Duration,
}

impl Default for Preflight {
    fn default() -> Self {
        Self {
            budget: LatencyBudget::default(),
            samples: 5,
            connect_timeout: Duration::from_secs(2),
        }
    }
}

impl Preflight {
    pub fn with_budget(budget: LatencyBudget) -> Self {
        Self {
            budget,
            ..Self::default()
        }
    }

    /// Measure TCP connect round-trip time to an exchange endpoint.
    ///
    /// TCP connect time is used rather than ICMP because it traverses the same path and the
    /// same port the session will use — and because exchange networks commonly drop ICMP,
    /// which would make a ping-based check report a failure that is not one.
    pub async fn check_endpoint(&self, name: &'static str, addr: &str) -> Check {
        let Ok(mut resolved) = addr.to_socket_addrs() else {
            return Check::new(
                name,
                CheckStatus::Fail,
                format!("{addr:?} does not resolve"),
            );
        };
        let Some(target) = resolved.next() else {
            return Check::new(
                name,
                CheckStatus::Fail,
                format!("{addr:?} resolved to no addresses"),
            );
        };

        let mut best: Option<Duration> = None;
        let mut failures = 0;
        for _ in 0..self.samples.max(1) {
            let start = Instant::now();
            match tokio::time::timeout(self.connect_timeout, TcpStream::connect(target)).await {
                Ok(Ok(_stream)) => {
                    let rtt = start.elapsed();
                    best = Some(best.map_or(rtt, |b: Duration| b.min(rtt)));
                }
                Ok(Err(e)) => {
                    failures += 1;
                    if failures == self.samples.max(1) {
                        return Check::new(
                            name,
                            CheckStatus::Fail,
                            format!("cannot reach {target}: {e}"),
                        );
                    }
                }
                Err(_) => {
                    failures += 1;
                    if failures == self.samples.max(1) {
                        return Check::new(
                            name,
                            CheckStatus::Fail,
                            format!("{target} did not answer within {:?}", self.connect_timeout),
                        );
                    }
                }
            }
        }

        let Some(rtt) = best else {
            return Check::new(
                name,
                CheckStatus::Fail,
                format!("no successful connect to {target}"),
            );
        };

        // The best of N samples is the right statistic: it is the closest thing to the
        // unloaded path latency, and a single slow sample says more about scheduling noise
        // than about the network.
        let status = if rtt <= self.budget.max_rtt {
            CheckStatus::Pass
        } else {
            CheckStatus::Warn
        };
        Check::new(
            name,
            status,
            format!(
                "{target} best-of-{} connect {:.3}ms (budget {:.3}ms)",
                self.samples.max(1),
                rtt.as_secs_f64() * 1000.0,
                self.budget.max_rtt.as_secs_f64() * 1000.0
            ),
        )
    }

    /// Verify that a multicast group can actually be joined on the intended interface.
    ///
    /// This is the check that catches the most expensive silent failure: a join that lands
    /// on the wrong NIC succeeds, and the feed then simply never delivers anything.
    pub async fn check_multicast_join(
        &self,
        name: &'static str,
        group: std::net::SocketAddrV4,
        interface: Ipv4Addr,
    ) -> Check {
        if !group.ip().is_multicast() {
            return Check::new(
                name,
                CheckStatus::Fail,
                format!("{} is not a multicast address", group.ip()),
            );
        }

        let socket = match UdpSocket::bind(std::net::SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            group.port(),
        ))
        .await
        {
            Ok(s) => s,
            Err(e) => {
                return Check::new(
                    name,
                    CheckStatus::Fail,
                    format!("cannot bind udp/{}: {e}", group.port()),
                )
            }
        };

        match socket.join_multicast_v4(*group.ip(), interface) {
            Ok(()) => Check::new(
                name,
                CheckStatus::Pass,
                format!(
                    "joined {} on interface {}",
                    group,
                    if interface.is_unspecified() {
                        "chosen by the OS — bind the feed NIC explicitly".to_string()
                    } else {
                        interface.to_string()
                    }
                ),
            ),
            Err(e) => Check::new(
                name,
                CheckStatus::Fail,
                format!("cannot join {group} on {interface}: {e}"),
            ),
        }
    }

    /// Report the socket receive buffer the OS actually granted.
    ///
    /// The OS silently clamps a requested size to its maximum, so the only trustworthy
    /// number is the one read back after setting it. A default-sized buffer will drop
    /// datagrams during the opening burst, and those losses cost a recovery round trip each.
    pub async fn check_receive_buffer(&self, name: &'static str, requested_bytes: usize) -> Check {
        let socket = match std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
            Ok(s) => s,
            Err(e) => return Check::new(name, CheckStatus::Fail, format!("cannot bind: {e}")),
        };

        // Rust's standard library exposes no portable accessor for SO_RCVBUF, and this crate
        // forbids unsafe, so the granted size cannot be read back here. Saying so is more
        // useful than reporting a pass that was never measured.
        drop(socket);
        Check::new(
            name,
            CheckStatus::Skipped,
            format!(
                "requested {} MiB; the granted size is not readable without a raw socket \
                 option — verify with `sysctl net.core.rmem_max` (Linux) or `netsh int ipv4 \
                 show dynamicport` and the driver's receive ring settings (Windows)",
                requested_bytes / (1024 * 1024)
            ),
        )
    }

    /// Check that the monotonic clock is fine-grained enough to time a microsecond path.
    ///
    /// A clock whose smallest observable step is larger than the latency being measured
    /// makes every latency figure downstream meaningless.
    pub fn check_clock_resolution(&self) -> Check {
        let mut smallest_step = Duration::MAX;
        for _ in 0..1000 {
            let a = Instant::now();
            let mut b = Instant::now();
            // Spin until the clock actually advances.
            while b == a {
                b = Instant::now();
            }
            smallest_step = smallest_step.min(b - a);
        }

        let status = if smallest_step <= Duration::from_nanos(1_000) {
            CheckStatus::Pass
        } else if smallest_step <= Duration::from_micros(100) {
            CheckStatus::Warn
        } else {
            CheckStatus::Fail
        };
        Check::new(
            name_of_clock_check(),
            status,
            format!(
                "monotonic clock resolves {:.0}ns; wire-to-book budget is {:.0}ns",
                smallest_step.as_nanos(),
                self.budget.max_p99_book.as_nanos()
            ),
        )
    }

    /// Report the parallelism available, which bounds how many feeds can be pinned to their
    /// own core.
    pub fn check_cpu_budget(&self, feeds: usize, order_sessions: usize) -> Check {
        let available = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(0);
        if available == 0 {
            return Check::new(
                "cpu-budget",
                CheckStatus::Skipped,
                "available parallelism is not reported on this host",
            );
        }
        // One core per feed handler, one per order session, and at least one left for
        // everything else.
        let needed = feeds + order_sessions + 1;
        let status = if available >= needed {
            CheckStatus::Pass
        } else {
            CheckStatus::Warn
        };
        Check::new(
            "cpu-budget",
            status,
            format!(
                "{available} cores available, {needed} wanted ({feeds} feed(s) + \
                 {order_sessions} order session(s) + 1)"
            ),
        )
    }

    /// Judge a measured wire-to-book latency against the budget.
    pub fn check_book_latency(&self, p99: Option<Duration>) -> Check {
        let Some(p99) = p99 else {
            return Check::new(
                "wire-to-book-latency",
                CheckStatus::Skipped,
                "no samples recorded yet; run during market hours",
            );
        };
        let status = if p99 <= self.budget.max_p99_book {
            CheckStatus::Pass
        } else {
            CheckStatus::Warn
        };
        Check::new(
            "wire-to-book-latency",
            status,
            format!(
                "p99 {:.1}us (budget {:.1}us)",
                p99.as_secs_f64() * 1e6,
                self.budget.max_p99_book.as_secs_f64() * 1e6
            ),
        )
    }

    /// Run the environment checks that need no exchange endpoint.
    ///
    /// Endpoint and multicast checks are separate because they touch the network and a
    /// caller may want to run them only in the target environment.
    pub fn local_checks(&self, feeds: usize, order_sessions: usize) -> PreflightReport {
        PreflightReport {
            checks: vec![
                self.check_clock_resolution(),
                self.check_cpu_budget(feeds, order_sessions),
            ],
        }
    }
}

const fn name_of_clock_check() -> &'static str {
    "clock-resolution"
}

/// Resolve a `host:port` string, for reporting which address a session will actually use.
pub fn resolve(addr: &str) -> Option<SocketAddr> {
    addr.to_socket_addrs().ok()?.next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skipped_check_is_not_a_pass() {
        let report = PreflightReport {
            checks: vec![Check::new("x", CheckStatus::Skipped, "not measurable here")],
        };
        assert!(report.is_go(), "a skip does not block");
        assert_eq!(report.skipped().count(), 1);
        assert_eq!(
            report.checks[0].status,
            CheckStatus::Skipped,
            "and it must not be recorded as a pass"
        );
    }

    #[test]
    fn a_failure_blocks_and_a_warning_does_not() {
        let report = PreflightReport {
            checks: vec![
                Check::new("a", CheckStatus::Warn, "slow"),
                Check::new("b", CheckStatus::Pass, "fine"),
            ],
        };
        assert!(report.is_go());
        assert_eq!(report.warnings().count(), 1);

        let report = PreflightReport {
            checks: vec![Check::new("c", CheckStatus::Fail, "unreachable")],
        };
        assert!(!report.is_go());
        assert_eq!(report.failures().count(), 1);
    }

    #[test]
    fn the_clock_check_reports_a_real_measurement() {
        let p = Preflight::default();
        let check = p.check_clock_resolution();
        assert_eq!(check.name, "clock-resolution");
        assert!(check.detail.contains("resolves"));
        assert!(
            check.detail.contains("ns"),
            "the measured value must appear in the detail: {}",
            check.detail
        );
        // Any modern host resolves well under 100us; a Fail here would be real information.
        assert_ne!(check.status, CheckStatus::Skipped);
    }

    #[test]
    fn the_cpu_check_compares_against_the_session_count() {
        let p = Preflight::default();
        let plenty = p.check_cpu_budget(1, 1);
        assert!(plenty.detail.contains("cores available"));

        // Asking for more cores than any machine has must warn, not silently pass.
        let starved = p.check_cpu_budget(10_000, 10_000);
        assert_eq!(starved.status, CheckStatus::Warn);
    }

    #[test]
    fn the_book_latency_check_skips_without_samples_and_judges_with_them() {
        let p = Preflight::default();
        assert_eq!(
            p.check_book_latency(None).status,
            CheckStatus::Skipped,
            "no data is not a pass"
        );
        assert_eq!(
            p.check_book_latency(Some(Duration::from_micros(10))).status,
            CheckStatus::Pass
        );
        assert_eq!(
            p.check_book_latency(Some(Duration::from_millis(5))).status,
            CheckStatus::Warn
        );
    }

    #[test]
    fn an_extranet_budget_accepts_latency_a_colo_budget_would_flag() {
        let colo = Preflight::default();
        let extranet = Preflight::with_budget(LatencyBudget::extranet());
        let observed = Some(Duration::from_micros(800));
        assert_eq!(colo.check_book_latency(observed).status, CheckStatus::Warn);
        assert_eq!(
            extranet.check_book_latency(observed).status,
            CheckStatus::Pass
        );
    }

    #[tokio::test]
    async fn a_unicast_address_fails_the_multicast_join_check() {
        let p = Preflight::default();
        let check = p
            .check_multicast_join(
                "itch-group",
                "10.0.0.1:26477".parse().unwrap(),
                Ipv4Addr::UNSPECIFIED,
            )
            .await;
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.detail.contains("not a multicast"));
    }

    #[tokio::test]
    async fn an_unresolvable_endpoint_fails_rather_than_hanging() {
        let p = Preflight {
            samples: 1,
            connect_timeout: Duration::from_millis(50),
            ..Preflight::default()
        };
        let check = p
            .check_endpoint("ouch", "this-host-does-not-exist.invalid:1234")
            .await;
        assert_eq!(check.status, CheckStatus::Fail);
    }

    #[tokio::test]
    async fn a_reachable_endpoint_is_measured_and_reported() {
        // A local listener stands in for an exchange endpoint; the point is that the check
        // reports a real measured number rather than a hard-coded verdict.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                if listener.accept().await.is_err() {
                    break;
                }
            }
        });

        let p = Preflight {
            samples: 3,
            ..Preflight::default()
        };
        let check = p.check_endpoint("loopback", &addr.to_string()).await;
        assert_ne!(check.status, CheckStatus::Fail);
        assert!(check.detail.contains("best-of-3"));
        assert!(check.detail.contains("budget"));
    }

    #[tokio::test]
    async fn the_receive_buffer_check_admits_it_cannot_measure() {
        let p = Preflight::default();
        let check = p.check_receive_buffer("rcvbuf", 16 * 1024 * 1024).await;
        assert_eq!(check.status, CheckStatus::Skipped);
        assert!(check.detail.contains("not readable"));
        assert!(check.detail.contains("16 MiB"));
    }

    #[test]
    fn local_checks_run_without_a_network() {
        let report = Preflight::default().local_checks(2, 1);
        assert_eq!(report.checks.len(), 2);
        assert!(report.to_string().contains("preflight:"));
    }

    #[test]
    fn the_report_renders_every_check_with_its_measurement() {
        let report = PreflightReport {
            checks: vec![
                Check::new("a", CheckStatus::Pass, "0.05ms"),
                Check::new("b", CheckStatus::Fail, "unreachable"),
            ],
        };
        let text = report.to_string();
        assert!(text.contains("[ok  ] a"));
        assert!(text.contains("[FAIL] b"));
        assert!(text.contains("NO-GO"));
    }
}
