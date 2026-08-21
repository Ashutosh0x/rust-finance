//! Nasdaq OUCH 4.2 as a RustForge [`ExecutionGateway`].
//!
//! The gateway owns one SoupBinTCP session in a background task and correlates
//! request/response by the 14-byte OUCH `Order Token`, which the *client* assigns. That is
//! the pivot the whole design turns on:
//!
//! * Nasdaq never acknowledges an inbound message directly; it emits an Accepted, Rejected
//!   or Canceled message carrying the token back.
//! * A token that has already been used is *silently ignored*, so resending an in-flight
//!   order after a socket failure is safe and is the documented recovery.
//! * Tokens must be day-unique per OUCH account, and uniqueness is entirely the client's
//!   responsibility. [`TokenAllocator`] is therefore not a convenience — a collision means
//!   an order that vanishes without a reject.
//!
//! `submit_order` waits for the terminal-or-working response for its token rather than
//! returning as soon as the bytes are written, because a write that succeeds tells you
//! nothing: Unsequenced Data is explicitly not guaranteed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::events::{OrderAccepted, OrderEvent, OrderRejected, OrderSide, OrderType};
use compact_str::CompactString;
use exchange_core::Price;
use execution::gateway::{ExecutionGateway, OpenRequest, TimeInForce};
use nasdaq::ouch::{
    self, Capacity, CrossType, Display, EnterOrder, IsoEligibility, OrderToken, Outbound,
};
use nasdaq::soupbintcp::{SoupEvent, SoupSession};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::config::OuchConfig;

/// How long to wait for the exchange's response to an order before giving up.
///
/// Nasdaq acknowledges in microseconds from co-location; a second is not a latency budget,
/// it is the point at which the session is presumed broken.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(1);

/// Allocates day-unique OUCH order tokens.
///
/// The 14-byte field is split into a short caller-supplied prefix and a base-36 counter, so
/// a two-character prefix leaves room for more orders per day than any single OUCH account
/// is permitted to send.
#[derive(Debug)]
pub struct TokenAllocator {
    prefix: String,
    counter: AtomicU64,
}

impl TokenAllocator {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            counter: AtomicU64::new(1),
        }
    }

    /// Next token. Monotonic within the process; across restarts the caller must not reuse
    /// a prefix on the same OUCH account and day.
    pub fn next(&self) -> OrderToken {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        OrderToken::new(&format!("{}{}", self.prefix, to_base36(n)))
    }

    /// Resume the counter after a restart, so a re-run on the same day cannot collide with
    /// tokens already sent.
    pub fn resume_at(&self, next: u64) {
        self.counter.store(next.max(1), Ordering::Relaxed);
    }
}

fn to_base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Terminal or working response for one order.
#[derive(Debug, Clone)]
pub enum OrderOutcome {
    Accepted {
        reference_number: u64,
        shares: u32,
    },
    Rejected {
        reason: ouch::RejectReason,
    },
    /// Accepted and immediately dead (an IOC with nothing to match, for instance).
    AcceptedAndDead {
        reason: &'static str,
    },
}

type Pending = Arc<Mutex<HashMap<OrderToken, oneshot::Sender<OrderOutcome>>>>;

/// A live OUCH order-entry session.
pub struct OuchGateway {
    outbound: mpsc::Sender<Vec<u8>>,
    pending: Pending,
    tokens: TokenAllocator,
    config: OuchConfig,
}

impl OuchGateway {
    /// Connect, log in, and start the session task.
    pub async fn connect(config: OuchConfig) -> Result<Self, anyhow::Error> {
        config
            .validate()
            .map_err(|e| anyhow::anyhow!("invalid OUCH configuration: {e}"))?;
        let session = SoupSession::connect(&config.session).await?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (outbound_tx, outbound_rx) = mpsc::channel::<Vec<u8>>(1024);

        let task_pending = Arc::clone(&pending);
        tokio::spawn(async move {
            // Boxed: carries the OUCH session buffers. Once per connection.
            if let Err(e) = Box::pin(run_session(session, outbound_rx, task_pending)).await {
                tracing::error!(target: "exchange::ouch", error = %e, "OUCH session ended");
            }
        });

        let tokens = TokenAllocator::new(config.token_prefix.clone());
        Ok(Self {
            outbound: outbound_tx,
            pending,
            tokens,
            config,
        })
    }

