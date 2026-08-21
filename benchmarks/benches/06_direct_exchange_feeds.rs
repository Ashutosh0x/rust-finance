//! Direct exchange connectivity benchmarks — Nasdaq ITCH/OUCH and NYSE XDP/Pillar.
//!
//! # What these numbers are, and what they are not
//!
//! These measure the **software path only**: the time this process spends turning bytes into
//! a maintained order book, and turning an order request into wire bytes. That is the one
//! component of end-to-end latency that is under this codebase's control.
//!
//! They are **not** exchange round-trip latency. A published figure like Nasdaq's
//! order-to-acknowledgment measurement covers the customer's cabinet, the cross-connect, the
//! exchange's network, the matching engine, and the return path. The software decode that
//! these benchmarks measure is one small term inside the client's share of that budget:
//!
//! ```text
//!   order-to-ack round trip  =  NIC tx + cross-connect + exchange network
//!                            +  matching engine + exchange network + cross-connect
//!                            +  NIC rx + [kernel/stack] + THIS CODE
//! ```
//!
//! No benchmark run on a developer machine can measure the other terms, and this file does
//! not pretend to. What it does establish is whether the decode path is small enough to be
//! irrelevant next to them — which is the only question a feed handler can answer for itself.
//!
//! Run with:
//! ```text
//! cargo bench -p benchmarks --bench 06_direct_exchange_feeds
//! ```

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};

use exchange_core::book::{BookSet, OrderBook};
use exchange_core::feed::{BookEvent, Side, TradeCondition};
use exchange_core::gap::SequenceTracker;
use exchange_core::latency::LatencyHistogram;
use exchange_core::{FirmId5, Mpid4, Price, Symbol8, UserData8};

use nasdaq::itch::{self, encode as itch_encode, Header, ItchFeedHandler, SessionClock};
use nasdaq::moldudp64::{self, SessionId};
use nasdaq::ouch::{self, EnterOrder, OrderToken};
use nasdaq::soupbintcp;

use nyse::pillar::binary as pillar;
use nyse::pillar::fix as pillar_fix;
use nyse::xdp::common::{encode_symbol_index_mapping, SymbolIndexMapping};
use nyse::xdp::{integrated, packet, XdpFeedHandler};

// ─── Fixtures ───────────────────────────────────────────────────────────────

fn h(locate: u16, ts: u64) -> Header {
    Header {
        stock_locate: locate,
        tracking_number: 1,
        timestamp: ts,
    }
}

/// The three ITCH messages that dominate a real session by volume.
fn itch_hot_path() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "A add_order",
            itch_encode::add_order(
                h(1, 34_200_000_000_000),
                42,
                'B',
                500,
                "AAPL",
                Price::from_price4(1_000_000),
                None,
            ),
        ),
        (
            "F add_order_mpid",
            itch_encode::add_order(
                h(1, 34_200_000_000_001),
                43,
                'S',
                300,
                "AAPL",
                Price::from_price4(1_000_200),
                Some("NSDQ"),
            ),
        ),
        (
            "E order_executed",
            itch_encode::order_executed(h(1, 34_200_000_000_002), 42, 100, 900),
        ),
        (
            "C order_executed_px",
            itch_encode::order_executed_with_price(
                h(1, 34_200_000_000_003),
                42,
                100,
                901,
                true,
                Price::from_price4(999_900),
            ),
        ),
        (
            "X order_cancel",
            itch_encode::order_cancel(h(1, 34_200_000_000_004), 42, 50),
        ),
        (
            "D order_delete",
            itch_encode::order_delete(h(1, 34_200_000_000_005), 42),
        ),
        (
            "U order_replace",
            itch_encode::order_replace(
                h(1, 34_200_000_000_006),
                42,
                99,
                400,
                Price::from_price4(1_000_100),
            ),
        ),
        (
            "P trade",
            itch_encode::trade_non_cross(
                h(1, 34_200_000_000_007),
                0,
                'B',
                250,
                "AAPL",
                Price::from_price4(1_000_050),
                902,
            ),
        ),
    ]
}

