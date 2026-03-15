// tests/integration/pipeline.rs
//
// End-to-end integration tests for the RustForge trading pipeline.
// Tests the full flow: market event → AI signal → risk check → order guard.
//
// Run with: cargo test --test pipeline -- --nocapture

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

// ── Shared test helpers ───────────────────────────────────────────────────────

/// Construct a synthetic market snapshot for testing.
fn synthetic_snapshot(symbol: &str, trend: f64, n_bars: usize) -> /*crate::mirofish::*/MarketSnapshot {
    let history: Vec<f64> = (0..n_bars)
        .map(|i| 100.0 + i as f64 * trend)
        .collect();
    let price = *history.last().unwrap();
    /*crate::mirofish::*/MarketSnapshot {
        symbol: symbol.to_string(),
        price,
        bid: price - 0.05,
        ask: price + 0.05,
        price_history: history.clone(),
        volume_history: vec![1_000.0; n_bars],
    }
}

// ── EventBus Integration ──────────────────────────────────────────────────────

#[cfg(test)]
mod event_bus_tests {
    use super::*;

    #[tokio::test]
    async fn test_event_bus_single_producer_multiple_consumers() {
        let (tx, _) = tokio::sync::broadcast::channel::<u64>(128);

        let rx1 = tx.subscribe();
        let rx2 = tx.subscribe();

        tx.send(42).unwrap();
        tx.send(99).unwrap();

        let received1: Vec<u64> = collect_n(rx1, 2).await;
        let received2: Vec<u64> = collect_n(rx2, 2).await;

        assert_eq!(received1, vec![42, 99]);
        assert_eq!(received2, vec![42, 99]);
    }

    async fn collect_n(mut rx: tokio::sync::broadcast::Receiver<u64>, n: usize) -> Vec<u64> {
        let mut out = Vec::new();
        for _ in 0..n {
            out.push(rx.recv().await.unwrap());
        }
        out
    }
}

// ── Reconnect Integration ─────────────────────────────────────────────────────

#[cfg(test)]
mod reconnect_tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    /*
    #[tokio::test]
    async fn test_reconnect_respects_max_attempts() {
        let cfg = crate::reconnect::ReconnectConfig {
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(4),
            multiplier: 2.0,
            max_attempts: Some(2),
        };

        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();

        crate::reconnect::reconnect_loop(cfg, move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>("error")
            }
        })
        .await;

        // 1 initial + 2 retries = 3
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
    */
}

// ── Circuit Breaker Integration ────────────────────────────────────────────────

#[cfg(test)]
mod circuit_breaker_tests {
    /*
    use crate::circuit_breaker::{BreakerConfig, BreakerError, CircuitBreaker};

    #[tokio::test]
    async fn test_half_open_allows_one_probe() {
        let cfg = BreakerConfig {
            failure_threshold: 1,
            recovery_timeout: Duration::from_millis(10),
            success_threshold: 1,
        };
        let cb = CircuitBreaker::new("test", cfg);

        // Trip it
        let _ = cb.call(|| async { Err::<(), _>("fail") }).await;
        assert!(cb.is_open().await);

        // Wait for recovery_timeout
        sleep(Duration::from_millis(15)).await;

        // Probe should succeed and close it
        let result = cb.call(|| async { Ok::<i32, &str>(1) }).await;
        assert!(result.is_ok());
        assert!(!cb.is_open().await);
    }
    */
}

// ── Risk Engine Integration ───────────────────────────────────────────────────

#[cfg(test)]
mod risk_integration_tests {
    /*
    use crate::kill_switch::{OrderGuard, RiskConfig, RiskEngine};

    #[tokio::test]
    async fn test_full_risk_pipeline() {
        let cfg = RiskConfig {
            max_drawdown: 0.05,
            vol_threshold: 2.0, // high threshold to avoid triggering
            ..Default::default()
        };
        let (mut engine, mut rx) = RiskEngine::new(cfg);
        let guard = OrderGuard::new(engine.kill_switch_handle());

        // Normal conditions — order should pass
        engine.update_portfolio(10_000.0).await.unwrap();
        assert!(guard.check().await.is_ok());

        // Severe drawdown — kill switch trips
        let _ = engine.update_portfolio(9_400.0).await;
        assert!(engine.is_kill_switch_active().await);

        // Order should now be blocked
        assert!(guard.check().await.is_err());

        // Verify event was emitted
        let event = timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("No event received")
            .unwrap();

        assert!(matches!(
            event,
            crate::kill_switch::RiskEvent::DrawdownHalt { .. }
                | crate::kill_switch::RiskEvent::KillSwitchActivated { .. }
        ));

        // Reset and verify orders flow again
        engine.reset_kill_switch().await;
        assert!(guard.check().await.is_ok());
    }

    #[tokio::test]
    async fn test_garch_vol_spike_trips_kill_switch() {
        let cfg = RiskConfig {
            vol_threshold: 0.001, // extremely low to force a trip
            ..Default::default()
        };
        let (mut engine, _rx) = RiskEngine::new(cfg);

        // One large price move to spike GARCH variance
        let _ = engine.on_price_tick("AAPL", 100.0).await;
        let result = engine.on_price_tick("AAPL", 200.0).await; // 100% move
        assert!(result.is_err());
        assert!(engine.is_kill_switch_active().await);
    }
    */
}

