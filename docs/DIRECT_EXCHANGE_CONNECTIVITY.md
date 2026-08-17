# Direct Exchange Connectivity — Nasdaq and NYSE

Native, co-location-grade connectivity to the two US primary listing venues: their proprietary
market-data feeds and their native order-entry gateways. No vendor SDK, no REST polling, no
intermediary — the exchange's own binary protocols, decoded from the specification.

| | Nasdaq | NYSE (ICE Pillar) |
|---|---|---|
| **Market data** | TotalView-ITCH 5.0 | XDP Integrated Feed |
| **Data transport** | MoldUDP64 (multicast) / SoupBinTCP (TCP) | UDP multicast, A/B lines |
| **Recovery** | Re-request server, GLIMPSE snapshot | Pillar Request Server (retransmit + refresh) |
| **Order entry** | OUCH 4.2 over SoupBinTCP | Pillar Binary Gateway, Pillar FIX 4.2 |
| **Data centre** | Carteret, NJ | Mahwah, NJ |
| **Byte order** | Big endian | Little endian |
| **String padding** | Space | NUL |
| **Price encoding** | 4 implied decimals (8 for MWCB) | Per-symbol `PriceScaleCode` (feed), scale 8 (gateway) |

Those last three rows are why this is four crates and not one file. The two venues agree on
almost nothing at the byte level, and a shared abstraction that papers over the difference is
exactly where the bugs live.

---

## Crate layout

```
exchange-core   Wire primitives, fixed-point Price, L3 order book, gap tracker, latency histogram
   ├─ nasdaq    ITCH 5.0 · OUCH 4.2 · SoupBinTCP 3.00 · MoldUDP64
   ├─ nyse      XDP packet/common/Integrated · Request Server · Pillar Binary · Pillar FIX 4.2
   └─ exchange  MarketDataSource + ExecutionGateway adapters, config, preflight, replay
```

`exchange-core` has one dependency (`thiserror`), forbids `unsafe`, and allocates nothing on the
decode path. Both venue crates decode their own wire format into its shared types, so a single
order-book implementation and a single gap tracker serve both.

**294 unit tests** across the four crates, all asserting against the published specifications.

---

## Design decisions worth knowing

### Prices are exact integers, never floats

Every venue publishes prices as scaled integers at a different scale. Converting to `f64` at the
decoder loses cents on large notionals and makes tick arithmetic non-associative. Instead
everything normalises to one signed integer scale of 1e-9 USD:

```rust
Price::from_price4(1_234_500)        // ITCH/OUCH, 4 implied decimals → $123.45
Price::from_xdp(1_805_000, 4)        // XDP numerator + PriceScaleCode  → $180.50
Price::from_pillar(123_000_000)      // Pillar gateway, scale 8         → $1.23
```

That range holds the widest venue maximum ($999,999.999999 on Pillar) with three orders of
magnitude of headroom in an `i64`, and every conversion is an exact integer multiply. `f64` is
produced only at the boundary where the rest of the system requires it.

The single most likely integration bug this prevents: the Pillar gateway uses price scale **8**
while ITCH uses **4**. Sending an order at the wrong scale enters it at 1/10,000th of the
intended price.

### Message length is read from the wire, differently per venue

The two specifications give opposite instructions, and both are obeyed:

* **ITCH** message lengths are fixed per type. A length that disagrees with the specification
  means the session framing itself is broken, so the decoder **rejects** it.
* **XDP** messages start with their own `MsgSize`, and NYSE explicitly reserves the right to
  append fields to existing types. A handler that walks packets by a compiled-in constant
  desynchronises mid-packet on release day, so the decoder **advances by the declared size** and
  only checks a documented *minimum*.

There is a test for exactly that: a message type grown by four bytes still leaves the next
message in the packet findable.

### A gap is never silently skipped

