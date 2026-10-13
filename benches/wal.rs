#![allow(clippy::cast_possible_wrap, clippy::cast_lossless, clippy::cast_sign_loss)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use matcher::orderbook::{Order, OrderType, Side, UDecimal};
use matcher::wal::{
    crc32, BookSnapshot, SbeDecoder, SbeEncoder, SnapshotReader, SnapshotWriter, SyncPolicy, WalWriter,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::hint::black_box;

fn bench_sbe_encode_decode(c: &mut Criterion) {
    let encoder = SbeEncoder::new();
    let decoder = SbeDecoder::new();

    let price = Some(UDecimal::new(dec!(65500.50)).unwrap());
    let qty = UDecimal::new(dec!(2.5)).unwrap();

    // 1. SubmitOrder frame
    let submit_frame = encoder
        .encode_submit_order(
            "ord-12345678",
            "user-abcdefgh",
            Side::Buy,
            OrderType::Limit,
            price,
            qty,
            1,
            1_700_000_000_000,
        )
        .unwrap();

    // 2. CancelOrder frame
    let cancel_frame = encoder
        .encode_cancel_order("ord-12345678", "user-abcdefgh", 2, 1_700_000_000_001)
        .unwrap();

    // 3. SnapshotMarker frame
    let marker_frame = encoder.encode_snapshot_marker(100, 50, 3, 1_700_000_000_002);

    let mut group = c.benchmark_group("sbe_encode");
    group.bench_function("submit_order", |b| {
        b.iter(|| {
            black_box(
                encoder
                    .encode_submit_order(
                        "ord-12345678",
                        "user-abcdefgh",
                        Side::Buy,
                        OrderType::Limit,
                        price,
                        qty,
                        1,
                        1_700_000_000_000,
                    )
                    .unwrap(),
            );
        });
    });

    group.bench_function("cancel_order", |b| {
        b.iter(|| {
            black_box(
                encoder
                    .encode_cancel_order("ord-12345678", "user-abcdefgh", 2, 1_700_000_000_001)
                    .unwrap(),
            );
        });
    });

    group.bench_function("snapshot_marker", |b| {
        b.iter(|| {
            black_box(encoder.encode_snapshot_marker(100, 50, 3, 1_700_000_000_002));
        });
    });
    group.finish();

    let mut group = c.benchmark_group("sbe_decode");
    group.bench_function("submit_order", |b| {
        b.iter(|| {
            black_box(decoder.decode_frame(&submit_frame).unwrap());
        });
    });

    group.bench_function("cancel_order", |b| {
        b.iter(|| {
            black_box(decoder.decode_frame(&cancel_frame).unwrap());
        });
    });

    group.bench_function("snapshot_marker", |b| {
        b.iter(|| {
            black_box(decoder.decode_frame(&marker_frame).unwrap());
        });
    });
    group.finish();
}

fn bench_crc32_checksum(c: &mut Criterion) {
    let mut group = c.benchmark_group("crc32_checksum");
    for &size in &[64, 128, 1024] {
        let payload = vec![0xA5u8; size];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(format!("{size}B")), &payload, |b, data| {
            b.iter(|| {
                black_box(crc32(data));
            });
        });
    }
    group.finish();
}

fn bench_wal_write_sync_none(c: &mut Criterion) {
    let temp_dir = std::env::temp_dir();
    let wal_path = temp_dir.join(format!(
        "bench_wal_{}.wal",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let mut writer = WalWriter::create(&wal_path, SyncPolicy::None).unwrap();
    let price = Some(UDecimal::new(dec!(65000)).unwrap());
    let qty = UDecimal::new(dec!(1.5)).unwrap();

    let mut group = c.benchmark_group("wal_write_sync_none");
    group.bench_function("append_submit_order", |b| {
        b.iter(|| {
            black_box(
                writer
                    .append_submit_order(
                        "bench-ord",
                        "bench-user",
                        Side::Buy,
                        OrderType::Limit,
                        price,
                        qty,
                        1_700_000_000_000,
                    )
                    .unwrap(),
            );
        });
    });
    group.finish();

    let _ = std::fs::remove_file(&wal_path);
}

fn create_sample_snapshot(symbol: &str, order_count: usize) -> BookSnapshot {
    let mut orders = Vec::with_capacity(order_count);
    for i in 1..=order_count {
        let side = if i % 2 == 0 { Side::Buy } else { Side::Sell };
        let price = UDecimal::new(dec!(65000) + Decimal::from(i as i64)).unwrap();
        let qty = UDecimal::new(dec!(1.5)).unwrap();
        let order = Order::new(
            format!("ord-{i}"),
            format!("user-{i}"),
            symbol.to_string(),
            side,
            OrderType::Limit,
            qty,
            Some(price),
        )
        .unwrap();
        orders.push(order);
    }
    BookSnapshot {
        symbol: symbol.to_string(),
        sequence: order_count as u64,
        timestamp_ns: 1_700_000_000_000,
        last_traded_price: Some(UDecimal::new(dec!(65000)).unwrap()),
        total_volume_traded: UDecimal::new(dec!(500000.0)).unwrap(),
        total_trades_count: (order_count / 2) as u64,
        open_orders: orders,
    }
}

fn bench_snapshot_serialize_deserialize(c: &mut Criterion) {
    let temp_dir = std::env::temp_dir();
    let mut group = c.benchmark_group("snapshot_serialize_deserialize");

    for &order_count in &[1000, 10000] {
        let snapshot = create_sample_snapshot("BTC-USDT", order_count);
        let snap_path = temp_dir.join(format!(
            "bench_snap_{order_count}_{}.snap",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        // Pre-write snapshot for deserialization benchmark
        SnapshotWriter::write_snapshot(&snap_path, &snapshot).unwrap();

        // 1. Benchmark Deserialization / Recovery
        group.bench_with_input(
            BenchmarkId::new("deserialize_snapshot", format!("{order_count}_orders")),
            &snap_path,
            |b, path| {
                b.iter(|| {
                    black_box(SnapshotReader::read_snapshot(path).unwrap());
                });
            },
        );

        // 2. Benchmark Serialization
        let write_target = temp_dir.join(format!(
            "bench_snap_write_{order_count}_{}.snap",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        group.bench_with_input(
            BenchmarkId::new("serialize_snapshot", format!("{order_count}_orders")),
            &snapshot,
            |b, snap| {
                b.iter(|| {
                    SnapshotWriter::write_snapshot(&write_target, snap).unwrap();
                    black_box(());
                });
            },
        );

        let _ = std::fs::remove_file(&snap_path);
        let _ = std::fs::remove_file(&write_target);
    }
    group.finish();
}

criterion_group!(
    name = wal_benches;
    config = Criterion::default()
        .sample_size(20)
        .measurement_time(std::time::Duration::from_secs(2));
    targets =
        bench_sbe_encode_decode,
        bench_crc32_checksum,
        bench_wal_write_sync_none,
        bench_snapshot_serialize_deserialize,
);
criterion_main!(wal_benches);
