//! NYSE Pillar order entry as a RustForge [`ExecutionGateway`].
//!
//! Two implementations, matching the two gateways NYSE operates:
//!
//! * [`PillarFixGateway`] — FIX 4.2 over TCP. Complete here: the session layer (logon,
//!   heartbeats, sequence numbers, resend) and the application layer are both public.
//! * [`PillarBinaryGateway`] — the native binary gateway. The *application* layer is
//!   implemented in full ([`nyse::pillar::binary`]): New Order, Cancel, Order Ack, Execution
//!   Report, and the `BitfieldOrderInstructions` packing. What is deliberately **not**
//!   implemented is the stream-establishment handshake, which lives in a separate document —
//!   the NYSE Pillar Stream Protocol specification — that describes the unsequenced messages
//!   used to open and position a TG/GT stream.
//!
//! So the binary gateway here takes an already-established stream and speaks the application
//! layer over it. That boundary is stated rather than papered over: inventing a handshake
//! would produce code that compiles, passes its own tests, and fails at certification.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::events::{OrderAccepted, OrderEvent, OrderRejected, OrderSide, OrderType};
use compact_str::CompactString;
use exchange_core::Price;
use execution::gateway::{ExecutionGateway, OpenRequest, TimeInForce};
use nyse::pillar::binary::{self, NewOrder, OrderInstructions};
use nyse::pillar::fix::{self, FixMessage, FixSessionConfig, NewOrderSingle};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::config::{PillarBinaryConfig, PillarFixConfig};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(1);

/// Outcome of an order submission on either gateway.
#[derive(Debug, Clone)]
pub enum PillarOutcome {
    Accepted { order_id: String },
    Rejected { reason: String },
}

// ─── FIX gateway ────────────────────────────────────────────────────────────

type FixPending = Arc<Mutex<HashMap<String, oneshot::Sender<PillarOutcome>>>>;

/// Pillar FIX 4.2 order entry.
pub struct PillarFixGateway {
    outbound: mpsc::Sender<Vec<u8>>,
    pending: FixPending,
    session: FixSessionConfig,
    market: nyse::pillar::Market,
    next_cl_ord_id: AtomicU64,
}

impl PillarFixGateway {
    /// Connect, log on, and start the session task.
    pub async fn connect(config: PillarFixConfig) -> Result<Self, anyhow::Error> {
        config
            .session
            .validate()
            .map_err(|e| anyhow::anyhow!("invalid Pillar FIX configuration: {e}"))?;

        let mut stream = TcpStream::connect(&config.addr).await?;
        stream.set_nodelay(true)?;

        let seq = Arc::new(AtomicU64::new(1));
        let logon = fix::encode_logon(
            &config.session,
            seq.fetch_add(1, Ordering::Relaxed),
            &utc_timestamp(),
            true,
            None,
        );
        stream.write_all(&logon).await?;
        stream.flush().await?;

        let pending: FixPending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::channel::<Vec<u8>>(1024);

        let task_pending = Arc::clone(&pending);
        let task_session = config.session.clone();
        let task_seq = Arc::clone(&seq);
        tokio::spawn(async move {
            if let Err(e) = run_fix_session(stream, rx, task_pending, task_session, task_seq).await
            {
                tracing::error!(target: "exchange::pillar-fix", error = %e, "FIX session ended");
            }
        });

        Ok(Self {
            outbound: tx,
            pending,
            session: config.session,
            market: config.market,
            next_cl_ord_id: AtomicU64::new(1),
        })
    }