Both feeds sequence per channel and expect the client to notice loss itself. The shared
`SequenceTracker` classifies every packet as in-order, duplicate, partially overlapping, or a
gap, and splits wide gaps into requests no larger than the venue's published limit (NYSE rejects
requests over 10,000 messages with `Status = 3`).

Two specific refusals:

* **Reset is never inferred.** "Sequence 1 while we are at 900" is indistinguishable from a stale
  duplicate. MoldUDP64 signals a real restart by changing the 10-byte session id; XDP sends a
  Sequence Number Reset message. Each transport calls `observe_reset` on its own real signal.
* **A gap with no recovery path ends the session.** If no re-request server or Request Server is
  configured, the connector returns an error rather than continuing to publish a book it knows
  has a hole in it.

### Heartbeats are gap evidence

MoldUDP64 heartbeats and XDP heartbeats both carry the *next expected* sequence number. During a
quiet period that is the only way a tail-end loss is discovered at all, so heartbeats feed the
tracker rather than being discarded as keepalives.

### A/B line arbitration is duplicate handling

NYSE publishes each channel on two multicast groups with identical, identically sequenced content
over separate network paths. Both lines share one sequence tracker, so whichever copy arrives
first is processed and the second is classified as a duplicate. `ChannelStats` reports the
first-arrival split between the lines; a lopsided split means one path is consistently slower.

### Order tokens are the client's problem

OUCH identifies an order by a 14-byte `Order Token` the *client* assigns, and a token that has
already been used is **silently ignored** — no reject, no error. That is what makes recovery
work: resending an in-flight order after a socket failure is the documented, safe response. It is
also what makes `TokenAllocator` load-bearing rather than a convenience, because a collision
produces an order that vanishes without a trace.

### The book refuses to guess

An event referencing an unknown order id means a dropped packet. The book reports
`UnknownOrder` and does **not** mutate; the only correct response is to recover state. Levels are
deleted the instant their quantity reaches zero, so an empty level can never quote a phantom
price.

---

## Configuration

Every value below is issued by an exchange under a subscription or connectivity agreement. None
of them can be guessed, and `VenueConfig::from_env` names the missing variable rather than
substituting a default.

### Nasdaq

| Variable | Meaning |
|---|---|
| `NASDAQ_ITCH_MOLD_GROUP` | MoldUDP64 multicast `host:port` (co-location path) |
| `NASDAQ_ITCH_INTERFACE` | Local IPv4 of the feed NIC — set it explicitly on a multi-homed host |
| `NASDAQ_MOLD_REREQUEST_SERVERS` | Comma-separated `host:port` list for retransmission |
| `NASDAQ_ITCH_SOUP_ADDR` / `_USER` / `_PASS` | SoupBinTCP alternative to multicast |
| `NASDAQ_GLIMPSE_ADDR` / `_USER` / `_PASS` | GLIMPSE snapshot service, for a mid-session start |
| `NASDAQ_OUCH_ADDR` / `_USER` / `_PASS` | OUCH order entry (opt-in; absent means market data only) |
| `NASDAQ_OUCH_FIRM` | 4-byte firm identifier; blank uses the account default |
| `NASDAQ_OUCH_TOKEN_PREFIX` | Short prefix for generated order tokens |

### NYSE

| Variable | Meaning |
|---|---|
| `NYSE_XDP_PRODUCT_ID` | Feed identifier from the product's client specification |
| `NYSE_XDP_CHANNEL_ID` | Multicast channel number within the product |
| `NYSE_XDP_GROUP_A` | Primary multicast `host:port` |
| `NYSE_XDP_GROUP_B` | Redundant line — omitting it means every loss becomes a round trip |
| `NYSE_XDP_INTERFACE` | Local IPv4 of the feed NIC |
| `NYSE_REQUEST_SERVER_ADDR` | Pillar Request Server `host:port` |
| `NYSE_REQUEST_SERVER_SOURCE_ID` | Client identifier, at most 10 characters |

Order entry is opt-in on top of market data, so a market-data-only deployment cannot accidentally
acquire the ability to trade.

---

