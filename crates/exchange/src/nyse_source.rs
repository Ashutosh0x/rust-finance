//! NYSE Pillar Integrated Feed as a RustForge [`MarketDataSource`].
//!
//! The session runs one or two multicast lines through an arbitrating receiver, applies
//! packets to the XDP feed handler, and drives recovery through the Request Server:
//!
//! ```text
//!   line A ─┐
//!           ├─ arbitrate (shared sequence tracker) ─ handler ─ books ─ normalise ─ stream
//!   line B ─┘                    │
//!                                └─ gap ─ Request Server ── retransmission channel
//!                                                       └── refresh channel (snapshot)
//! ```
//!
//! Two failure modes get explicit handling rather than a log line:
//!
//! * **Prices before the symbol mapping.** XDP publishes bare numerators; the divisor is the
//!   per-symbol `PriceScaleCode` from the Symbol Index Mapping message. Until that arrives,
//!   messages for the symbol are counted and dropped, and a mapping request is sent — never
//!   decoded at a guessed scale.
//! * **Unrecoverable gaps.** After the configured number of failed retransmission attempts,
//!   the only correct recovery is a full refresh; if the Request Server is not configured,
//!   the session ends with an error instead of publishing a book with a hole in it.

use std::sync::Arc;

use async_trait::async_trait;
use common::events::{Envelope, MarketEvent};
use exchange_core::latency::now_monotonic_ns;
use ingestion::source::{DataType, IngestionError, MarketDataSource, MarketStream, Subscription};
use nyse::xdp::receiver::{ChannelEvent, ChannelReceiver};
use nyse::xdp::request_server::{RequestServerClient, RequestStatus};
use nyse::xdp::XdpFeedHandler;
use tokio::sync::mpsc;

use crate::config::NyseConfig;
use crate::normalize::Normalizer;

const DEFAULT_DEPTH_LEVELS: usize = 10;
const CHANNEL_CAPACITY: usize = 65_536;

/// A NYSE Integrated Feed market data source.
pub struct NyseXdpSource {
    config: Arc<NyseConfig>,
    supported: Vec<DataType>,
}

impl NyseXdpSource {
    pub fn new(config: NyseConfig) -> Self {
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
}

#[async_trait]
impl MarketDataSource for NyseXdpSource {
    fn name(&self) -> &str {
        "nyse-xdp-integrated"
    }

    fn supported_data_types(&self) -> &[DataType] {
        &self.supported
    }