/// A realistic ITCH message mix, in the proportions a live session produces: adds and
/// deletes dominate, executions are a small minority.
fn itch_session(messages: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::with_capacity(messages + 1);
    out.push(itch_encode::stock_directory(
        h(1, 1),
        "AAPL",
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
    ));

    let mut next_order: u64 = 1;
    let mut live: Vec<u64> = Vec::with_capacity(1024);
    let mut ts = 34_200_000_000_000u64;

    for i in 0..messages {
        ts += 1_000;
        match i % 10 {
            // 60% adds, alternating sides and walking the price a little.
            0..=5 => {
                let side = if i % 2 == 0 { 'B' } else { 'S' };
                let px = if side == 'B' {
                    1_000_000 - (i as u32 % 20) * 100
                } else {
                    1_000_200 + (i as u32 % 20) * 100
                };
                out.push(itch_encode::add_order(
                    h(1, ts),
                    next_order,
                    side,
                    100,
                    "AAPL",
                    Price::from_price4(px),
                    None,
                ));
                live.push(next_order);
                next_order += 1;
            }
            // 20% deletes.
            6 | 7 => {
                if let Some(id) = live.pop() {
                    out.push(itch_encode::order_delete(h(1, ts), id));
                }
            }
            // 10% partial cancels.
            8 => {
                if let Some(&id) = live.first() {
                    out.push(itch_encode::order_cancel(h(1, ts), id, 10));
                }
            }
            // 10% executions.
            _ => {
                if let Some(&id) = live.first() {
                    out.push(itch_encode::order_executed(h(1, ts), id, 10, i as u64));
                }
            }
        }
    }
    out
}

/// Pack ITCH messages into MoldUDP64 datagrams the way the feed does — several messages per
/// packet, which is what makes per-packet overhead amortise.
fn mold_packets(messages: &[Vec<u8>], per_packet: usize) -> Vec<Vec<u8>> {
    let session = SessionId::from_str_padded("20260818AA");
    let mut out = Vec::new();
    let mut seq = 1u64;
    for chunk in messages.chunks(per_packet) {
        let refs: Vec<&[u8]> = chunk.iter().map(|m| m.as_slice()).collect();
        out.push(moldudp64::encode_packet(session, seq, &refs));
        seq += chunk.len() as u64;
    }
    out
}

fn xdp_mapping() -> SymbolIndexMapping {
    SymbolIndexMapping {
        symbol_index: 4242,
        symbol: "IBM".into(),
        market_id: 1,
        system_id: 1,
        exchange_code: 'N',
        price_scale_code: 4,
        security_type: 'C',
        lot_size: 100,
        prev_close_price: Price::ZERO,
        prev_close_volume: 0,
        price_resolution: 0,
        round_lot: 'Y',
        mpv: 1,
        unit_of_trade: 100,
    }
}

fn xdp_hot_path() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "100 add_order",
            integrated::AddOrder {
                source_time_nanos: 123_456_789,
                symbol_index: 4242,
                symbol_seq_num: 1,
                order_id: 42,
                price_raw: 1_805_000,
                volume: 500,
                side: integrated::Side::Buy,
                firm_id: FirmId5::NUL,
            }
            .encode(),
        ),
        (
            "101 modify_order",
            integrated::ModifyOrder {
                source_time_nanos: 123_456_790,
                symbol_index: 4242,
                symbol_seq_num: 2,
                order_id: 42,
                price_raw: 1_805_100,
                volume: 400,
                position_change: 1,
            }
            .encode(),
        ),
        (
            "102 delete_order",
            integrated::DeleteOrder {
                source_time_nanos: 123_456_791,
                symbol_index: 4242,
                symbol_seq_num: 3,
                order_id: 42,
            }
            .encode(),
        ),
        (
            "103 order_execution",
            integrated::OrderExecution {
                source_time_nanos: 123_456_792,
                symbol_index: 4242,
                symbol_seq_num: 4,
                order_id: 42,
                trade_id: 7,
                price_raw: 1_805_000,
                volume: 100,
                printable_flag: 1,
                conditions: Default::default(),
            }
            .encode(),
        ),
        (
            "104 replace_order",
            integrated::ReplaceOrder {
                source_time_nanos: 123_456_793,
                symbol_index: 4242,
                symbol_seq_num: 5,
                order_id: 42,
                new_order_id: 43,
                price_raw: 1_805_200,
                volume: 300,
            }
            .encode(),
        ),
    ]
}

