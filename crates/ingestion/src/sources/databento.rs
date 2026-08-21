//! Databento live market data.
//!
//! # Why this source exists
//!
//! `crates/nasdaq` and `crates/nyse` speak ITCH, OUCH, XDP and Pillar directly.
//! Running them needs exchange ports, a co-location cabinet and non-display
//! licences — a five-figure monthly commitment before a single message arrives.
//! Databento carries the same venues, normalised, over one TCP session, so the
//! decode-to-book-to-strategy path can be exercised against real L1/L2/L3
//! without any of that.
//!
//! The direct connectors remain the production path. This is the on-ramp.
//!
//! # What this deliberately does not do
//!
//! **No historical requests.** `HistoricalClient` bills per gigabyte against
//! the caller's account, and a market-data *source* that silently spends money
//! when a strategy restarts is a trap. Backfill belongs in an explicit,
//! operator-run tool where the cost is visible before it is incurred, not
//! behind a `connect()` that a supervisor may call in a reconnect loop.
//!
//! **No fallback.** A missing key, an unentitled dataset or a rejected session
//! ends the connection with the reason. Substituting mock data would make a
//! misconfigured deployment look like a quiet market.

use std::sync::Arc;

use async_trait::async_trait;
use common::events::{Envelope, MarketEvent, QuoteEvent, TradeEvent, TradeSide};
use common::time::{SequenceGenerator, UnixNanos};
use compact_str::CompactString;
use databento::dbn::{Mbp1Msg, PitSymbolMap, SType, Schema, SymbolIndex, TradeMsg};
use databento::live::Subscription as DbSubscription;
use databento::LiveClient;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};

use crate::source::{DataType, IngestionError, MarketDataSource, MarketStream, Subscription};

/// Environment variable holding the Databento API key.
const KEY_ENV: &str = "DATABENTO_API_KEY";
/// Which dataset to subscribe to, e.g. `XNAS.ITCH`, `GLBX.MDP3`, `EQUS.MINI`.
const DATASET_ENV: &str = "DATABENTO_DATASET";

/// DBN prices are fixed-point with 9 implied decimals.
///
/// The same convention `exchange_core::Price` uses, which is not a
/// coincidence — both exist so a venue's tick arithmetic stays exact. The
/// conversion to `f64` happens here only because `MarketEvent` is defined in
/// `f64`; it is the lossy boundary, and it is deliberately in one place.
const PRICE_SCALE: f64 = 1e-9;

/// A price field that carries no value.
///
/// DBN uses `i64::MAX` as its undefined sentinel rather than zero, because
/// zero is a legitimate price. Treating the sentinel as a number would publish
/// a quote at roughly 9.2 billion dollars.
const UNDEF_PRICE: i64 = i64::MAX;

pub struct DatabentoSource {
    seq_gen: Arc<SequenceGenerator>,
    api_key: String,
    dataset: String,
}

/// Hand-written so the key cannot be printed.
///
/// `#[derive(Debug)]` would put the API key into every log line, panic message
/// and error report that formats this struct — and a key in a log is a key that
/// has to be rotated. The dataset is safe to show and is the field anyone
/// debugging actually wants.
impl std::fmt::Debug for DatabentoSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabentoSource")
            .field("dataset", &self.dataset)
            .field("api_key", &"<redacted>")
            // Non-exhaustive rather than listing `seq_gen`: the generator's
            // internal counter is noise in a debug line, and the point of this
            // impl is to control what appears, not to mirror the struct.
            .finish_non_exhaustive()
    }
}

impl DatabentoSource {
    /// Build from the environment, naming whatever is missing.
    ///
    /// Neither value can be guessed. A default dataset would silently bill
    /// against, or fail on, a venue the operator never chose.
    pub fn from_env(seq_gen: Arc<SequenceGenerator>) -> Result<Self, IngestionError> {
        let api_key = std::env::var(KEY_ENV)
            .map_err(|_| IngestionError::ConnectionFailed(format!("{KEY_ENV} not set")))?;
        let dataset = std::env::var(DATASET_ENV).map_err(|_| {
            IngestionError::ConnectionFailed(format!(
                "{DATASET_ENV} not set (e.g. XNAS.ITCH, XNYS.PILLAR, GLBX.MDP3, EQUS.MINI)"
            ))
        })?;

        Ok(Self {
            seq_gen,
            api_key,
            dataset,
        })
    }

    pub fn dataset(&self) -> &str {
        &self.dataset
    }
}