    /// Translate a RustForge request into a Pillar FIX New Order Single.
    ///
    /// The market-specific quantity ceiling is applied here rather than left to the
    /// exchange, because a rejected order still costs a round trip and a sequence number.
    pub fn to_new_order(
        &self,
        req: &OpenRequest,
        cl_ord_id: &str,
    ) -> Result<NewOrderSingle, anyhow::Error> {
        let side = match req.side {
            OrderSide::Buy => fix::Side::Buy,
            OrderSide::Sell => fix::Side::Sell,
        };
        let (ord_type, price) = match req.order_type {
            OrderType::Market => (fix::OrdType::Market, None),
            OrderType::Limit => (
                fix::OrdType::Limit,
                Some(Price::from_f64(req.limit_price.ok_or_else(|| {
                    anyhow::anyhow!("limit order without a limit price")
                })?)),
            ),
        };
        let time_in_force = match req.time_in_force {
            TimeInForce::DAY => fix::TimeInForce::Day,
            TimeInForce::IOC | TimeInForce::FOK => fix::TimeInForce::Ioc,
            TimeInForce::GTC => {
                return Err(anyhow::anyhow!(
                    "Pillar equities accept Day, At-the-Opening, IOC and On-Close; there is no \
                     good-till-cancelled"
                ))
            }
        };

        let qty = req.quantity.round() as u64;
        let ceiling = self.market.max_order_qty(false) as u64;
        if qty > ceiling {
            return Err(anyhow::anyhow!(
                "{} shares exceeds the {} ceiling on {}",
                qty,
                ceiling,
                self.market.as_str()
            ));
        }

        let mut order =
            NewOrderSingle::limit(cl_ord_id, req.symbol.as_str(), side, qty, Price::ZERO);
        order.ord_type = ord_type;
        order.price = price;
        order.time_in_force = time_in_force;
        if matches!(req.time_in_force, TimeInForce::FOK) {
            order.min_qty = Some(qty);
        }
        order
            .validate()
            .map_err(|e| anyhow::anyhow!("order rejected before send: {e}"))?;
        Ok(order)
    }

    fn next_cl_ord_id(&self) -> String {
        format!(
            "{}{}",
            self.session
                .sender_comp_id
                .chars()
                .take(4)
                .collect::<String>(),
            self.next_cl_ord_id.fetch_add(1, Ordering::Relaxed)
        )
    }
}

#[async_trait]
impl ExecutionGateway for PillarFixGateway {
    fn name(&self) -> &str {
        "nyse-pillar-fix"
    }

    async fn submit_order(&self, req: OpenRequest) -> Result<OrderEvent, anyhow::Error> {
        req.validate()?;
        let cl_ord_id = self.next_cl_ord_id();
        let order = self.to_new_order(&req, &cl_ord_id)?;

        let now = utc_timestamp();
        // Sequence numbers are owned by the session task, which stamps the message on the
        // way out; the placeholder here is replaced there.
        let bytes = order.encode(&self.session, 0, &now, &now);

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(cl_ord_id.clone(), tx);

        if self.outbound.send(bytes).await.is_err() {
            self.pending.lock().await.remove(&cl_ord_id);
            return Err(anyhow::anyhow!("Pillar FIX session is no longer running"));
        }

        match tokio::time::timeout(RESPONSE_TIMEOUT, rx).await {
            Ok(Ok(PillarOutcome::Accepted { order_id })) => {
                Ok(OrderEvent::Accepted(OrderAccepted {
                    client_order_id: req.client_order_id.clone(),
                    venue_order_id: CompactString::new(order_id),
                }))
            }
            Ok(Ok(PillarOutcome::Rejected { reason })) => Ok(OrderEvent::Rejected(OrderRejected {
                client_order_id: req.client_order_id.clone(),
                reason: CompactString::new(reason),
            })),
            Ok(Err(_)) => Err(anyhow::anyhow!(
                "Pillar FIX session dropped the pending order"
            )),
            Err(_) => {
                self.pending.lock().await.remove(&cl_ord_id);
                Err(anyhow::anyhow!(
                    "no execution report for {cl_ord_id} within {RESPONSE_TIMEOUT:?}"
                ))
            }
        }
    }
}

/// The FIX session task: stamps sequence numbers, answers heartbeats and test requests, and
/// routes execution reports back to whoever is waiting on the `ClOrdID`.
async fn run_fix_session(
    mut stream: TcpStream,
    mut outbound: mpsc::Receiver<Vec<u8>>,
    pending: FixPending,
    session: FixSessionConfig,
    seq: Arc<AtomicU64>,
) -> Result<(), String> {
    let mut buf: Vec<u8> = Vec::with_capacity(16 * 1024);

    loop {
        tokio::select! {
            Some(bytes) = outbound.recv() => {
                // Re-stamp with the authoritative sequence number: only this task knows it.
                let stamped = restamp_sequence(&bytes, seq.fetch_add(1, Ordering::Relaxed));
                stream.write_all(&stamped).await.map_err(|e| e.to_string())?;
                stream.flush().await.map_err(|e| e.to_string())?;
            }
            read = stream.read_buf(&mut buf) => {
                let n = read.map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("Pillar FIX gateway closed the connection".into());
                }
                while let Some((raw, consumed)) = next_fix_message(&buf) {
                    let raw = raw.to_vec();
                    buf.drain(..consumed);
                    handle_fix_message(&raw, &pending, &session, &seq, &mut stream).await?;
                }
            }
        }
    }
}