/// An XDP session mix, matching the ITCH one so the two decoders are compared on equal work.
fn xdp_session(messages: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::with_capacity(messages + 2);
    out.push(encode_symbol_index_mapping(&xdp_mapping()));
    out.push(nyse::xdp::common::encode_source_time_reference(
        0,
        0,
        1_700_000_000,
    ));

    let mut next_order: u64 = 1;
    let mut live: Vec<u64> = Vec::with_capacity(1024);

    for i in 0..messages {
        let ns = (i as u32).wrapping_mul(1_000);
        match i % 10 {
            0..=5 => {
                let side = if i % 2 == 0 {
                    integrated::Side::Buy
                } else {
                    integrated::Side::Sell
                };
                let px = if matches!(side, integrated::Side::Buy) {
                    1_805_000 - (i as i32 % 20) * 100
                } else {
                    1_805_200 + (i as i32 % 20) * 100
                };
                out.push(
                    integrated::AddOrder {
                        source_time_nanos: ns,
                        symbol_index: 4242,
                        symbol_seq_num: i as u32,
                        order_id: next_order,
                        price_raw: px,
                        volume: 100,
                        side,
                        firm_id: FirmId5::NUL,
                    }
                    .encode(),
                );
                live.push(next_order);
                next_order += 1;
            }
            6 | 7 => {
                if let Some(id) = live.pop() {
                    out.push(
                        integrated::DeleteOrder {
                            source_time_nanos: ns,
                            symbol_index: 4242,
                            symbol_seq_num: i as u32,
                            order_id: id,
                        }
                        .encode(),
                    );
                }
            }
            8 => {
                if let Some(&id) = live.first() {
                    out.push(
                        integrated::ModifyOrder {
                            source_time_nanos: ns,
                            symbol_index: 4242,
                            symbol_seq_num: i as u32,
                            order_id: id,
                            price_raw: 1_805_000,
                            volume: 90,
                            position_change: 0,
                        }
                        .encode(),
                    );
                }
            }
            _ => {
                if let Some(&id) = live.first() {
                    out.push(
                        integrated::OrderExecution {
                            source_time_nanos: ns,
                            symbol_index: 4242,
                            symbol_seq_num: i as u32,
                            order_id: id,
                            trade_id: i as u32,
                            price_raw: 1_805_000,
                            volume: 10,
                            printable_flag: 1,
                            conditions: Default::default(),
                        }
                        .encode(),
                    );
                }
            }
        }
    }
    out
}

fn xdp_packets(messages: &[Vec<u8>], per_packet: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut seq = 1u32;
    for chunk in messages.chunks(per_packet) {
        let refs: Vec<&[u8]> = chunk.iter().map(|m| m.as_slice()).collect();
        out.push(packet::encode_packet(
            packet::delivery_flag::ORIGINAL,
            seq,
            1_700_000_000,
            0,
            &refs,
        ));
        seq += chunk.len() as u32;
    }
    out
}

// ─── 1. Single-message decode ───────────────────────────────────────────────

fn bench_itch_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("itch_decode_single_message");
    group.throughput(Throughput::Elements(1));

    for (name, bytes) in itch_hot_path() {
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| itch::decode(black_box(&bytes)).unwrap());
        });
    }
    group.finish();
}

fn bench_xdp_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("xdp_decode_single_message");
    group.throughput(Throughput::Elements(1));

    for (name, bytes) in xdp_hot_path() {
        let (_, msg_type) = packet::message_header(&bytes).unwrap();
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| integrated::decode(black_box(msg_type), black_box(&bytes)).unwrap());
        });
    }
    group.finish();
}

/// The cheap filter that makes a small watch list nearly free on the full tape: read the
/// stock locate at a fixed offset and discard without decoding any other field.
fn bench_itch_locate_filter(c: &mut Criterion) {
    let bytes = itch_encode::add_order(
        h(9_999, 1),
        1,
        'B',
        100,
        "AAPL",
        Price::from_price4(1_000_000),
        None,
    );
    let mut group = c.benchmark_group("itch_prefilter");
    group.throughput(Throughput::Elements(1));
    group.bench_function("peek_stock_locate", |b| {
        b.iter(|| itch::peek_stock_locate(black_box(&bytes)));
    });
    group.bench_function("full_decode_for_comparison", |b| {
        b.iter(|| itch::decode(black_box(&bytes)).unwrap());
    });
    group.finish();
}

// ─── 2. Session-layer framing ───────────────────────────────────────────────