    /// Translate a RustForge order request into an OUCH Enter Order message.
    ///
    /// Two mappings deserve attention:
    ///
    /// * **Market orders.** OUCH has no order-type field; a market order is expressed as a
    ///   limit at the documented sentinel price, `$214,748.3647`.
    /// * **Time in force.** OUCH's field is a lifetime in *seconds*, not an enum. `IOC` is
    ///   0, `DAY` is the Market Hours sentinel, and `GTC` has no representation at all — an
    ///   equity order cannot outlive the session, so it is rejected rather than quietly
    ///   downgraded to a day order.
    pub fn to_enter_order(
        &self,
        req: &OpenRequest,
        token: OrderToken,
    ) -> Result<EnterOrder, anyhow::Error> {
        let side = match req.side {
            OrderSide::Buy => ouch::Side::Buy,
            OrderSide::Sell => ouch::Side::Sell,
        };

        let price = match req.order_type {
            OrderType::Limit => {
                let px = req
                    .limit_price
                    .ok_or_else(|| anyhow::anyhow!("limit order without a limit price"))?;
                Price::from_f64(px)
            }
            OrderType::Market => Price::from_price4(Price::OUCH_MARKET_SENTINEL_RAW),
        };

        let time_in_force = match req.time_in_force {
            TimeInForce::IOC | TimeInForce::FOK => ouch::time_in_force::IOC,
            TimeInForce::DAY => ouch::time_in_force::MARKET_HOURS,
            TimeInForce::GTC => {
                return Err(anyhow::anyhow!(
                    "OUCH has no good-till-cancelled: an order can live at most until the \
                     end of the Nasdaq trading day"
                ))
            }
        };

        // FOK is minimum-quantity-equals-full-size on an immediate-or-cancel order; OUCH
        // has no separate fill-or-kill flag.
        let min_quantity = match req.time_in_force {
            TimeInForce::FOK => req.quantity.round() as u32,
            _ => 0,
        };

        let order = EnterOrder {
            token,
            side,
            shares: req.quantity.round() as u32,
            stock: exchange_core::Symbol8::try_new(req.symbol.as_str())
                .map_err(|e| anyhow::anyhow!("symbol rejected: {e}"))?,
            price,
            time_in_force,
            firm: exchange_core::Mpid4::new(&self.config.firm),
            display: Display::Anonymous,
            capacity: Capacity::Agency,
            iso_eligibility: IsoEligibility::NotEligible,
            min_quantity,
            cross_type: CrossType::None,
            customer_type: ' ',
        };
        order
            .validate()
            .map_err(|e| anyhow::anyhow!("order rejected before send: {e}"))?;
        Ok(order)
    }

    /// Cancel the remaining balance of an order.
    pub async fn cancel(&self, token: OrderToken) -> Result<(), anyhow::Error> {
        let msg = ouch::CancelOrder::full(token).encode();
        self.outbound
            .send(msg)
            .await
            .map_err(|_| anyhow::anyhow!("OUCH session is no longer running"))?;
        Ok(())
    }

    /// Send a raw inbound message and wait for the response keyed to `token`.
    async fn send_and_await(
        &self,
        token: OrderToken,
        bytes: Vec<u8>,
    ) -> Result<OrderOutcome, anyhow::Error> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(token, tx);

        if self.outbound.send(bytes).await.is_err() {
            self.pending.lock().await.remove(&token);
            return Err(anyhow::anyhow!("OUCH session is no longer running"));
        }

        match tokio::time::timeout(RESPONSE_TIMEOUT, rx).await {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(_)) => Err(anyhow::anyhow!("OUCH session dropped the pending order")),
            Err(_) => {
                self.pending.lock().await.remove(&token);
                Err(anyhow::anyhow!(
                    "no OUCH response for token {token} within {RESPONSE_TIMEOUT:?}; the order \
                     may or may not be live — resend the same token to find out safely"
                ))
            }
        }
    }
}

#[async_trait]
impl ExecutionGateway for OuchGateway {
    fn name(&self) -> &str {
        "nasdaq-ouch"
    }

