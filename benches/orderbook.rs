#![allow(clippy::cast_possible_wrap, clippy::cast_lossless, clippy::cast_sign_loss)]

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use matcher::orderbook::{OrderBook, OrderType, Side, UDecimal};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::hint::black_box;

fn create_base_book(bids_count: usize, asks_count: usize) -> OrderBook {
    let mut book = OrderBook::new("BTC-USDT".to_string(), Some(UDecimal::new(dec!(65000)).unwrap()));
    for i in 1..=bids_count {
        let price = UDecimal::new(dec!(65000) - Decimal::from(i as i64)).unwrap();
        let qty = UDecimal::new(dec!(1.5)).unwrap();
        book.add_order(
            format!("maker-bid-{i}"),
            format!("user-maker-{i}"),
            Side::Buy,
            OrderType::Limit,
            Some(price),
            qty,
        )
        .unwrap();
    }
    for i in 1..=asks_count {
        let price = UDecimal::new(dec!(65000) + Decimal::from(i as i64)).unwrap();
        let qty = UDecimal::new(dec!(1.5)).unwrap();
        book.add_order(
            format!("maker-ask-{i}"),
            format!("user-maker-{i}"),
            Side::Sell,
            OrderType::Limit,
            Some(price),
            qty,
        )
        .unwrap();
    }
    book
}

fn bench_limit_order_insert_resting(c: &mut Criterion) {
    let base_book = create_base_book(100, 100);
    let price = Some(UDecimal::new(dec!(64000)).unwrap());
    let qty = UDecimal::new(dec!(2.0)).unwrap();

    c.bench_function("limit_order_insert_resting", |b| {
        b.iter_batched(
            || base_book.clone(),
            |mut book| {
                black_box(
                    book.add_order(
                        "bench-insert".to_string(),
                        "bench-user".to_string(),
                        Side::Buy,
                        OrderType::Limit,
                        price,
                        qty,
                    )
                    .unwrap(),
                );
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_market_order_single_level_match(c: &mut Criterion) {
    let mut base_book = OrderBook::new("BTC-USDT".to_string(), Some(UDecimal::new(dec!(65000)).unwrap()));
    base_book
        .add_order(
            "maker-ask-1".to_string(),
            "user-maker".to_string(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(65001)).unwrap()),
            UDecimal::new(dec!(10.0)).unwrap(),
        )
        .unwrap();
    let taker_qty = UDecimal::new(dec!(10.0)).unwrap();

    c.bench_function("market_order_single_level_match", |b| {
        b.iter_batched(
            || base_book.clone(),
            |mut book| {
                black_box(
                    book.add_order(
                        "taker-market".to_string(),
                        "user-taker".to_string(),
                        Side::Buy,
                        OrderType::Market,
                        None,
                        taker_qty,
                    )
                    .unwrap(),
                );
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_market_order_sweep_levels(c: &mut Criterion) {
    let mut group = c.benchmark_group("market_order_sweep_levels");
    for &levels in &[5, 20, 50] {
        let mut base_book = OrderBook::new("BTC-USDT".to_string(), Some(UDecimal::new(dec!(65000)).unwrap()));
        for i in 1..=levels {
            let price = UDecimal::new(dec!(65000) + Decimal::from(i as i64)).unwrap();
            base_book
                .add_order(
                    format!("maker-ask-{i}"),
                    format!("user-maker-{i}"),
                    Side::Sell,
                    OrderType::Limit,
                    Some(price),
                    UDecimal::new(dec!(1.0)).unwrap(),
                )
                .unwrap();
        }
        let taker_qty = UDecimal::new(Decimal::from(levels as i64)).unwrap();

        group.bench_with_input(BenchmarkId::from_parameter(levels), &levels, |b, _| {
            b.iter_batched(
                || base_book.clone(),
                |mut book| {
                    black_box(
                        book.add_order(
                            "taker-sweep".to_string(),
                            "user-taker".to_string(),
                            Side::Buy,
                            OrderType::Market,
                            None,
                            taker_qty,
                        )
                        .unwrap(),
                    );
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_limit_order_partial_match_and_rest(c: &mut Criterion) {
    let mut base_book = OrderBook::new("BTC-USDT".to_string(), Some(UDecimal::new(dec!(65000)).unwrap()));
    base_book
        .add_order(
            "maker-ask-1".to_string(),
            "user-maker".to_string(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(65001)).unwrap()),
            UDecimal::new(dec!(2.0)).unwrap(),
        )
        .unwrap();
    let limit_price = Some(UDecimal::new(dec!(65001)).unwrap());
    let limit_qty = UDecimal::new(dec!(5.0)).unwrap();

    c.bench_function("limit_order_partial_match_and_rest", |b| {
        b.iter_batched(
            || base_book.clone(),
            |mut book| {
                black_box(
                    book.add_order(
                        "taker-partial".to_string(),
                        "user-taker".to_string(),
                        Side::Buy,
                        OrderType::Limit,
                        limit_price,
                        limit_qty,
                    )
                    .unwrap(),
                );
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_order_cancel_o1(c: &mut Criterion) {
    let base_book = create_base_book(500, 500);
    let cancel_id = "maker-bid-250";
    let cancel_user = "user-maker-250";

    c.bench_function("order_cancel_o1", |b| {
        b.iter_batched(
            || base_book.clone(),
            |mut book| {
                black_box(book.cancel_order(cancel_id, cancel_user).unwrap());
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_get_depth_snapshot(c: &mut Criterion) {
    let base_book = create_base_book(100, 100);
    let mut group = c.benchmark_group("get_depth_snapshot");
    for &limit in &[10, 50, 100] {
        group.bench_with_input(BenchmarkId::from_parameter(limit), &limit, |b, &lim| {
            b.iter(|| {
                black_box(base_book.get_depth(lim));
            });
        });
    }
    group.finish();
}

criterion_group!(
    name = orderbook_benches;
    config = Criterion::default()
        .sample_size(20)
        .measurement_time(std::time::Duration::from_secs(2));
    targets =
        bench_limit_order_insert_resting,
        bench_market_order_single_level_match,
        bench_market_order_sweep_levels,
        bench_limit_order_partial_match_and_rest,
        bench_order_cancel_o1,
        bench_get_depth_snapshot,
);
criterion_main!(orderbook_benches);