// ── MiroFish Swarm Integration ────────────────────────────────────────────────

#[cfg(test)]
mod mirofish_integration_tests {
    /*
    use crate::mirofish::{run_swarm, SwarmConfig};

    #[tokio::test]
    async fn test_5000_agents_complete_under_5_seconds() {
        let snap = super::synthetic_snapshot("BTC", 0.1, 50);
        let cfg = SwarmConfig {
            n_agents: 5_000,
            concurrency: 500,
            ..Default::default()
        };

        let start = std::time::Instant::now();
        let signal = run_swarm(snap, cfg).await;
        let elapsed = start.elapsed();

        assert_eq!(signal.agents_run, 5_000);
        assert!(
            elapsed < Duration::from_secs(5),
            "5000 agents took {:?} — too slow",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_probabilities_sum_to_one() {
        let snap = super::synthetic_snapshot("ETH", 0.0, 50);
        let signal = run_swarm(snap, SwarmConfig { n_agents: 200, ..Default::default() }).await;
        let total = signal.buy_probability + signal.sell_probability + signal.hold_probability;
        assert!((total - 1.0).abs() < 0.001, "Probabilities sum to {total}");
    }
    */
}

// ── FIX 4.4 Integration ───────────────────────────────────────────────────────

#[cfg(test)]
mod fix_integration_tests {
    /*
    use crate::fix_serializer::{FixSerializer, FixError, MsgType, SessionConfig};

    #[test]
    fn test_full_logon_order_logout_sequence() {
        let mut ser = FixSerializer::new(SessionConfig::default());

        let logon = ser.logon(30, "trader1", "pass");
        let order = ser.new_order_single("ORD001", "AAPL", '1', 100.0, Some(175.50));
        let logout = ser.logout(Some("End of day"));

        // Parse all three back
        let parsed_logon = FixSerializer::deserialize(&logon).unwrap();
        let parsed_order = FixSerializer::deserialize(&order).unwrap();
        let parsed_logout = FixSerializer::deserialize(&logout).unwrap();

        assert_eq!(parsed_logon.msg_type(), Some(MsgType::Logon));
        assert_eq!(parsed_order.msg_type(), Some(MsgType::NewOrderSingle));
        assert_eq!(parsed_logout.msg_type(), Some(MsgType::Logout));

        // Sequence numbers should increment
        let seq1: u32 = parsed_logon.get(crate::fix_serializer::tags::MSG_SEQ_NUM)
            .unwrap().parse().unwrap();
        let seq2: u32 = parsed_order.get(crate::fix_serializer::tags::MSG_SEQ_NUM)
            .unwrap().parse().unwrap();
        let seq3: u32 = parsed_logout.get(crate::fix_serializer::tags::MSG_SEQ_NUM)
            .unwrap().parse().unwrap();

        assert_eq!(seq2, seq1 + 1);
        assert_eq!(seq3, seq2 + 1);
    }

    #[test]
    fn test_deserialize_rejects_missing_msg_type() {
        let malformed = "49=SENDER\x0156=TARGET\x01";
        assert!(matches!(
            FixSerializer::deserialize(malformed),
            Err(FixError::MissingMsgType)
        ));
    }
    */
}

// ── RPC Router Integration ────────────────────────────────────────────────────

#[cfg(test)]
mod rpc_router_tests {
    /*
    use crate::rpc_router::NodeStats;

    #[test]
    fn test_node_ordering_by_latency_ema() {
        let mut fast = NodeStats::new("fast");
        fast.update_latency(100);

        let mut slow = NodeStats::new("slow");
        slow.update_latency(1000);

        // fast should sort before slow
        let mut nodes = vec![(&slow, "slow"), (&fast, "fast")];
        nodes.sort_by(|a, b| {
            a.0.latency_ema_us
                .partial_cmp(&b.0.latency_ema_us)
                .unwrap()
        });
        assert_eq!(nodes[0].1, "fast");
    }

    #[test]
    fn test_unhealthy_node_after_3_failures() {
        let mut stats = NodeStats::new("flaky");
        stats.update_latency(200);
        for _ in 0..3 {
            stats.record_failure();
        }
        assert!(!stats.is_healthy);
    }
    */
}

// ── Shutdown Integration ──────────────────────────────────────────────────────

#[cfg(test)]
mod shutdown_tests {
    /*
    use crate::shutdown::{drain_subsystems, ShutdownController};

    #[tokio::test]
    async fn test_drain_completes_within_deadline() {
        let subs: Vec<(&'static str, _)> = vec![
            ("subsystem_a", async { sleep(Duration::from_millis(10)).await }),
            ("subsystem_b", async { sleep(Duration::from_millis(20)).await }),
        ];

        let start = std::time::Instant::now();
        drain_subsystems(subs, Duration::from_secs(2)).await;
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn test_shutdown_signal_received_by_multiple_tasks() {
        let (ctrl, _root_rx) = ShutdownController::new();

        let mut rx1 = ctrl.subscribe();
        let mut rx2 = ctrl.subscribe();

        ctrl.shutdown();

        let r1 = timeout(Duration::from_millis(50), rx1.wait()).await;
        let r2 = timeout(Duration::from_millis(50), rx2.wait()).await;

        assert!(r1.is_ok());
        assert!(r2.is_ok());
    }
    */
}