/// The schema that serves a set of requested data types.
///
/// One subscription per schema, so asking for trades and quotes together does
/// not silently drop one. `Mbp1` carries the top of book *and* the trade that
/// caused it, which is why quotes and L1 map to the same schema — the same
/// reason Binance maps both to `@bookTicker`.
fn schemas_for(types: &[DataType]) -> Vec<Schema> {
    let mut out = Vec::new();
    if types.contains(&DataType::Trades) {
        out.push(Schema::Trades);
    }
    if types.contains(&DataType::Quotes) || types.contains(&DataType::OrderBookL1) {
        out.push(Schema::Mbp1);
    }
    if types.contains(&DataType::OrderBookL2) {
        out.push(Schema::Mbp10);
    }
    // Deduplicated for the same reason Binance's stream list is: a duplicate
    // subscription is bandwidth spent receiving what is already arriving.
    out.sort_unstable_by_key(|s| *s as u8);
    out.dedup();
    out
}

const SUPPORTED: &[DataType] = &[
    DataType::Trades,
    DataType::Quotes,
    DataType::OrderBookL1,
    DataType::OrderBookL2,
];

#[async_trait]
impl MarketDataSource for DatabentoSource {
    fn name(&self) -> &str {
        "Databento"
    }

    fn supported_data_types(&self) -> &[DataType] {
        SUPPORTED
    }

    async fn connect(&self, subscription: &Subscription) -> Result<MarketStream, IngestionError> {
        if subscription.symbols.is_empty() {
            return Err(IngestionError::ConnectionFailed(
                "no symbols requested".into(),
            ));
        }

        let schemas = schemas_for(&subscription.data_types);
        if schemas.is_empty() {
            return Err(IngestionError::ConnectionFailed(
                "no requested data type maps to a Databento schema".into(),
            ));
        }

        let mut client = LiveClient::builder()
            .key(&self.api_key)
            .map_err(|e| IngestionError::ConnectionFailed(e.to_string()))?
            .dataset(self.dataset.clone())
            .build()
            .await
            // Reported verbatim. An unentitled dataset and a bad key fail
            // differently, and collapsing them into "connection failed" costs
            // an operator the one clue that tells them which it is.
            .map_err(|e| {
                IngestionError::ConnectionFailed(format!(
                    "Databento live session for {}: {e}",
                    self.dataset
                ))
            })?;

        for schema in &schemas {
            client
                .subscribe(
                    DbSubscription::builder()
                        .symbols(subscription.symbols.clone())
                        .schema(*schema)
                        .stype_in(SType::RawSymbol)
                        .build(),
                )
                .await
                .map_err(|e| {
                    IngestionError::ConnectionFailed(format!("subscribe {schema:?}: {e}"))
                })?;
        }

        info!(
            target: "ingestion::databento",
            dataset = %self.dataset,
            symbols = subscription.symbols.len(),
            schemas = schemas.len(),
            "Databento live session established"
        );

        // `start` returns the session metadata. It is not retained: for a live
        // session the symbol map is built from the SymbolMappingMsg records
        // that follow, not from this, because mappings can change mid-session.
        client
            .start()
            .await
            .map_err(|e| IngestionError::ConnectionFailed(format!("start: {e}")))?;

        let (tx, rx) = mpsc::channel(4096);
        let seq_gen = Arc::clone(&self.seq_gen);
        let dataset = self.dataset.clone();

        tokio::spawn(async move {
            // Boxed: the future owns the session's read buffers, and this runs
            // once per session rather than per message.
            Box::pin(pump(client, seq_gen, tx, dataset)).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)) as MarketStream)
    }

    async fn is_healthy(&self) -> bool {
        // Deliberately not a probe. Opening a second live session to answer a
        // health check would consume an entitlement slot and tell us about a
        // connection nobody is reading from. The stream ending is the real
        // signal, and the multiplexer already surfaces that.
        !self.api_key.is_empty() && !self.dataset.is_empty()
    }
}