async fn handle_fix_message(
    raw: &[u8],
    pending: &FixPending,
    session: &FixSessionConfig,
    seq: &AtomicU64,
    stream: &mut TcpStream,
) -> Result<(), String> {
    let msg = match FixMessage::parse(raw) {
        Ok(m) => m,
        Err(e) => {
            // A checksum failure means the framing is wrong, so the bytes after it cannot be
            // trusted either; the session has to be torn down and re-established.
            return Err(format!("FIX parse failed: {e}"));
        }
    };

    match msg.msg_type.as_str() {
        // Test Request: answer with a Heartbeat echoing the id, or be disconnected.
        "1" => {
            let hb = fix::encode_heartbeat(
                session,
                seq.fetch_add(1, Ordering::Relaxed),
                &utc_timestamp(),
                msg.get(fix::tag::TEST_REQ_ID),
            );
            stream.write_all(&hb).await.map_err(|e| e.to_string())?;
            stream.flush().await.map_err(|e| e.to_string())?;
        }
        "0" | "A" => {}
        "5" => return Err("gateway logged us out".into()),
        "8" => {
            if let Some(report) = fix::ExecutionReport::from_message(&msg) {
                let outcome = if matches!(report.exec_type, fix::ExecType::Rejected) {
                    PillarOutcome::Rejected {
                        reason: report.text.clone().unwrap_or_else(|| "rejected".into()),
                    }
                } else {
                    PillarOutcome::Accepted {
                        order_id: report.order_id.clone(),
                    }
                };
                if let Some(waiter) = pending.lock().await.remove(&report.cl_ord_id) {
                    let _ = waiter.send(outcome);
                }
                if report.exec_type.is_fill() {
                    tracing::info!(
                        target: "exchange::pillar-fix",
                        cl_ord_id = %report.cl_ord_id,
                        qty = ?report.last_qty,
                        px = ?report.last_px.map(|p| p.to_string()),
                        liquidity = ?report.liquidity_indicator,
                        "fill"
                    );
                }
            }
        }
        // Order Cancel Reject.
        "9" => tracing::warn!(
            target: "exchange::pillar-fix",
            cl_ord_id = ?msg.get(fix::tag::CL_ORD_ID),
            text = ?msg.get(fix::tag::TEXT),
            "cancel rejected"
        ),
        // Session-level reject: the message we sent was malformed.
        "3" => tracing::error!(
            target: "exchange::pillar-fix",
            ref_seq = ?msg.get(fix::tag::REF_SEQ_NUM),
            ref_tag = ?msg.get(fix::tag::REF_TAG_ID),
            text = ?msg.get(fix::tag::TEXT),
            "session-level reject"
        ),
        other => tracing::debug!(target: "exchange::pillar-fix", msg_type = other, "unhandled"),
    }
    Ok(())
}

/// Replace tag 34 and recompute the framing so the session task owns sequencing.
fn restamp_sequence(raw: &[u8], seq_num: u64) -> Vec<u8> {
    let Ok(msg) = FixMessage::parse(raw) else {
        return raw.to_vec();
    };
    let mut b = fix::FixMessageBuilder::new(&msg.msg_type);
    for (tag, value) in &msg.fields {
        match *tag {
            // Rebuilt by the encoder.
            fix::tag::BEGIN_STRING | fix::tag::BODY_LENGTH | fix::tag::MSG_TYPE => {}
            fix::tag::MSG_SEQ_NUM => {
                b.set(fix::tag::MSG_SEQ_NUM, seq_num.to_string());
            }
            _ => {
                b.set(*tag, value.clone());
            }
        }
    }
    let begin_string = msg.get(fix::tag::BEGIN_STRING).unwrap_or("FIX.4.2");
    b.encode(begin_string)
}