fn bench_transport_framing(c: &mut Criterion) {
    let messages = itch_session(64);
    let mold = mold_packets(&messages, 16);
    let one_packet = &mold[0];

    let mut group = c.benchmark_group("transport_framing");

    group.throughput(Throughput::Elements(16));
    group.bench_function("moldudp64_parse_and_walk_16_msgs", |b| {
        b.iter(|| {
            let p = moldudp64::MoldPacket::parse(black_box(one_packet)).unwrap();
            let mut n = 0usize;
            for m in p.messages() {
                n += m.len();
            }
            n
        });
    });

    let xdp_msgs = xdp_session(64);
    let xdp_pkts = xdp_packets(&xdp_msgs, 16);
    let xdp_one = &xdp_pkts[1];
    group.bench_function("xdp_parse_and_walk_16_msgs", |b| {
        b.iter(|| {
            let p = packet::Packet::parse(black_box(xdp_one)).unwrap();
            let mut n = 0usize;
            for m in p.messages() {
                n += m.bytes.len();
            }
            n
        });
    });

    // SoupBinTCP frames one higher-level message per packet, so its per-message cost is the
    // one that matters for an order-entry session.
    let soup = soupbintcp::encode_sequenced(&messages[1]);
    group.throughput(Throughput::Elements(1));
    group.bench_function("soupbintcp_decode_packet", |b| {
        b.iter(|| soupbintcp::decode_packet(black_box(&soup)).unwrap());
    });

    group.finish();
}

// ─── 3. Order book ──────────────────────────────────────────────────────────

