//! Nasdaq TotalView-ITCH as a RustForge [`MarketDataSource`].
//!
//! Session shape, per transport:
//!
//! * **MoldUDP64** — join the multicast group, track sequence numbers, unicast a Request
//!   Packet to a re-request server on a gap. This is the co-location path and the only one
//!   the FPGA feed offers.
//! * **SoupBinTCP** — log in, consume Sequenced Data, and on socket failure reconnect with
//!   the session id and next expected sequence so the server resumes exactly where it
//!   stopped. Recovery is built into the session rather than bolted on.
//!
//! Starting mid-session is the case worth calling out. ITCH is a stream of *changes*; a
//! handler that joins at 11:00 has no book until every resting order happens to be touched.
//! The fix is GLIMPSE, which replays the current book state over SoupBinTCP. When a gap
//! cannot be recovered, this source ends the stream with an error instead of continuing to
//! publish from a book it knows is wrong.

use std::sync::Arc;

use async_trait::async_trait;
use common::events::{Envelope, MarketEvent};
use exchange_core::latency::now_monotonic_ns;
use ingestion::source::{DataType, IngestionError, MarketDataSource, MarketStream, Subscription};
use nasdaq::itch::{ItchFeedHandler, SessionClock};
use nasdaq::moldudp64::{MoldEvent, MoldReceiver};
use nasdaq::soupbintcp::{SoupEvent, SoupSession};
use tokio::sync::mpsc;

use crate::config::{ItchTransport, NasdaqConfig};
use crate::normalize::Normalizer;

/// Depth published on an L2 subscription.
const DEFAULT_DEPTH_LEVELS: usize = 10;

/// Bound on the channel between the session task and the consumer.
///
/// Sized for the burst at the open rather than for steady state: a shallow queue turns a
/// momentary consumer stall into dropped market data. It is bounded rather than unbounded
/// so that sustained back-pressure shows up as a slow session instead of unbounded memory.
const CHANNEL_CAPACITY: usize = 65_536;

/// A Nasdaq direct-feed market data source.
pub struct NasdaqItchSource {
    config: Arc<NasdaqConfig>,
    supported: Vec<DataType>,
}

impl NasdaqItchSource {
    pub fn new(config: NasdaqConfig) -> Self {
        Self {
            config: Arc::new(config),
            supported: vec![
                DataType::Trades,
                DataType::Quotes,
                DataType::OrderBookL1,
                DataType::OrderBookL2,
            ],
        }
    }

    /// The tickers to publish, or `None` for the whole tape.
    fn wanted_symbols(subscription: &Subscription) -> Option<Vec<String>> {
        (!subscription.symbols.is_empty()).then(|| subscription.symbols.clone())
    }
}

#[async_trait]
impl MarketDataSource for NasdaqItchSource {
    fn name(&self) -> &str {
        "nasdaq-itch"
    }

    fn supported_data_types(&self) -> &[DataType] {
        &self.supported
    }

    async fn connect(&self, subscription: &Subscription) -> Result<MarketStream, IngestionError> {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let config = Arc::clone(&self.config);
        let subscription = subscription.clone();

        match &config.itch {
            ItchTransport::MoldUdp64(mold) => {
                // Bind before returning the stream so a configuration or permission problem
                // surfaces as a connect error rather than as a silent, empty stream.
                let receiver = MoldReceiver::bind(mold.clone())
                    .await
                    .map_err(|e| IngestionError::ConnectionFailed(e.to_string()))?;
                tokio::spawn(async move {
                    // Boxed: the future owns the receiver's datagram buffers, so
                    // it is large enough that clippy flags it. This runs once per
                    // session, so the allocation is not on any hot path.
                    let err = Box::pin(run_mold(receiver, config, subscription, tx.clone())).await;
                    report_exit(err, &tx).await;
                });
            }
            ItchTransport::SoupBinTcp(soup) => {
                let session = SoupSession::connect(soup)
                    .await
                    .map_err(|e| IngestionError::ConnectionFailed(e.to_string()))?;
                tokio::spawn(async move {
                    // Boxed for the same reason as the MoldUDP64 arm above.
                    let err = Box::pin(run_soup(session, config, subscription, tx.clone())).await;
                    report_exit(err, &tx).await;
                });
            }
        }

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn is_healthy(&self) -> bool {
        // Health is a property of a live session, not of the source description. A source
        // that has not been connected reports unhealthy rather than claiming otherwise.
        false
    }
}

type EventSender = mpsc::Sender<Result<Envelope<MarketEvent>, IngestionError>>;

/// Push a session's terminal condition onto the stream so the consumer sees why it ended.
async fn report_exit(result: Result<(), String>, tx: &EventSender) {
    if let Err(reason) = result {
        tracing::error!(target: "exchange::nasdaq", %reason, "ITCH session ended");
        let _ = tx.send(Err(IngestionError::ConnectionFailed(reason))).await;
    }
}

/// State shared by both transports: the ITCH handler plus the downstream normaliser.
struct Pipeline {
    handler: ItchFeedHandler,
    normalizer: Normalizer,
    /// Subscribed tickers not yet resolved to locate codes.
    pending_symbols: Option<Vec<String>>,
}

impl Pipeline {
    fn new(config: &NasdaqConfig, subscription: &Subscription) -> Self {
        Self {
            handler: ItchFeedHandler::new(SessionClock::new(config.session_midnight_epoch_nanos)),
            normalizer: Normalizer::new(&subscription.data_types, DEFAULT_DEPTH_LEVELS),
            pending_symbols: NasdaqItchSource::wanted_symbols(subscription),
        }
    }

