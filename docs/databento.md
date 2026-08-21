# Databento integration

Live market data from Nasdaq, NYSE, CME, OPRA and 25 other venues over one
session, without exchange ports or a co-location cabinet.

## Status

| Capability | State |
|---|---|
| Live source (`DatabentoSource`) | **Built**, behind `--features databento` |
| Historical backfill | **Not built, deliberately** — see below |
| Live entitlement on the current account | **Refused** — see below |

## Two things measured, not assumed

### 1. Live requires a licence this account has not activated

The gateway refuses at the CRAM handshake, before any subscription:

```
XNAS.ITCH     REFUSED: authentication failed: A live data license is required to access XNAS.ITCH.
EQUS.MINI     REFUSED: ...
GLBX.MDP3     REFUSED: ...
OPRA.PILLAR   REFUSED: ...
```

This is a **licensing** gate, not a billing one — it corresponds to the
"Activate live data" questionnaire in the Databento portal, which generates the
venue contracts. Note that `metadata.list_unit_prices` *does* quote a live mode
(`mbo` at $1.44/GB on XNAS.ITCH), so live is usage-billable rather than
exclusive to a $199/mo plan — but the licence has to exist first.

Reproduce with the probe, which authenticates and closes without subscribing or
calling `start()`, so nothing is delivered and nothing is billed:

```bash
DATABENTO_API_KEY=... cargo run -p ingestion --features databento \
  --example databento_probe -- XNAS.ITCH GLBX.MDP3
```

### 2. The portal's latency figures are not your latency

The portal shows ~3.7 ms to Equinix NY4 and ~14.8 ms to CyrusOne Aurora. Those
are measured **from a receive location you select in the portal**, not from the
machine running this code.

Measured TCP handshake from a development machine in India:

| Gateway | Location | Median RTT |
|---|---|---|
| `xnas-itch.lsg.databento.com` | NY4, Secaucus | **233 ms** |
| `equs-mini.lsg.databento.com` | NY4, Secaucus | **247 ms** |
| `opra-pillar.lsg.databento.com` | NY4, Secaucus | **248 ms** |
| `glbx-mdp3.lsg.databento.com` | Aurora, IL | **250 ms** |
| `hist.databento.com` | — | 244 ms |

Every gateway is within a couple of milliseconds of every other, because the
transit dominates entirely. The ~11 ms New Jersey ↔ Chicago difference that
matters from inside a US data centre is **invisible from here** — it is under
5% of the total and inside the jitter.

The consequence for this repository is specific: the `wire` stage in
`docs/latency.md` would be ~235 ms of transit if it were recorded against this
path, which says nothing about feed-handler performance. Latency work using
Databento from outside a US facility should read `decode` and `book`, and
ignore anything upstream of the socket.

## Why there is no historical backfill here

`HistoricalClient` bills per gigabyte. A market-data **source** that spends
money inside `connect()` is a trap: a supervisor restarting a strategy in a
reconnect loop would bill repeatedly, and nothing in the call site suggests a
cost.

Backfill belongs in an explicit operator-run tool that prints
`metadata.get_cost` and waits for confirmation. `metadata.*` endpoints are free,
which is what makes that possible — and how the coverage table below was
produced without spending anything.

## Coverage, as of 2026-08-21

From `metadata.get_dataset_range`, free:

| Dataset | Coverage |
|---|---|
| `XNAS.ITCH` | 2018-05-01 → 2026-08-21 04:00 UTC |
| `XNYS.PILLAR` | 2018-05-01 → 2026-08-21 04:00 UTC |
| `GLBX.MDP3` | 2010-06-06 → 2026-08-21 00:58 UTC |
| `EQUS.MINI` | 2023-03-28 → 2026-08-21 00:00 UTC |
| `OPRA.PILLAR` | 2013-04-01 → 2026-08-20 13:30 UTC |

US equities end at 04:00 UTC — midnight Eastern, the session boundary — so
these are current to the most recent completed session, not stale.

## Configuration

```bash
DATABENTO_API_KEY=db-...      # required; no default
DATABENTO_DATASET=XNAS.ITCH   # required; no default
```

Neither is defaulted. A default dataset would connect to, and bill for, a venue
the operator never chose. Both are reported by name when missing, because
"connection failed" alone sends someone to the network when the problem is an
unset variable.

## Schema mapping

| Repo `DataType` | DBN schema |
|---|---|
| `Trades` | `trades` |
| `Quotes`, `OrderBookL1` | `mbp-1` |
| `OrderBookL2` | `mbp-10` |
| `Bars1m` | *unmapped* — reported, not silently dropped |

Quotes and L1 collapse to one schema, and the list is deduplicated — the same
issue that had the Binance source subscribing to `@bookTicker` twice.

## Details that are easy to get wrong

**Prices are fixed-point with 9 implied decimals**, matching
`exchange_core::Price`. The conversion to `f64` happens in exactly one function
because `MarketEvent` is defined in `f64`; that is the lossy boundary.

**`i64::MAX` is the undefined-price sentinel**, not zero — zero is a legitimate
price. Publishing the sentinel scaled would quote roughly 9.2 billion dollars,
so a record carrying it is dropped.

**`side` is a C char (`i8`)**, not `u8`. Anything other than `B`/`A` is reported
as `Unknown` rather than defaulted to a side, because the aggressor is a fact
the feed either stated or did not.

**Symbol mappings arrive in-band.** A live session sends `SymbolMappingMsg`
records as it goes, and mappings can be revised mid-session, so `PitSymbolMap`
is fed from the stream rather than snapshotted from the start-up metadata. A
record whose instrument id is not yet mapped is dropped rather than published
under a guessed symbol — the same rule the XDP handler applies to a message
arriving before its price scale.

**The `Debug` impl is hand-written** so the API key cannot reach a log line.
`#[derive(Debug)]` would print it into every panic message and error report
that formats the struct, and a key in a log is a key that needs rotating.

## What to do next

1. Activate the live licence in the portal (the questionnaire), then re-run the
   probe — it should report `ENTITLED`.
2. Register the source in `daemon/src/bootstrap.rs` with `add_source_for`, so
   it receives only the symbols its dataset serves.
3. For L3 development without a live licence, capture a small DBN file with an
   explicit cost check and replay it through `crates/exchange/src/replay.rs`.