## Usage

### Market data

```rust
use exchange::{VenueConfig, market_data_sources};
use ingestion::source::{DataType, Subscription};

let config = VenueConfig::from_env()?;
let sources = market_data_sources(&config)?;   // validates before opening a socket

let subscription = Subscription {
    symbols: vec!["AAPL".into(), "IBM".into()],  // empty = the whole tape
    data_types: vec![DataType::Trades, DataType::Quotes, DataType::OrderBookL2],
};

for source in &sources {
    let mut stream = source.connect(&subscription).await?;
    // …yields Envelope<MarketEvent> exactly like every other RustForge source
}
```

Subscribing to specific symbols matters: on the full Nasdaq tape a twenty-symbol strategy
discards over 99% of messages, and the filter runs on the stock locate code read at a fixed
offset *before* any field is decoded.

### Order entry

```rust
use exchange::OuchGateway;
use execution::gateway::{ExecutionGateway, OpenRequest, TimeInForce};

let gateway = OuchGateway::connect(ouch_config).await?;
let event = gateway.submit_order(OpenRequest {
    client_order_id: "C1".into(),
    symbol: "AAPL".into(),
    side: OrderSide::Buy,
    quantity: 500.0,
    order_type: OrderType::Limit,
    limit_price: Some(123.45),
    time_in_force: TimeInForce::DAY,
}).await?;
```

`submit_order` waits for the exchange's response, not for the write to succeed — OUCH inbound
messages are explicitly not guaranteed, so a successful write tells you nothing.

Translation rules that are not obvious:

* OUCH has no order-type field. A **market order** is a limit at the sentinel price
  `$214,748.3647`.
* OUCH time-in-force is a lifetime in *seconds*, not an enum. `IOC` is `0`, `DAY` is the Market
  Hours sentinel, and **`GTC` is rejected** — a US equity order cannot outlive the session, so it
  is refused rather than quietly downgraded to a day order.
* **`FOK`** has no flag on either venue; it is IOC with a minimum quantity equal to the full size.
* The Pillar **binary** gateway addresses instruments by numeric `SymbolID`, not by ticker.
  Passing a ticker returns an error telling you to resolve it from the Symbol Reference Data.

### Preflight

```rust
use exchange::colo::Preflight;

let preflight = Preflight::default();          // co-location budget: 500µs RTT, 50µs p99 book
let report = preflight.local_checks(2, 1);     // 2 feeds, 1 order session
println!("{report}");
```

Every check reports what it actually measured. A check that cannot run here reports `Skipped`,
never `Pass` — an unmeasured check is unknown, and the socket-receive-buffer check says so
explicitly rather than pretending.

### Replay

```rust
use exchange::replay::{Replayer, ReplayFormat};

let mut replayer = Replayer::new(ReplayFormat::MoldUdp64, &[DataType::Quotes], 10, midnight_ns);
let events = replayer.replay_file("capture.mold")?;
let book = replayer.itch().unwrap().books().get(locate).unwrap();
```

Replay pushes recorded bytes through the same decoders, book builders and normalisers as the live
path — there is no parallel implementation to drift. Captures are length-prefixed datagrams
(4-byte big-endian length, then the datagram); that is a storage format, not a wire format.

---

## What this does not do

Stated plainly, because a connector that quietly produces plausible output is worse than one that
does not run:

* **No synthetic market data.** Nothing here fabricates prices, simulates a session, or fills in a
  book it did not receive. An unconfigured connector fails to start and names what it lacks.
* **Encoders are for tests and capture tooling only.** `itch::encode`, the XDP encoders and the
  OUCH/Pillar outbound encoders exist to round-trip byte layouts against the specification and to
  rewrite captures. They are never wired into a feed path.