fn bench_order_book(c: &mut Criterion) {
    let mut group = c.benchmark_group("order_book_l3");
    group.throughput(Throughput::Elements(1));

    // Apply into a book already carrying realistic depth, so the BTreeMap is not trivially
    // shallow — a book benchmarked empty flatters itself.
    let seeded = || {
        let mut book = OrderBook::new(1, "AAPL");
        for i in 0..1_000u64 {
            let side = if i % 2 == 0 { Side::Buy } else { Side::Sell };
            let px = if side == Side::Buy {
                1_000_000 - (i as u32 % 100) * 100
            } else {
                1_000_200 + (i as u32 % 100) * 100
            };
            book.apply(&BookEvent::Add {
                key: 1,
                symbol: "AAPL",
                ts: i,
                order_id: i + 1,
                side,
                price: Price::from_price4(px),
                qty: 100,
                participant: None,
            })
            .unwrap();
        }
        book
    };

    group.bench_function("add_order_into_1000_order_book", |b| {
        b.iter_batched_ref(
            seeded,
            |book| {
                book.apply(black_box(&BookEvent::Add {
                    key: 1,
                    symbol: "AAPL",
                    ts: 9_999,
                    order_id: 999_999,
                    side: Side::Buy,
                    price: Price::from_price4(1_000_100),
                    qty: 200,
                    participant: None,
                }))
                .unwrap()
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("execute_at_touch", |b| {
        b.iter_batched_ref(
            seeded,
            |book| {
                book.apply(black_box(&BookEvent::Execute {
                    key: 1,
                    ts: 9_999,
                    order_id: 1,
                    qty: 50,
                    price: None,
                    trade_id: 1,
                    condition: TradeCondition::Printable,
                }))
                .unwrap()
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("delete_order", |b| {
        b.iter_batched_ref(
            seeded,
            |book| {
                book.apply(black_box(&BookEvent::Delete {
                    key: 1,
                    ts: 9_999,
                    order_id: 500,
                }))
                .unwrap()
            },
            BatchSize::SmallInput,
        );
    });

    let book = seeded();
    group.bench_function("best_bid_offer_read", |b| {
        b.iter(|| black_box(&book).bbo());
    });
    group.bench_function("depth_10_levels", |b| {
        b.iter(|| black_box(&book).depth(Side::Buy, 10));
    });

    group.finish();
}

// ─── 4. End-to-end: wire bytes → maintained book ────────────────────────────

fn bench_end_to_end(c: &mut Criterion) {
    const MESSAGES: usize = 10_000;

    let mut group = c.benchmark_group("wire_to_book_end_to_end");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(10));
    group.throughput(Throughput::Elements(MESSAGES as u64));

    // Nasdaq: MoldUDP64 datagram → ITCH decode → book apply, 16 messages per packet.
    let itch_msgs = itch_session(MESSAGES);
    let itch_pkts = mold_packets(&itch_msgs, 16);
    group.bench_function("nasdaq_mold_itch_book", |b| {
        b.iter_batched(
            || ItchFeedHandler::new(SessionClock::new(1_755_489_600_000_000_000)),
            |mut handler| {
                for pkt in &itch_pkts {
                    let p = moldudp64::MoldPacket::parse(pkt).unwrap();
                    for raw in p.messages() {
                        let _ = handler.on_message(black_box(raw), 0);
                    }
                }
                handler
            },
            BatchSize::LargeInput,
        );
    });

    // NYSE: XDP packet → Integrated decode → book apply, 16 messages per packet.
    let xdp_msgs = xdp_session(MESSAGES);
    let xdp_pkts = xdp_packets(&xdp_msgs, 16);
    group.bench_function("nyse_xdp_integrated_book", |b| {
        b.iter_batched(
            XdpFeedHandler::new,
            |mut handler| {
                for pkt in &xdp_pkts {
                    // recv_ns = 0: this benchmark measures the decode path alone.
                    // Passing a live stamp would fold the latency instrumentation
                    // into the number, and 07_latency_recording measures that
                    // separately for exactly that reason.
                    let _ = handler.on_packet(black_box(pkt), 0, 0);
                    handler.clear_events();
                }
                handler
            },
            BatchSize::LargeInput,
        );
    });

    group.finish();
}

/// Per-packet cost across packing densities. MoldUDP64 aggregates messages to cut network
/// traffic, so this shows how much of the cost is per-packet versus per-message.
fn bench_packing_density(c: &mut Criterion) {
    let messages = itch_session(4_096);
    let mut group = c.benchmark_group("mold_packing_density");
    group.throughput(Throughput::Elements(4_096));

    for per_packet in [1usize, 4, 16, 64] {
        let pkts = mold_packets(&messages, per_packet);
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{per_packet}_msgs_per_packet")),
            &pkts,
            |b, pkts| {
                b.iter_batched(
                    || ItchFeedHandler::new(SessionClock::raw()),
                    |mut handler| {
                        for pkt in pkts {
                            let p = moldudp64::MoldPacket::parse(pkt).unwrap();
                            for raw in p.messages() {
                                let _ = handler.on_message(raw, 0);
                            }
                        }
                        handler
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

// ─── 5. Order entry (the path that feeds order-to-ack) ──────────────────────

fn bench_order_entry(c: &mut Criterion) {
    let mut group = c.benchmark_group("order_entry_encode");
    group.throughput(Throughput::Elements(1));

    // Nasdaq OUCH 4.2: 49 bytes.
    let ouch_order = EnterOrder::limit(
        OrderToken::new("RF0000001"),
        ouch::Side::Buy,
        500,
        "AAPL",
        Price::from_f64(123.45),
    );
    group.bench_function("nasdaq_ouch_enter_order", |b| {
        b.iter(|| black_box(&ouch_order).encode());
    });
    group.bench_function("nasdaq_ouch_enter_order_validated", |b| {
        b.iter(|| {
            let o = black_box(&ouch_order);
            o.validate().unwrap();
            o.encode()
        });
    });

    // The full outbound path an order actually takes: OUCH message wrapped in a SoupBinTCP
    // Unsequenced Data packet.
    group.bench_function("nasdaq_ouch_plus_soup_framing", |b| {
        b.iter(|| soupbintcp::encode_unsequenced(&black_box(&ouch_order).encode()));
    });

    // NYSE Pillar Binary: 65-byte order inside a 32-byte SeqMsg envelope.
    let pillar_order = pillar::NewOrder {
        symbol_id: 4242,
        mpid: Mpid4::new("ABCD"),
        mmid: 0,
        mp_sub_id: 'A',
        cl_ord_id: 1,
        orig_cl_ord_id: 0,
        instructions: pillar::OrderInstructions::default(),
        price: Price::from_f64(180.50),
        order_qty: 500,
        min_qty: 0,
        user_data: UserData8::NUL,
    };
    group.bench_function("nyse_pillar_binary_new_order", |b| {
        b.iter(|| black_box(&pillar_order).encode());
    });
    group.bench_function("nyse_pillar_binary_with_seqmsg_envelope", |b| {
        b.iter(|| {
            let payload = black_box(&pillar_order).encode();
            pillar::SeqMsg::new(
                pillar::SeqMsgId {
                    stream_id: 1,
                    sequence: 1,
                },
                1_700_000_000_000_000_000,
                payload,
            )
            .encode()
        });
    });

    // The bitfield packing that carries every order attribute on the Pillar binary gateway.
    let instructions = pillar::OrderInstructions::default();
    group.bench_function("pillar_order_instructions_pack", |b| {
        b.iter(|| black_box(&instructions).to_bits());
    });

    // NYSE Pillar FIX 4.2, for the comparison that actually matters when choosing a gateway.
    let fix_session = pillar_fix::FixSessionConfig::new("FIRMFIX", "NYSE", "ABCD");
    let fix_order = pillar_fix::NewOrderSingle::limit(
        "ORD00000001",
        "IBM",
        pillar_fix::Side::Buy,
        500,
        Price::from_f64(180.50),
    );
    group.bench_function("nyse_pillar_fix_new_order_single", |b| {
        b.iter(|| {
            black_box(&fix_order).encode(
                &fix_session,
                1,
                "20260818-14:30:00.000",
                "20260818-14:30:00.000",
            )
        });
    });

    group.finish();
}

/// Inbound acknowledgement decode — the last software step before a strategy learns its
/// order is live.
fn bench_ack_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("order_ack_decode");
    group.throughput(Throughput::Elements(1));

    let ouch_ack = ouch::Outbound::Accepted(ouch::Acknowledgement {
        timestamp: 34_200_000_000_000,
        token: OrderToken::new("RF0000001"),
        side: ouch::Side::Buy,
        shares: 500,
        stock: Symbol8::new("AAPL"),
        price: Price::from_f64(123.45),
        time_in_force: ouch::time_in_force::MARKET_HOURS,
        firm: Mpid4::new("ABCD"),
        display: ouch::Display::Anonymous,
        reference_number: 987_654_321,
        capacity: ouch::Capacity::Agency,
        iso_eligibility: ouch::IsoEligibility::NotEligible,
        min_quantity: 0,
        cross_type: ouch::CrossType::None,
        order_state: ouch::OrderState::Live,
        previous_token: None,
        bbo_weight_indicator: '0',
    })
    .encode();
    group.bench_function("nasdaq_ouch_accepted", |b| {
        b.iter(|| ouch::Outbound::decode(black_box(&ouch_ack)).unwrap());
    });

    let ouch_fill = ouch::Outbound::Executed(ouch::Executed {
        timestamp: 34_200_000_000_001,
        token: OrderToken::new("RF0000001"),
        executed_shares: 100,
        execution_price: Price::from_f64(123.45),
        liquidity_flag: 'R',
        match_number: 55_555,
        reference_price: None,
        reference_price_type: None,
    })
    .encode();
    group.bench_function("nasdaq_ouch_executed", |b| {
        b.iter(|| ouch::Outbound::decode(black_box(&ouch_fill)).unwrap());
    });

    let pillar_exec = pillar::ExecutionReport {
        transact_time: 1_700_000_000_000_000_000,
        symbol_id: 4242,
        mpid: Mpid4::new("ABCD"),
        order_id: 0x0102_0304_0506_0708,
        cl_ord_id: 1,
        deal_id: 0x00AA_BBCC_DDEE_FF00,
        last_px: Price::from_f64(180.50),
        leaves_qty: 0,
        cum_qty: 500,
        last_qty: 500,
        liquidity_indicator: Mpid4::new("R"),
        execution_details: 0,
        locate_reqd: 0,
        participant_type: pillar::ParticipantType::Customer,
        reason_code: 0,
        user_data: UserData8::NUL,
    }
    .encode();
    group.bench_function("nyse_pillar_binary_execution_report", |b| {
        b.iter(|| pillar::ExecutionReport::parse(black_box(&pillar_exec)).unwrap());
    });

    // The same event over FIX, for the binary-versus-FIX comparison.
    let mut b = pillar_fix::FixMessageBuilder::new("8");
    b.set(pillar_fix::tag::MSG_SEQ_NUM, "5")
        .set(pillar_fix::tag::SENDER_COMP_ID, "NYSE")
        .set(pillar_fix::tag::SENDING_TIME, "20260818-14:30:00.000")
        .set(pillar_fix::tag::TARGET_COMP_ID, "FIRMFIX")
        .set(pillar_fix::tag::CL_ORD_ID, "ORD00000001")
        .set(pillar_fix::tag::ORDER_ID, "9876543210")
        .set(pillar_fix::tag::EXEC_ID, "EX1")
        .set(pillar_fix::tag::EXEC_TYPE, "2")
        .set(pillar_fix::tag::ORD_STATUS, "2")
        .set(pillar_fix::tag::SYMBOL, "IBM")
        .set(pillar_fix::tag::SIDE, "1")
        .set(pillar_fix::tag::LAST_PX, "180.500000")
        .set(pillar_fix::tag::LAST_QTY, "500")
        .set(pillar_fix::tag::CUM_QTY, "500")
        .set(pillar_fix::tag::LEAVES_QTY, "0")
        .set(pillar_fix::tag::LIQUIDITY_INDICATOR, "R");
    let fix_exec = b.encode("FIX.4.2");
    group.bench_function("nyse_pillar_fix_execution_report", |bencher| {
        bencher.iter(|| {
            let m = pillar_fix::FixMessage::parse(black_box(&fix_exec)).unwrap();
            pillar_fix::ExecutionReport::from_message(&m)
        });
    });

    group.finish();
}

// ─── 6. Supporting machinery ────────────────────────────────────────────────

fn bench_infrastructure(c: &mut Criterion) {
    let mut group = c.benchmark_group("infrastructure");
    group.throughput(Throughput::Elements(1));

    // Sequence tracking runs on every packet, so its cost is never amortised away.
    group.bench_function("gap_tracker_in_order_packet", |b| {
        b.iter_batched_ref(
            || {
                let mut t = SequenceTracker::new(10_000);
                t.observe(1, 1);
                t
            },
            |t| t.observe(black_box(2), black_box(16)),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("gap_tracker_duplicate_ab_line", |b| {
        b.iter_batched_ref(
            || {
                let mut t = SequenceTracker::new(10_000);
                t.observe(1, 100);
                t
            },
            |t| t.observe(black_box(1), black_box(100)),
            BatchSize::SmallInput,
        );
    });

    // Price conversions happen on every price field of every message.
    group.bench_function("price_from_itch_price4", |b| {
        b.iter(|| Price::from_price4(black_box(1_234_500)));
    });
    group.bench_function("price_from_xdp_scaled", |b| {
        b.iter(|| Price::from_xdp(black_box(1_805_000), black_box(4)));
    });
    group.bench_function("price_to_pillar_scale8", |b| {
        let p = Price::from_f64(180.50);
        b.iter(|| black_box(p).to_pillar());
    });

    // Latency recording is only useful if recording it is cheaper than what it measures.
    group.bench_function("latency_histogram_record", |b| {
        b.iter_batched_ref(
            || LatencyHistogram::new("bench"),
            |h| h.record(black_box(4_237)),
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

/// Book maintenance across many instruments, which is what a real session does — the
/// hash lookup per event is not free and does not show up in a single-book benchmark.
fn bench_multi_instrument(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_instrument_bookset");
    group.throughput(Throughput::Elements(1));

    for instruments in [1u32, 100, 3_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{instruments}_symbols")),
            &instruments,
            |b, &instruments| {
                b.iter_batched_ref(
                    || {
                        let mut set = BookSet::new();
                        for k in 0..instruments {
                            set.apply(&BookEvent::Add {
                                key: k,
                                symbol: "SYM",
                                ts: 1,
                                order_id: k as u64 + 1,
                                side: Side::Buy,
                                price: Price::from_price4(1_000_000),
                                qty: 100,
                                participant: None,
                            })
                            .unwrap();
                        }
                        set
                    },
                    |set| {
                        set.apply(black_box(&BookEvent::Add {
                            key: instruments / 2,
                            symbol: "SYM",
                            ts: 2,
                            order_id: 10_000_000,
                            side: Side::Sell,
                            price: Price::from_price4(1_000_200),
                            qty: 100,
                            participant: None,
                        }))
                        .unwrap();
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(
    direct_exchange,
    bench_itch_decode,
    bench_xdp_decode,
    bench_itch_locate_filter,
    bench_transport_framing,
    bench_order_book,
    bench_end_to_end,
    bench_packing_density,
    bench_order_entry,
    bench_ack_decode,
    bench_infrastructure,
    bench_multi_instrument,
);
criterion_main!(direct_exchange);