/// Read records until the session ends, publishing what maps to a market event.
async fn pump(
    mut client: LiveClient,
    seq_gen: Arc<SequenceGenerator>,
    tx: mpsc::Sender<Result<Envelope<MarketEvent>, IngestionError>>,
    dataset: String,
) {
    // Live sessions carry their symbol mappings in-band: a SymbolMappingMsg
    // arrives before the records it applies to, and mappings can be revised
    // mid-session. So the map is fed from the stream rather than snapshotted
    // from the start-up metadata.
    let mut symbol_map = PitSymbolMap::new();

    loop {
        let record = match client.next_record().await {
            Ok(Some(rec)) => rec,
            Ok(None) => {
                warn!(target: "ingestion::databento", %dataset, "live session closed by server");
                let _ = tx.send(Err(IngestionError::StreamClosed)).await;
                return;
            }
            Err(e) => {
                let _ = tx
                    .send(Err(IngestionError::ConnectionFailed(e.to_string())))
                    .await;
                return;
            }
        };

        let ts_init = UnixNanos::now();

        // Feed the mapping records in before anything tries to resolve against
        // them. An error here means a malformed mapping, which is worth seeing
        // rather than silently leaving the map stale.
        if let Err(e) = symbol_map.on_record(record) {
            warn!(target: "ingestion::databento", %dataset, error = %e, "symbol mapping rejected");
            continue;
        }

        // A record whose instrument id is not yet mapped is dropped rather than
        // published under a guessed name — the same rule the XDP handler
        // applies to a message that arrives before its price scale.
        let envelope = if let Some(trade) = record.get::<TradeMsg>() {
            symbol_map
                .get_for_rec(trade)
                .map(|s| CompactString::from(s.as_str()))
                .and_then(|symbol| {
                    let price = to_f64(trade.price)?;
                    Some(Envelope {
                        provenance: None,
                        ts_event: UnixNanos::from(trade.hd.ts_event),
                        ts_init,
                        sequence_id: seq_gen.next_id(),
                        payload: MarketEvent::Trade(TradeEvent {
                            symbol,
                            price,
                            quantity: f64::from(trade.size),
                            // DBN records the aggressor: 'B' means a buy order
                            // lifted the offer. Anything else is not a claim about
                            // direction, so it is reported as unknown rather than
                            // defaulted to one side.
                            // `side` is a C char (i8) in DBN, so it is widened
                            // before matching rather than each arm being cast.
                            side: match trade.side as u8 {
                                b'B' => TradeSide::Buy,
                                b'A' => TradeSide::Sell,
                                _ => TradeSide::Unknown,
                            },
                        }),
                    })
                })
        } else if let Some(mbp) = record.get::<Mbp1Msg>() {
            symbol_map
                .get_for_rec(mbp)
                .map(|s| CompactString::from(s.as_str()))
                .and_then(|symbol| {
                    let level = mbp.levels.first()?;
                    // A one-sided book is normal at an open or a halt. Publishing
                    // it with the missing side as 0.0 would look like a quote at
                    // zero, so the whole update is skipped instead.
                    let bid = to_f64(level.bid_px)?;
                    let ask = to_f64(level.ask_px)?;
                    Some(Envelope {
                        provenance: None,
                        ts_event: UnixNanos::from(mbp.hd.ts_event),
                        ts_init,
                        sequence_id: seq_gen.next_id(),
                        payload: MarketEvent::Quote(QuoteEvent {
                            symbol,
                            bid,
                            bid_size: f64::from(level.bid_sz),
                            ask,
                            ask_size: f64::from(level.ask_sz),
                        }),
                    })
                })
        } else {
            // System, symbol-mapping and error records carry no market event.
            None
        };

        if let Some(env) = envelope {
            if tx.send(Ok(env)).await.is_err() {
                // Consumer dropped; nothing left to publish to.
                return;
            }
        }
    }
}

/// Fixed-point DBN price to `f64`, or `None` for the undefined sentinel.
fn to_f64(px: i64) -> Option<f64> {
    if px == UNDEF_PRICE {
        None
    } else {
        Some(px as f64 * PRICE_SCALE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undefined_price_is_not_a_number() {
        // i64::MAX scaled would be ~9.2e9 dollars. Publishing that as a quote
        // is worse than publishing nothing.
        assert_eq!(to_f64(UNDEF_PRICE), None);
    }

    #[test]
    fn prices_use_dbn_fixed_point_scaling() {
        // 45.25 in DBN's 9-implied-decimal encoding.
        assert_eq!(to_f64(45_250_000_000), Some(45.25));
        // Zero is a real price, not a missing one.
        assert_eq!(to_f64(0), Some(0.0));
    }

    #[test]
    fn quotes_and_book_l1_share_one_schema_without_duplicating_it() {
        let s = schemas_for(&[DataType::Trades, DataType::Quotes, DataType::OrderBookL1]);
        assert_eq!(s.len(), 2, "expected trades + mbp1, got {s:?}");
        assert!(s.contains(&Schema::Trades));
        assert!(s.contains(&Schema::Mbp1));
    }

    #[test]
    fn an_unmappable_request_yields_no_schema() {
        // Bars have no live DBN schema here, so this must be reported rather
        // than silently connecting and delivering nothing.
        assert!(schemas_for(&[DataType::Bars1m]).is_empty());
    }

    #[test]
    fn l2_maps_to_mbp10() {
        assert_eq!(schemas_for(&[DataType::OrderBookL2]), vec![Schema::Mbp10]);
    }

    #[test]
    fn missing_configuration_names_the_variable() {
        // Fail-closed, and say which variable. "Connection failed" alone sends
        // an operator to the network when the problem is a missing env var.
        std::env::remove_var(KEY_ENV);
        let err = DatabentoSource::from_env(Arc::new(SequenceGenerator::new()))
            .expect_err("must refuse to build without a key");
        assert!(err.to_string().contains(KEY_ENV), "got: {err}");
    }
}