* **The Pillar Binary Gateway stream handshake is out of scope.** The application layer is
  complete — New Order, Cancel, Order Ack, Execution Report, and the `BitfieldOrderInstructions`
  packing. Stream establishment lives in a separate document (the NYSE Pillar Stream Protocol
  specification) covering the unsequenced messages that open and position a TG/GT stream.
  Inventing that handshake would produce code that compiles, passes its own tests, and fails at
  certification, so `PillarBinaryGateway` takes an already-established stream instead.
* **Entitlements are real.** Both venues require a data agreement, assigned identifiers and
  network reach to Carteret or Mahwah. This code speaks the protocols correctly; it cannot grant
  access to them.

---

## Message coverage

**ITCH 5.0** — all 22 message types: `S` System Event, `R` Stock Directory, `H` Stock Trading
Action, `Y` Reg SHO, `L` Market Participant Position, `V`/`W` MWCB, `K` IPO Quoting Period, `J`
LULD Auction Collar, `h` Operational Halt, `A`/`F` Add Order, `E` Order Executed, `C` Order
Executed With Price, `X` Order Cancel, `D` Order Delete, `U` Order Replace, `P` Trade, `Q` Cross
Trade, `B` Broken Trade, `I` NOII, `N` RPII, `O` Direct Listing with Capital Raise.

**OUCH 4.2** — inbound: Enter, Replace, Cancel, Modify. Outbound: System Event, Accepted,
Replaced, Canceled, AIQ Canceled, Executed, Executed with Reference Price, Broken Trade, Rejected,
Cancel Pending, Cancel Reject, Priority Update, Order Modified — with the full cancel-reason,
reject-reason and liquidity-flag tables.

**XDP** — control: Sequence Number Reset (1), Source Time Reference (2), Symbol Index Mapping (3),
Retransmission Request (10), Request Response (11), Heartbeat Response (12), Symbol Index Mapping
Request (13), Refresh Request (15), Message Unavailable (31), Symbol Clear (32), Security Status
(34), Refresh Header (35). Integrated Feed: Add Order (100), Modify (101), Delete (102),
Execution (103), Replace (104), Imbalance (105), Add Order Refresh (106), Non-Displayed Trade
(110), Cross Trade (111), Trade Cancel (112), Cross Correction (113), RPI (114), Stock Summary
(223).

**Pillar Binary** — `MsgHeader`, `SeqMsgId`, `SeqMsg`, `BitfieldOrderInstructions` (all 17 packed
fields), New Order / Cancel-Replace, Order Cancel Request, Order Acknowledgement, Execution
Report.

**Pillar FIX 4.2** — Logon, Logout, Heartbeat, Test Request, Resend Request, New Order Single,
Order Cancel Request, Execution Report, Order Cancel Reject, plus the NYSE extension tags
(`9303` RoutingInst, `9416` ExtendedExecInst, `9483` DealID, `9730` LiquidityIndicator, `20004`
WorkingPrice, `20008` ParticipantType and the rest).

---

## Sources

Specifications this implementation was written against:

- [Nasdaq TotalView-ITCH 5.0](https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/NQTVITCHSpecification.pdf)
- [Nasdaq OUCH 4.2](https://www.nasdaqtrader.com/content/technicalsupport/specifications/tradingproducts/ouch4.2.pdf)
- [SoupBinTCP 3.00](https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/soupbintcp.pdf)
- [MoldUDP64 1.00](https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/moldudp64.pdf)
- [NYSE Pillar Common Client Specification](https://www.nyse.com/publicdocs/nyse/data/Pillar_Common_Client_Specification_v2.4d.pdf)
- [NYSE Integrated Feed Client Specification](https://www.nyse.com/publicdocs/nyse/data/Integrated_Feed_Client_Specification_v2.4a.pdf)
- [NYSE Pillar Gateway Binary Protocol Specification](https://www.nyse.com/publicdocs/nyse/NYSE_Pillar_Gateway_Binary_Protocol_Specification.pdf)
- [NYSE Pillar Gateway FIX Protocol Specification](https://www.nyse.com/publicdocs/nyse/NYSE_Pillar_Gateway_FIX_Protocol_Specification.pdf)