    /// Decode one ITCH message, update the books, and publish whatever it produced.
    ///
    /// A decode failure is logged and skipped rather than propagated: within a validated
    /// packet it means one message type this build does not understand, and killing the
    /// session over it would be worse than dropping it. A *framing* failure is caught one
    /// level up, where the whole packet is discarded and recovery is requested.
    ///
    /// `recv_ns` is the monotonic reading taken when the datagram arrived. It
    /// used to be hardcoded to 0 here, and `ItchFeedHandler` skips recording
    /// when it is 0 — so the latency histograms were fed nothing at the only
    /// call site that carries live traffic, while still being advertised as a
    /// feature. Passing the real timestamp is what makes them measure anything.
    async fn on_message(
        &mut self,
        raw: &[u8],
        recv_ns: u64,
        tx: &EventSender,
    ) -> Result<(), String> {
        let applied = match self.handler.on_message(raw, recv_ns) {
            Ok(Some(event)) => event,
            Ok(None) => {
                self.resolve_symbols();
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(target: "exchange::nasdaq", error = %e, "ITCH decode failed");
                return Ok(());
            }
        };

        let key = applied.key();
        let Some(book) = self.handler.books().get(key) else {
            return Ok(());
        };
        // ITCH carries the ticker inline on the messages that introduce an instrument, but
        // not on order-id-only ones, so fall back to the directory.
        let symbol = self.handler.directory().symbol(key as u16);
        for envelope in self.normalizer.on_event(&applied, book, symbol) {
            tx.send(Ok(envelope))
                .await
                .map_err(|_| "downstream consumer dropped the market data stream".to_string())?;
        }

        self.resolve_symbols();
        Ok(())
    }