    async fn connect(&self, subscription: &Subscription) -> Result<MarketStream, IngestionError> {
        let channel = self.config.channels.first().cloned().ok_or_else(|| {
            IngestionError::ConnectionFailed(
                "no XDP channel configured; NYSE issues the product and channel ids".into(),
            )
        })?;

        let receiver = ChannelReceiver::bind(channel)
            .await
            .map_err(|e| IngestionError::ConnectionFailed(e.to_string()))?;

        // Connect the Request Server up front. Discovering at the first gap that recovery is
        // unreachable is discovering it too late.
        let request_client = if let Some(cfg) = &self.config.request_server {
            Some(
                RequestServerClient::connect(cfg.clone())
                    .await
                    .map_err(|e| IngestionError::ConnectionFailed(e.to_string()))?,
            )
        } else {
            tracing::warn!(
                target: "exchange::nyse",
                "no Request Server configured; a sequence gap will end the session"
            );
            None
        };

        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let subscription = subscription.clone();
        tokio::spawn(async move {
            // Boxed: the future owns the A/B line datagram buffers. One
            // allocation per session, never per message.
            let result = Box::pin(run(receiver, request_client, subscription, tx.clone())).await;
            if let Err(reason) = result {
                tracing::error!(target: "exchange::nyse", %reason, "XDP session ended");
                let _ = tx.send(Err(IngestionError::ConnectionFailed(reason))).await;
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn is_healthy(&self) -> bool {
        false
    }
}

type EventSender = mpsc::Sender<Result<Envelope<MarketEvent>, IngestionError>>;

async fn run(
    mut receiver: ChannelReceiver,
    mut request_client: Option<RequestServerClient>,
    subscription: Subscription,
    tx: EventSender,
) -> Result<(), String> {
    let mut handler = XdpFeedHandler::new();
    let mut normalizer = Normalizer::new(&subscription.data_types, DEFAULT_DEPTH_LEVELS);
    let wanted: Vec<String> = subscription.symbols.clone();
    let mut symbols_narrowed = wanted.is_empty();
    let mut mapping_requested = false;

    loop {
        let event = receiver.recv().await.map_err(|e| e.to_string())?;
        // Stamped before any decoding, so the span covers this process's whole share of
        // the path rather than starting part way through it.
        let recv_ns = now_monotonic_ns();

        match event {
            ChannelEvent::Data { skip, .. } => {
                let datagram = receiver.datagram().to_vec();
                if let Err(e) = handler.on_packet(&datagram, skip, recv_ns) {
                    tracing::warn!(target: "exchange::nyse", error = %e, "XDP packet decode failed");
                }
                publish(&mut handler, &mut normalizer, &tx).await?;
            }

            // Recovery traffic is applied the same way, but it does not advance the live
            // watermark — the receiver already knows not to move it.
            ChannelEvent::Recovery {
                sequence, count, ..
            } => {
                let datagram = receiver.datagram().to_vec();
                if let Err(e) = handler.on_packet(&datagram, 0, recv_ns) {
                    tracing::warn!(target: "exchange::nyse", error = %e, "XDP recovery packet decode failed");
                } else {
                    receiver.note_recovered(sequence, sequence + count.max(1) as u32 - 1);
                }
                publish(&mut handler, &mut normalizer, &tx).await?;

                // A refresh restates the book as of a known sequence; resume the live
                // channel there rather than wherever it happens to be.
                if !handler.in_refresh() {
                    if let Some(resume) = handler.refresh_resume_sequence() {
                        receiver.resume_at(resume);
                    }
                }
            }

            ChannelEvent::SequenceReset { sequence, .. } => {
                tracing::warn!(
                    target: "exchange::nyse",
                    sequence,
                    "publisher reset the sequence; discarding channel state"
                );
                handler.clear_events();
            }

            ChannelEvent::Duplicate { .. } | ChannelEvent::Heartbeat { .. } => {}
        }

        // Ask for the symbol mapping once, as soon as it is clear it is needed: without it
        // prices cannot be decoded at all, so this is not an optional optimisation.
        if !mapping_requested && handler.stats().awaiting_symbol_mapping > 0 {
            mapping_requested = true;
            match request_client.as_mut() {
                Some(client) => {
                    let response = client
                        .request_symbol_index_mapping(0)
                        .await
                        .map_err(|e| e.to_string())?;
                    if !response.status.is_accepted() {
                        return Err(format!(
                            "symbol index mapping request rejected: {}",
                            response.status
                        ));
                    }
                }
                None => {
                    return Err(
                        "messages arrived before the symbol index mapping and no Request Server \
                         is configured; prices cannot be decoded"
                            .into(),
                    )
                }
            }
        }

        if !symbols_narrowed {
            let indexes: Vec<u32> = wanted
                .iter()
                .filter_map(|s| handler.directory().index(s))
                .collect();
            if indexes.len() == wanted.len() {
                handler.watch_indexes(indexes);
                symbols_narrowed = true;
            }
        }

        recover(&mut receiver, request_client.as_mut()).await?;
    }
}

/// Publish everything the handler applied since the last call.
async fn publish(
    handler: &mut XdpFeedHandler,
    normalizer: &mut Normalizer,
    tx: &EventSender,
) -> Result<(), String> {
    for event in handler.drain_events() {
        let key = event.key();
        let Some(book) = handler.books().get(key) else {
            continue;
        };
        // XDP data messages carry no ticker, so the symbol always comes from the directory.
        let symbol = handler.directory().symbol(key);
        for envelope in normalizer.on_event(&event, book, symbol) {
            tx.send(Ok(envelope))
                .await
                .map_err(|_| "downstream consumer dropped the market data stream".to_string())?;
        }
    }
    Ok(())
}

/// Send retransmission and refresh requests for outstanding gaps.
async fn recover(
    receiver: &mut ChannelReceiver,
    client: Option<&mut RequestServerClient>,
) -> Result<(), String> {
    let recovery = receiver.pending_recovery();
    if recovery.is_empty() {
        return Ok(());
    }

    let Some(client) = client else {
        return Err(format!(
            "sequence gap of {} range(s) with no Request Server configured; the book cannot \
             be trusted",
            recovery.retransmit.len() + recovery.refresh_required.len()
        ));
    };

    for range in &recovery.retransmit {
        let response = client
            .request_retransmission(range.from as u32, range.to as u32)
            .await
            .map_err(|e| e.to_string())?;
        match response.status {
            RequestStatus::Accepted => {}
            // A range the server considers too wide is a client-side sizing bug; splitting
            // further is the fix, and the tracker already caps requests, so surface it.
            status if status.is_permanent() => {
                return Err(format!("retransmission permanently rejected: {status}"))
            }
            status => {
                tracing::warn!(target: "exchange::nyse", %status, "retransmission rejected, will retry")
            }
        }
    }

    // Ranges that survived the retry budget need a snapshot, not another retransmission.
    if !recovery.refresh_required.is_empty() {
        tracing::warn!(
            target: "exchange::nyse",
            ranges = recovery.refresh_required.len(),
            "escalating to a full refresh"
        );
        let response = client.request_refresh(0).await.map_err(|e| e.to_string())?;
        if !response.status.is_accepted() {
            return Err(format!("refresh request rejected: {}", response.status));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyse::xdp::receiver::{ChannelConfig, FeedLine};
    use nyse::xdp::request_server::RequestServerConfig;

    fn config() -> NyseConfig {
        NyseConfig {
            channels: vec![ChannelConfig::new(
                7,
                1,
                FeedLine::new("224.0.59.1:11111".parse().unwrap()),
            )
            .with_line_b(FeedLine::new("224.0.59.2:11111".parse().unwrap()))],
            request_server: Some(RequestServerConfig::new("host:1234", "RFORGE", 7, 1)),
            order_entry: None,
        }
    }

    #[test]
    fn the_source_advertises_what_the_integrated_feed_can_serve() {
        let s = NyseXdpSource::new(config());
        assert_eq!(s.name(), "nyse-xdp-integrated");
        assert!(s.supported_data_types().contains(&DataType::OrderBookL2));
        assert!(!s.supported_data_types().contains(&DataType::Bars1m));
    }

    #[tokio::test]
    async fn connecting_without_a_channel_fails_rather_than_returning_an_empty_stream() {
        let s = NyseXdpSource::new(NyseConfig {
            channels: vec![],
            request_server: None,
            order_entry: None,
        });
        let result = s
            .connect(&Subscription {
                symbols: vec![],
                data_types: vec![DataType::Trades],
            })
            .await;
        // `MarketStream` is a boxed trait object, so it has no `Debug` to unwrap through.
        match result {
            Err(IngestionError::ConnectionFailed(reason)) => {
                assert!(reason.contains("no XDP channel configured"), "{reason}")
            }
            Err(other) => panic!("expected a connection failure, got {other}"),
            Ok(_) => panic!("connecting without a channel must not yield a stream"),
        }
    }

    #[tokio::test]
    async fn an_unconnected_source_reports_unhealthy() {
        assert!(!NyseXdpSource::new(config()).is_healthy().await);
    }

    #[tokio::test]
    async fn publish_emits_normalised_events_from_applied_packets() {
        use nyse::xdp::common::{encode_symbol_index_mapping, SymbolIndexMapping};
        use nyse::xdp::{integrated, packet};

        let mut handler = XdpFeedHandler::new();
        let mut normalizer = Normalizer::new(&[DataType::Quotes, DataType::Trades], 5);
        let (tx, mut rx) = mpsc::channel(64);

        let mapping = SymbolIndexMapping {
            symbol_index: 4242,
            symbol: "IBM".into(),
            market_id: 1,
            system_id: 1,
            exchange_code: 'N',
            price_scale_code: 4,
            security_type: 'C',
            lot_size: 100,
            prev_close_price: exchange_core::Price::ZERO,
            prev_close_volume: 0,
            price_resolution: 0,
            round_lot: 'Y',
            mpv: 1,
            unit_of_trade: 100,
        };
        let mapping_bytes = encode_symbol_index_mapping(&mapping);
        let add = integrated::AddOrder {
            source_time_nanos: 1,
            symbol_index: 4242,
            symbol_seq_num: 1,
            order_id: 1,
            price_raw: 1_805_000,
            volume: 500,
            side: integrated::Side::Buy,
            firm_id: exchange_core::FirmId5::NUL,
        }
        .encode();

        let pkt = packet::encode_packet(
            packet::delivery_flag::ORIGINAL,
            1,
            1_700_000_000,
            0,
            &[&mapping_bytes, &add],
        );
        handler.on_packet(&pkt, 0, 0).unwrap();
        publish(&mut handler, &mut normalizer, &tx).await.unwrap();
        drop(tx);

        let env = rx.recv().await.expect("a quote").unwrap();
        let MarketEvent::Quote(q) = env.payload else {
            panic!("expected a quote")
        };
        assert_eq!(q.symbol, "IBM", "symbol resolved from the directory");
        assert_eq!(q.bid, 180.50);
        assert_eq!(q.bid_size, 500.0);
        assert!(q.ask.is_nan());
    }

    #[tokio::test]
    async fn publish_reports_a_dropped_consumer() {
        use nyse::xdp::common::{encode_symbol_index_mapping, SymbolIndexMapping};
        use nyse::xdp::{integrated, packet};

        let mut handler = XdpFeedHandler::new();
        handler.seed_directory([SymbolIndexMapping {
            symbol_index: 4242,
            symbol: "IBM".into(),
            market_id: 1,
            system_id: 1,
            exchange_code: 'N',
            price_scale_code: 4,
            security_type: 'C',
            lot_size: 100,
            prev_close_price: exchange_core::Price::ZERO,
            prev_close_volume: 0,
            price_resolution: 0,
            round_lot: 'Y',
            mpv: 1,
            unit_of_trade: 100,
        }]);
        let _ = encode_symbol_index_mapping;

        let add = integrated::AddOrder {
            source_time_nanos: 1,
            symbol_index: 4242,
            symbol_seq_num: 1,
            order_id: 1,
            price_raw: 1_805_000,
            volume: 500,
            side: integrated::Side::Buy,
            firm_id: exchange_core::FirmId5::NUL,
        }
        .encode();
        let pkt = packet::encode_packet(packet::delivery_flag::ORIGINAL, 1, 0, 0, &[&add]);
        handler.on_packet(&pkt, 0, 0).unwrap();

        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let mut normalizer = Normalizer::new(&[DataType::Quotes], 5);
        let err = publish(&mut handler, &mut normalizer, &tx)
            .await
            .unwrap_err();
        assert!(err.contains("consumer dropped"));
    }
}