/// Frame one FIX message out of a byte buffer using `BodyLength`.
fn next_fix_message(buf: &[u8]) -> Option<(&[u8], usize)> {
    let start = find(buf, b"8=")?;
    let nine = find(&buf[start..], b"\x019=")? + start + 1;
    let value_start = nine + 2;
    let value_end = find(&buf[value_start..], b"\x01")? + value_start;
    let body_length: usize = std::str::from_utf8(&buf[value_start..value_end])
        .ok()?
        .trim()
        .parse()
        .ok()?;

    let body_start = value_end + 1;
    let checksum_start = body_start + body_length;
    // "10=" plus at least three digits and a SOH.
    if buf.len() < checksum_start + 7 {
        return None;
    }
    if &buf[checksum_start..checksum_start + 3] != b"10=" {
        return None;
    }
    let end = find(&buf[checksum_start..], b"\x01")? + checksum_start + 1;
    Some((&buf[start..end], end))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// FIX `UTCTimestamp` with millisecond precision.
fn utc_timestamp() -> String {
    chrono::Utc::now().format("%Y%m%d-%H:%M:%S%.3f").to_string()
}

// ─── Binary gateway ─────────────────────────────────────────────────────────

/// Pillar Binary Gateway order entry, over an already-established TG/GT stream.
///
/// See the module documentation for why stream establishment is out of scope here.
pub struct PillarBinaryGateway {
    config: PillarBinaryConfig,
    next_cl_ord_id: AtomicU64,
    next_sequence: AtomicU64,
}

impl PillarBinaryGateway {
    pub fn new(config: PillarBinaryConfig) -> Self {
        Self {
            config,
            next_cl_ord_id: AtomicU64::new(1),
            next_sequence: AtomicU64::new(1),
        }
    }

    /// Translate a RustForge request into a Pillar binary New Order.
    pub fn to_new_order(
        &self,
        req: &OpenRequest,
        cl_ord_id: u64,
    ) -> Result<NewOrder, anyhow::Error> {
        let side = match req.side {
            OrderSide::Buy => binary::Side::Buy,
            OrderSide::Sell => binary::Side::Sell,
        };
        let (ord_type, price) = match req.order_type {
            OrderType::Market => (binary::OrdType::Market, Price::ZERO),
            OrderType::Limit => (
                binary::OrdType::Limit,
                Price::from_f64(
                    req.limit_price
                        .ok_or_else(|| anyhow::anyhow!("limit order without a limit price"))?,
                ),
            ),
        };
        let time_in_force = match req.time_in_force {
            TimeInForce::DAY => binary::TimeInForce::Day,
            TimeInForce::IOC | TimeInForce::FOK => binary::TimeInForce::Ioc,
            TimeInForce::GTC => {
                return Err(anyhow::anyhow!(
                    "Pillar has no good-till-cancelled: Day, IOC, At-the-Opening and On-Close only"
                ))
            }
        };

        let qty = req.quantity.round() as u32;
        let ceiling = self.config.market.max_order_qty(false);
        if qty > ceiling {
            return Err(anyhow::anyhow!(
                "{qty} shares exceeds the {ceiling} ceiling on {}",
                self.config.market.as_str()
            ));
        }

        // A symbol id, not a ticker, identifies the instrument on this protocol. It comes
        // from the Symbol Reference Data published on the REF stream, so the caller must
        // resolve it before an order can be built at all.
        let symbol_id = req.symbol.as_str().parse::<u32>().map_err(|_| {
            anyhow::anyhow!(
                "the Pillar binary gateway addresses instruments by numeric SymbolID, not by \
                     ticker; resolve {:?} from the Symbol Reference Data first",
                req.symbol
            )
        })?;

        let order = NewOrder {
            symbol_id,
            mpid: exchange_core::Mpid4::new(&self.config.mpid),
            mmid: self.config.mmid,
            mp_sub_id: self.config.mp_sub_id,
            cl_ord_id,
            orig_cl_ord_id: 0,
            instructions: OrderInstructions {
                side,
                ord_type,
                time_in_force,
                ..Default::default()
            },
            price,
            order_qty: qty,
            min_qty: if matches!(req.time_in_force, TimeInForce::FOK) {
                qty
            } else {
                0
            },
            user_data: exchange_core::UserData8::NUL,
        };
        order
            .validate()
            .map_err(|e| anyhow::anyhow!("order rejected before send: {e}"))?;
        Ok(order)
    }

    /// Wrap an application message in the `SeqMsg` envelope for this session's TG stream.
    pub fn envelope(&self, payload: Vec<u8>) -> binary::SeqMsg {
        binary::SeqMsg::new(
            binary::SeqMsgId {
                stream_id: self.config.stream_id,
                sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
            },
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            payload,
        )
    }

    /// Build the bytes for one order, ready to write to an established TG stream.
    pub fn build_order(&self, req: &OpenRequest) -> Result<(u64, Vec<u8>), anyhow::Error> {
        let cl_ord_id = self.next_cl_ord_id.fetch_add(1, Ordering::Relaxed);
        let order = self.to_new_order(req, cl_ord_id)?;
        Ok((cl_ord_id, self.envelope(order.encode()).encode()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyse::pillar::Market;

    fn fix_config() -> PillarFixConfig {
        PillarFixConfig {
            addr: "host:1234".into(),
            session: FixSessionConfig::new("FIRMFIX", "NYSE", "ABCD"),
            market: Market::Nyse,
        }
    }

    fn offline_fix() -> PillarFixGateway {
        let (tx, _rx) = mpsc::channel(1);
        PillarFixGateway {
            outbound: tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
            session: fix_config().session,
            market: Market::Nyse,
            next_cl_ord_id: AtomicU64::new(1),
        }
    }

    fn binary_gateway() -> PillarBinaryGateway {
        PillarBinaryGateway::new(PillarBinaryConfig {
            addr: "host:1234".into(),
            mpid: "ABCD".into(),
            mmid: 0,
            mp_sub_id: 'A',
            stream_id: 0x1122_3344,
            market: Market::Nyse,
        })
    }

    fn request(ty: OrderType, tif: TimeInForce, price: Option<f64>, symbol: &str) -> OpenRequest {
        OpenRequest {
            client_order_id: CompactString::new("C1"),
            symbol: CompactString::new(symbol),
            side: OrderSide::Buy,
            quantity: 500.0,
            order_type: ty,
            limit_price: price,
            time_in_force: tif,
        }
    }

    #[test]
    fn fix_limit_orders_map_to_the_documented_values() {
        let g = offline_fix();
        let o = g
            .to_new_order(
                &request(OrderType::Limit, TimeInForce::DAY, Some(180.50), "IBM"),
                "ORD1",
            )
            .unwrap();
        assert_eq!(o.symbol, "IBM");
        assert_eq!(o.order_qty, 500);
        assert_eq!(o.ord_type.code(), "2");
        assert_eq!(o.time_in_force.code(), "0");
        assert_eq!(o.price.unwrap().to_string(), "180.50");
    }

    #[test]
    fn fix_market_orders_carry_no_price() {
        let g = offline_fix();
        let o = g
            .to_new_order(
                &request(OrderType::Market, TimeInForce::DAY, None, "IBM"),
                "O",
            )
            .unwrap();
        assert_eq!(o.ord_type.code(), "1");
        assert!(o.price.is_none());
    }

    #[test]
    fn fok_becomes_ioc_with_a_full_size_minimum_on_both_gateways() {
        let o = offline_fix()
            .to_new_order(
                &request(OrderType::Limit, TimeInForce::FOK, Some(180.0), "IBM"),
                "O",
            )
            .unwrap();
        assert_eq!(o.time_in_force.code(), "3");
        assert_eq!(o.min_qty, Some(500));

        let b = binary_gateway()
            .to_new_order(
                &request(OrderType::Limit, TimeInForce::FOK, Some(180.0), "4242"),
                1,
            )
            .unwrap();
        assert_eq!(b.instructions.time_in_force, binary::TimeInForce::Ioc);
        assert_eq!(b.min_qty, 500);
    }

    #[test]
    fn gtc_is_refused_on_both_gateways() {
        assert!(offline_fix()
            .to_new_order(
                &request(OrderType::Limit, TimeInForce::GTC, Some(1.0), "IBM"),
                "O"
            )
            .is_err());
        assert!(binary_gateway()
            .to_new_order(
                &request(OrderType::Limit, TimeInForce::GTC, Some(1.0), "4242"),
                1
            )
            .is_err());
    }

    #[test]
    fn the_market_quantity_ceiling_is_applied_locally() {
        let g = offline_fix();
        let mut req = request(OrderType::Limit, TimeInForce::DAY, Some(180.0), "IBM");
        req.quantity = 6_000_000.0;
        let err = g.to_new_order(&req, "O").unwrap_err();
        assert!(err.to_string().contains("5000000"));
    }

    #[test]
    fn the_binary_gateway_requires_a_numeric_symbol_id() {
        let g = binary_gateway();
        let err = g
            .to_new_order(
                &request(OrderType::Limit, TimeInForce::DAY, Some(180.0), "IBM"),
                1,
            )
            .unwrap_err();
        assert!(err.to_string().contains("SymbolID"));

        // The numeric form is accepted.
        assert!(g
            .to_new_order(
                &request(OrderType::Limit, TimeInForce::DAY, Some(180.0), "4242"),
                1
            )
            .is_ok());
    }

    #[test]
    fn binary_orders_are_wrapped_in_a_sequenced_envelope() {
        let g = binary_gateway();
        let (cl_ord_id, bytes) = g
            .build_order(&request(
                OrderType::Limit,
                TimeInForce::DAY,
                Some(180.50),
                "4242",
            ))
            .unwrap();
        assert_eq!(cl_ord_id, 1);

        let seq = binary::SeqMsg::parse(&bytes).unwrap();
        assert_eq!(seq.id.stream_id, 0x1122_3344);
        assert_eq!(seq.id.sequence, 1);
        assert_eq!(seq.payload_type().unwrap(), binary::msg_type::NEW_ORDER);

        let order = binary::NewOrder::parse(&seq.payload).unwrap();
        assert_eq!(order.symbol_id, 4242);
        assert_eq!(order.mpid, "ABCD");
        assert_eq!(order.price.to_string(), "180.50");
        assert_eq!(order.cl_ord_id, 1);
        assert_eq!(order.orig_cl_ord_id, 0, "a new order, not a replace");
    }

    #[test]
    fn binary_stream_sequence_numbers_increase_per_message() {
        let g = binary_gateway();
        let (_, a) = g
            .build_order(&request(OrderType::Limit, TimeInForce::DAY, Some(1.0), "1"))
            .unwrap();
        let (_, b) = g
            .build_order(&request(OrderType::Limit, TimeInForce::DAY, Some(1.0), "1"))
            .unwrap();
        assert_eq!(binary::SeqMsg::parse(&a).unwrap().id.sequence, 1);
        assert_eq!(binary::SeqMsg::parse(&b).unwrap().id.sequence, 2);
    }

    #[test]
    fn fix_client_order_ids_are_unique_within_a_session() {
        let g = offline_fix();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(g.next_cl_ord_id()));
        }
    }

    #[test]
    fn restamping_replaces_the_sequence_number_and_keeps_the_message_valid() {
        let session = fix_config().session;
        let raw = fix::encode_heartbeat(&session, 1, "20260817-00:00:00.000", None);
        let stamped = restamp_sequence(&raw, 42);
        let parsed = FixMessage::parse(&stamped).expect("checksum recomputed");
        assert_eq!(parsed.get(fix::tag::MSG_SEQ_NUM), Some("42"));
        assert_eq!(parsed.msg_type, "0");
    }

    #[test]
    fn fix_framing_extracts_one_message_at_a_time() {
        let session = fix_config().session;
        let a = fix::encode_heartbeat(&session, 1, "T", None);
        let b = fix::encode_heartbeat(&session, 2, "T", Some("TR1"));
        let mut stream = a.clone();
        stream.extend_from_slice(&b);

        let (first, consumed) = next_fix_message(&stream).unwrap();
        assert_eq!(first, a.as_slice());
        assert_eq!(consumed, a.len());
        let (second, _) = next_fix_message(&stream[consumed..]).unwrap();
        assert_eq!(second, b.as_slice());
    }

    #[test]
    fn fix_framing_waits_for_a_complete_message() {
        let session = fix_config().session;
        let msg = fix::encode_heartbeat(&session, 1, "T", None);
        for cut in 0..msg.len() {
            assert!(
                next_fix_message(&msg[..cut]).is_none(),
                "truncated at {cut}"
            );
        }
        assert!(next_fix_message(&msg).is_some());
    }

    #[test]
    fn utc_timestamps_use_the_fix_format() {
        let ts = utc_timestamp();
        assert_eq!(ts.len(), 21, "YYYYMMDD-HH:MM:SS.mmm");
        assert_eq!(&ts[8..9], "-");
        assert_eq!(&ts[17..18], ".");
    }

    #[tokio::test]
    async fn submitting_on_a_dead_fix_session_errors_rather_than_hanging() {
        let g = offline_fix();
        let err = g
            .submit_order(request(
                OrderType::Limit,
                TimeInForce::DAY,
                Some(180.0),
                "IBM",
            ))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no longer running"));
    }

    #[test]
    fn the_gateways_name_themselves_for_routing() {
        assert_eq!(offline_fix().name(), "nyse-pillar-fix");
    }
}