    async fn submit_order(&self, req: OpenRequest) -> Result<OrderEvent, anyhow::Error> {
        req.validate()?;
        let token = self.tokens.next();
        let order = self.to_enter_order(&req, token)?;

        let outcome = self.send_and_await(token, order.encode()).await?;
        Ok(match outcome {
            OrderOutcome::Accepted {
                reference_number, ..
            } => OrderEvent::Accepted(OrderAccepted {
                client_order_id: req.client_order_id.clone(),
                // The reference number is the same value ITCH publishes for this order,
                // which is what lets the firm find it in the public book.
                venue_order_id: CompactString::new(reference_number.to_string()),
            }),
            OrderOutcome::AcceptedAndDead { reason } => OrderEvent::Rejected(OrderRejected {
                client_order_id: req.client_order_id.clone(),
                reason: CompactString::new(reason),
            }),
            OrderOutcome::Rejected { reason } => OrderEvent::Rejected(OrderRejected {
                client_order_id: req.client_order_id.clone(),
                reason: CompactString::new(format!("{reason:?}")),
            }),
        })
    }
}

/// The session task: writes outbound messages, reads and dispatches inbound ones.
async fn run_session(
    mut session: SoupSession,
    mut outbound: mpsc::Receiver<Vec<u8>>,
    pending: Pending,
) -> Result<(), String> {
    loop {
        tokio::select! {
            Some(bytes) = outbound.recv() => {
                session.send(&bytes).await.map_err(|e| e.to_string())?;
            }
            event = session.next_event() => {
                let payload = match event.map_err(|e| e.to_string())? {
                    SoupEvent::Message { payload, .. } => payload.to_vec(),
                    SoupEvent::EndOfSession => return Ok(()),
                    _ => continue,
                };
                dispatch(&payload, &pending).await;
            }
        }
    }
}