    /// Once the Stock Directory spin has covered every subscribed ticker, narrow the handler
    /// to their locate codes so the rest of the tape is discarded before it is decoded.
    fn resolve_symbols(&mut self) {
        let Some(symbols) = self.pending_symbols.as_ref() else {
            return;
        };
        let locates: Vec<u16> = symbols
            .iter()
            .filter_map(|s| self.handler.directory().locate(s))
            .collect();
        if locates.len() == symbols.len() {
            tracing::info!(
                target: "exchange::nasdaq",
                count = locates.len(),
                "subscribed symbols resolved to locate codes; filtering the tape"
            );
            self.handler.watch_locates(locates);
            self.pending_symbols = None;
        }
    }
}

/// Drive a MoldUDP64 multicast session.
async fn run_mold(
    mut receiver: MoldReceiver,
    config: Arc<NasdaqConfig>,
    subscription: Subscription,
    tx: EventSender,
) -> Result<(), String> {
    let mut pipeline = Pipeline::new(&config, &subscription);

    loop {
        let event = receiver.recv().await.map_err(|e| e.to_string())?;
        // Stamped here, before any decoding, so the span covers this process's
        // whole share of the path rather than starting part way through it.
        let recv_ns = now_monotonic_ns();

        match event {
            MoldEvent::Data { skip, .. } => {
                // Copy the packet's messages out before touching the pipeline: the receive
                // buffer is reused on the next `recv`, and the borrow must not outlive it.
                let messages: Vec<Vec<u8>> = {
                    let packet = receiver.packet().map_err(|e| e.to_string())?;
                    // Strict validation here rather than the lenient iterator: a malformed
                    // packet must trigger recovery, not a partially applied book.
                    match packet.validate() {
                        Ok(msgs) => msgs
                            .into_iter()
                            .skip(skip as usize)
                            .map(<[u8]>::to_vec)
                            .collect(),
                        Err(e) => {
                            tracing::warn!(
                                target: "exchange::nasdaq",
                                error = %e,
                                "malformed MoldUDP64 packet dropped; awaiting retransmission"
                            );
                            Vec::new()
                        }
                    }
                };
                for raw in &messages {
                    // Every message in a packet carries the packet's arrival
                    // stamp: they did arrive together, and re-stamping each one
                    // would hide the cost of the messages decoded before it.
                    pipeline.on_message(raw, recv_ns, &tx).await?;
                }
            }
            MoldEvent::Duplicate | MoldEvent::Heartbeat => {}
            MoldEvent::SessionChanged { session } => {
                // Every sequence number and every book is meaningless across a session
                // change; continuing would publish stale state as live.
                return Err(format!(
                    "MoldUDP64 session changed to {session}; a fresh snapshot is required"
                ));
            }
            MoldEvent::EndOfSession => {
                tracing::info!(target: "exchange::nasdaq", "end of MoldUDP64 session");
                return Ok(());
            }
        }

        // Ask for anything missing. A gap with no re-request server configured is fatal:
        // continuing would mean serving a book known to be incomplete.
        receiver
            .request_retransmissions()
            .await
            .map_err(|e| e.to_string())?;
    }
}

/// Drive a SoupBinTCP session.
async fn run_soup(
    mut session: SoupSession,
    config: Arc<NasdaqConfig>,
    subscription: Subscription,
    tx: EventSender,
) -> Result<(), String> {
    let mut pipeline = Pipeline::new(&config, &subscription);

    loop {
        // Copy the payload out of the session buffer so its borrow ends before the next
        // call; Soup reuses the buffer across reads.
        // NOT boxed, deliberately. This awaits once per inbound message on
        // the steady-state read path, and `Box::pin` here would put a heap
        // allocation in front of every message on the tape. The future is
        // large because it borrows the session's read buffer, which is the
        // point — that buffer is reused rather than reallocated per read.
        #[allow(clippy::large_futures)]
        let event = session.next_event().await.map_err(|e| e.to_string())?;
        let recv_ns = now_monotonic_ns();
        let payload = match event {
            SoupEvent::Message { payload, .. } => Some(payload.to_vec()),
            SoupEvent::Heartbeat => None,
            SoupEvent::Debug(text) => {
                tracing::debug!(
                    target: "exchange::nasdaq",
                    text = %String::from_utf8_lossy(text),
                    "soup debug packet"
                );
                None
            }
            SoupEvent::EndOfSession => {
                tracing::info!(target: "exchange::nasdaq", "end of SoupBinTCP session");
                return Ok(());
            }
        };

        if let Some(raw) = payload {
            pipeline.on_message(&raw, recv_ns, &tx).await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::session_midnight_epoch_nanos;
    use nasdaq::itch::{encode, Header};
    use nasdaq::moldudp64::MoldConfig;

    fn config() -> NasdaqConfig {
        NasdaqConfig {
            itch: ItchTransport::MoldUdp64(MoldConfig::new("233.54.12.111:26477".parse().unwrap())),
            glimpse: None,
            ouch: None,
            session_midnight_epoch_nanos: session_midnight_epoch_nanos(),
        }
    }

    fn header(locate: u16, ts: u64) -> Header {
        Header {
            stock_locate: locate,
            tracking_number: 0,
            timestamp: ts,
        }
    }

    fn directory(locate: u16, sym: &str) -> Vec<u8> {
        encode::stock_directory(
            header(locate, 1),
            sym,
            'Q',
            'N',
            100,
            false,
            'C',
            "",
            'P',
            'N',
            'N',
            '1',
            'N',
            0,
            false,
        )
    }

    fn subscription(symbols: &[&str], types: &[DataType]) -> Subscription {
        Subscription {
            symbols: symbols.iter().map(|s| s.to_string()).collect(),
            data_types: types.to_vec(),
        }
    }

    #[test]
    fn the_source_advertises_only_what_itch_can_serve() {
        let s = NasdaqItchSource::new(config());
        assert_eq!(s.name(), "nasdaq-itch");
        for want in [
            DataType::Trades,
            DataType::Quotes,
            DataType::OrderBookL1,
            DataType::OrderBookL2,
        ] {
            assert!(s.supported_data_types().contains(&want));
        }
        // ITCH is an order feed, not a bar feed; claiming bars would be a lie.
        assert!(!s.supported_data_types().contains(&DataType::Bars1m));
    }

    #[test]
    fn an_empty_symbol_list_means_the_whole_tape() {
        assert!(
            NasdaqItchSource::wanted_symbols(&subscription(&[], &[DataType::Trades])).is_none()
        );
        assert_eq!(
            NasdaqItchSource::wanted_symbols(&subscription(&["AAPL"], &[DataType::Trades])),
            Some(vec!["AAPL".to_string()])
        );
    }

    #[test]
    fn symbols_are_narrowed_to_locates_only_once_all_are_known() {
        let mut p = Pipeline::new(
            &config(),
            &subscription(&["AAPL", "MSFT"], &[DataType::Trades]),
        );

        p.handler.on_message(&directory(1, "AAPL"), 0).unwrap();
        p.resolve_symbols();
        assert!(p.pending_symbols.is_some(), "still waiting for MSFT");

        p.handler.on_message(&directory(2, "MSFT"), 0).unwrap();
        p.resolve_symbols();
        assert!(p.pending_symbols.is_none(), "both resolved, filter applied");
    }

    #[tokio::test]
    async fn a_message_sequence_produces_normalised_events() {
        let (tx, mut rx) = mpsc::channel(64);
        let mut p = Pipeline::new(
            &config(),
            &subscription(&[], &[DataType::Trades, DataType::Quotes]),
        );

        let msgs = [
            directory(1, "AAPL"),
            encode::add_order(
                header(1, 2),
                100,
                'B',
                500,
                "AAPL",
                exchange_core::Price::from_price4(1_000_000),
                None,
            ),
            encode::add_order(
                header(1, 3),
                101,
                'S',
                300,
                "AAPL",
                exchange_core::Price::from_price4(1_000_200),
                None,
            ),
            encode::order_executed(header(1, 4), 101, 100, 900),
        ];
        for m in &msgs {
            p.on_message(m, 0, &tx).await.unwrap();
        }
        drop(tx);

        let mut quotes = 0;
        let mut trades = 0;
        while let Some(Ok(env)) = rx.recv().await {
            match env.payload {
                MarketEvent::Quote(q) => {
                    assert_eq!(q.symbol, "AAPL");
                    quotes += 1;
                }
                MarketEvent::Trade(t) => {
                    assert_eq!(t.symbol, "AAPL");
                    assert_eq!(t.quantity, 100.0);
                    trades += 1;
                }
                other => panic!("unexpected event {other:?}"),
            }
        }
        assert_eq!(trades, 1, "one printable execution");
        assert!(quotes >= 2, "the bid and the offer each moved the BBO");
    }

    #[tokio::test]
    async fn a_dropped_consumer_ends_the_session_rather_than_blocking_it() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let mut p = Pipeline::new(&config(), &subscription(&[], &[DataType::Quotes]));
        p.on_message(&directory(1, "AAPL"), 0, &tx).await.unwrap();
        let err = p
            .on_message(
                &encode::add_order(
                    header(1, 2),
                    1,
                    'B',
                    100,
                    "AAPL",
                    exchange_core::Price::from_price4(1_000_000),
                    None,
                ),
                0,
                &tx,
            )
            .await
            .unwrap_err();
        assert!(err.contains("consumer dropped"));
    }

    #[tokio::test]
    async fn an_undecodable_message_is_skipped_without_killing_the_session() {
        let (tx, _rx) = mpsc::channel(64);
        let mut p = Pipeline::new(&config(), &subscription(&[], &[DataType::Quotes]));
        assert!(p.on_message(&[b'z'; 19], 0, &tx).await.is_ok());
        assert_eq!(p.handler.stats().decode_errors, 1);
    }

    #[tokio::test]
    async fn an_unconnected_source_reports_unhealthy() {
        let s = NasdaqItchSource::new(config());
        assert!(!s.is_healthy().await, "health must reflect a live session");
    }
}