/// Route one outbound OUCH message to whoever is waiting on its token.
async fn dispatch(payload: &[u8], pending: &Pending) {
    let msg = match Outbound::decode(payload) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(target: "exchange::ouch", error = %e, "OUCH decode failed");
            return;
        }
    };

    let Some(token) = msg.token() else {
        return;
    };

    let outcome = match &msg {
        Outbound::Accepted(ack) => {
            if ack.order_state == ouch::OrderState::Dead {
                // Order Dead means accepted and then immediately cancelled; no further
                // messages will arrive for this token, so it must not be left waiting.
                Some(OrderOutcome::AcceptedAndDead {
                    reason: "accepted and immediately cancelled (Order Dead)",
                })
            } else {
                Some(OrderOutcome::Accepted {
                    reference_number: ack.reference_number,
                    shares: ack.shares,
                })
            }
        }
        Outbound::Rejected(r) => Some(OrderOutcome::Rejected { reason: r.reason }),
        _ => None,
    };

    if let Some(outcome) = outcome {
        if let Some(waiter) = pending.lock().await.remove(&token) {
            let _ = waiter.send(outcome);
        }
    }

    // Executions, cancels and breaks arrive after the acknowledgement and belong to the
    // position keeper, not to the submitting call.
    match &msg {
        Outbound::Executed(e) => tracing::info!(
            target: "exchange::ouch",
            token = %token,
            shares = e.executed_shares,
            price = %e.execution_price,
            liquidity = %e.liquidity_flag,
            match_number = e.match_number,
            "fill"
        ),
        Outbound::Canceled(c) => tracing::info!(
            target: "exchange::ouch",
            token = %token,
            shares = c.decrement_shares,
            reason = ?c.reason,
            "cancelled"
        ),
        Outbound::BrokenTrade(b) => tracing::warn!(
            target: "exchange::ouch",
            token = %token,
            match_number = b.match_number,
            reason = ?b.reason,
            "trade broken"
        ),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nasdaq::soupbintcp::SoupConfig;

    fn config() -> OuchConfig {
        OuchConfig {
            session: SoupConfig::new("host:1234", "USER01", "secret"),
            firm: "ABCD".into(),
            token_prefix: "RF".into(),
        }
    }

    /// A gateway with no live session, for testing pure translation.
    fn offline_gateway() -> OuchGateway {
        let (tx, _rx) = mpsc::channel(1);
        OuchGateway {
            outbound: tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
            tokens: TokenAllocator::new("RF"),
            config: config(),
        }
    }

    fn request(
        side: OrderSide,
        ty: OrderType,
        tif: TimeInForce,
        price: Option<f64>,
    ) -> OpenRequest {
        OpenRequest {
            client_order_id: CompactString::new("C1"),
            symbol: CompactString::new("AAPL"),
            side,
            quantity: 500.0,
            order_type: ty,
            limit_price: price,
            time_in_force: tif,
        }
    }

    #[test]
    fn tokens_are_unique_and_fit_the_fourteen_byte_field() {
        let a = TokenAllocator::new("RF");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            let t = a.next();
            assert!(t.as_str().len() <= 14);
            assert!(seen.insert(t), "token collision: {t}");
        }
    }

    #[test]
    fn tokens_can_resume_after_a_restart() {
        let a = TokenAllocator::new("RF");
        let first = a.next();
        a.resume_at(1_000_000);
        let after = a.next();
        assert_ne!(first, after);
        assert_eq!(after.as_str(), format!("RF{}", to_base36(1_000_000)));
    }

    #[test]
    fn base36_encoding_is_compact_and_monotonic() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "Z");
        assert_eq!(to_base36(36), "10");
        // Twelve base-36 digits is far more than a day's order count.
        assert!(to_base36(u64::MAX).len() <= 13);
    }

    #[test]
    fn a_limit_order_maps_to_the_documented_fields() {
        let g = offline_gateway();
        let order = g
            .to_enter_order(
                &request(
                    OrderSide::Buy,
                    OrderType::Limit,
                    TimeInForce::DAY,
                    Some(123.45),
                ),
                OrderToken::new("T1"),
            )
            .unwrap();
        assert_eq!(order.side, ouch::Side::Buy);
        assert_eq!(order.shares, 500);
        assert_eq!(order.stock, "AAPL");
        assert_eq!(order.price.to_string(), "123.45");
        assert_eq!(order.time_in_force, ouch::time_in_force::MARKET_HOURS);
        assert_eq!(order.firm, "ABCD");
        assert_eq!(order.min_quantity, 0);
    }

    #[test]
    fn a_market_order_uses_the_sentinel_price_because_ouch_has_no_order_type() {
        let g = offline_gateway();
        let order = g
            .to_enter_order(
                &request(OrderSide::Sell, OrderType::Market, TimeInForce::DAY, None),
                OrderToken::new("T2"),
            )
            .unwrap();
        assert_eq!(order.price.to_price4(), Price::OUCH_MARKET_SENTINEL_RAW);
        assert_eq!(order.side, ouch::Side::Sell);
    }

    #[test]
    fn ioc_is_a_time_in_force_of_zero_seconds() {
        let g = offline_gateway();
        let order = g
            .to_enter_order(
                &request(
                    OrderSide::Buy,
                    OrderType::Limit,
                    TimeInForce::IOC,
                    Some(100.0),
                ),
                OrderToken::new("T3"),
            )
            .unwrap();
        assert_eq!(order.time_in_force, ouch::time_in_force::IOC);
        assert_eq!(order.min_quantity, 0);
    }

    #[test]
    fn fok_is_expressed_as_ioc_with_a_full_size_minimum() {
        let g = offline_gateway();
        let order = g
            .to_enter_order(
                &request(
                    OrderSide::Buy,
                    OrderType::Limit,
                    TimeInForce::FOK,
                    Some(100.0),
                ),
                OrderToken::new("T4"),
            )
            .unwrap();
        assert_eq!(order.time_in_force, ouch::time_in_force::IOC);
        assert_eq!(order.min_quantity, 500, "all or nothing");
    }

    #[test]
    fn gtc_is_rejected_rather_than_downgraded_to_a_day_order() {
        let g = offline_gateway();
        let err = g
            .to_enter_order(
                &request(
                    OrderSide::Buy,
                    OrderType::Limit,
                    TimeInForce::GTC,
                    Some(100.0),
                ),
                OrderToken::new("T5"),
            )
            .unwrap_err();
        assert!(err.to_string().contains("good-till-cancelled"));
    }

    #[test]
    fn a_limit_order_without_a_price_is_refused_locally() {
        let g = offline_gateway();
        assert!(g
            .to_enter_order(
                &request(OrderSide::Buy, OrderType::Limit, TimeInForce::DAY, None),
                OrderToken::new("T6"),
            )
            .is_err());
    }

    #[test]
    fn an_oversized_order_is_refused_before_it_reaches_the_wire() {
        let g = offline_gateway();
        let mut req = request(
            OrderSide::Buy,
            OrderType::Limit,
            TimeInForce::DAY,
            Some(100.0),
        );
        req.quantity = 1_000_000.0;
        let err = g.to_enter_order(&req, OrderToken::new("T7")).unwrap_err();
        assert!(err.to_string().contains("1,000,000"));
    }

    #[tokio::test]
    async fn an_accepted_message_resolves_the_waiting_order() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let token = OrderToken::new("T8");
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(token, tx);

        let ack = Outbound::Accepted(ouch::Acknowledgement {
            timestamp: 1,
            token,
            side: ouch::Side::Buy,
            shares: 500,
            stock: exchange_core::Symbol8::new("AAPL"),
            price: Price::from_f64(123.45),
            time_in_force: ouch::time_in_force::MARKET_HOURS,
            firm: "ABCD".into(),
            display: Display::Anonymous,
            reference_number: 987_654,
            capacity: Capacity::Agency,
            iso_eligibility: IsoEligibility::NotEligible,
            min_quantity: 0,
            cross_type: CrossType::None,
            order_state: ouch::OrderState::Live,
            previous_token: None,
            bbo_weight_indicator: '0',
        });
        dispatch(&ack.encode(), &pending).await;

        match rx.await.unwrap() {
            OrderOutcome::Accepted {
                reference_number,
                shares,
            } => {
                assert_eq!(reference_number, 987_654);
                assert_eq!(shares, 500);
            }
            other => panic!("expected Accepted, got {other:?}"),
        }
        assert!(pending.lock().await.is_empty(), "waiter is removed");
    }

    #[tokio::test]
    async fn order_dead_does_not_leave_the_caller_waiting_forever() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let token = OrderToken::new("T9");
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(token, tx);

        let ack = Outbound::Accepted(ouch::Acknowledgement {
            timestamp: 1,
            token,
            side: ouch::Side::Buy,
            shares: 0,
            stock: exchange_core::Symbol8::new("AAPL"),
            price: Price::from_f64(1.0),
            time_in_force: ouch::time_in_force::IOC,
            firm: exchange_core::Mpid4::BLANK,
            display: Display::Anonymous,
            reference_number: 0,
            capacity: Capacity::Agency,
            iso_eligibility: IsoEligibility::NotEligible,
            min_quantity: 0,
            cross_type: CrossType::None,
            order_state: ouch::OrderState::Dead,
            previous_token: None,
            bbo_weight_indicator: ' ',
        });
        dispatch(&ack.encode(), &pending).await;
        assert!(matches!(
            rx.await.unwrap(),
            OrderOutcome::AcceptedAndDead { .. }
        ));
    }

    #[tokio::test]
    async fn a_reject_resolves_the_waiting_order_with_its_reason() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let token = OrderToken::new("TA");
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(token, tx);

        let reject = Outbound::Rejected(ouch::Rejected {
            timestamp: 1,
            token,
            reason: ouch::RejectReason::RiskFatFinger,
        });
        dispatch(&reject.encode(), &pending).await;
        match rx.await.unwrap() {
            OrderOutcome::Rejected { reason } => {
                assert_eq!(reason, ouch::RejectReason::RiskFatFinger);
                assert!(reason.is_risk_control());
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_fill_does_not_resolve_the_submit_call() {
        // Executions belong to the position keeper; the submit call resolves on the ack.
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let token = OrderToken::new("TB");
        let (tx, mut rx) = oneshot::channel();
        pending.lock().await.insert(token, tx);

        let fill = Outbound::Executed(ouch::Executed {
            timestamp: 1,
            token,
            executed_shares: 100,
            execution_price: Price::from_f64(123.45),
            liquidity_flag: 'R',
            match_number: 42,
            reference_price: None,
            reference_price_type: None,
        });
        dispatch(&fill.encode(), &pending).await;
        assert!(
            rx.try_recv().is_err(),
            "still waiting for the acknowledgement"
        );
        assert_eq!(pending.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn submitting_on_a_dead_session_errors_rather_than_hanging() {
        let g = offline_gateway();
        // The receiver was dropped when the gateway was built, so the send fails.
        let err = g
            .submit_order(request(
                OrderSide::Buy,
                OrderType::Limit,
                TimeInForce::DAY,
                Some(100.0),
            ))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no longer running"));
    }

    #[test]
    fn the_gateway_names_itself_for_routing() {
        assert_eq!(offline_gateway().name(), "nasdaq-ouch");
    }
}
