#![allow(clippy::cast_possible_wrap, clippy::cast_lossless, clippy::cast_sign_loss)]

use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use matcher::engine::ShardedEngine;
use matcher::orderbook::{OrderType, Side, UDecimal};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

static GLOBAL_ORDER_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_order_id() -> String {
    let id = GLOBAL_ORDER_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("o-{id}")
}

#[derive(Clone, Debug)]
enum SimulatedOrder {
    LimitBuy { id: String, price: UDecimal, qty: UDecimal },
    LimitSell { id: String, price: UDecimal, qty: UDecimal },
    MarketBuy { id: String, qty: UDecimal },
    MarketSell { id: String, qty: UDecimal },
    Cancel { id: String },
}

fn generate_order_flow_mix(batch_size: usize) -> Vec<SimulatedOrder> {
    let mut orders = Vec::with_capacity(batch_size);
    let mut resting_ids = Vec::new();

    // 80% Limit, 15% Market, 5% Cancel
    // Example: For 1000 items -> 800 Limit (400 Buy / 400 Sell), 150 Market (75 Buy / 75 Sell), 50 Cancel
    let limit_count = (batch_size * 80) / 100;
    let market_count = (batch_size * 15) / 100;
    let cancel_count = batch_size - limit_count - market_count;

    for i in 0..limit_count {
        let id = next_order_id();
        if i % 2 == 0 {
            let offset = (i % 50) as i64 + 1;
            let price = UDecimal::new(dec!(65000) - Decimal::from(offset)).unwrap();
            let qty = UDecimal::new(dec!(1.0)).unwrap();
            resting_ids.push(id.clone());
            orders.push(SimulatedOrder::LimitBuy { id, price, qty });
        } else {
            let offset = (i % 50) as i64 + 1;
            let price = UDecimal::new(dec!(65000) + Decimal::from(offset)).unwrap();
            let qty = UDecimal::new(dec!(1.0)).unwrap();
            orders.push(SimulatedOrder::LimitSell { id, price, qty });
        }
    }

    for i in 0..market_count {
        let id = next_order_id();
        let qty = UDecimal::new(dec!(0.5)).unwrap();
        if i % 2 == 0 {
            orders.push(SimulatedOrder::MarketBuy { id, qty });
        } else {
            orders.push(SimulatedOrder::MarketSell { id, qty });
        }
    }

    for i in 0..cancel_count {
        if let Some(target_id) = resting_ids.get(i % resting_ids.len()) {
            orders.push(SimulatedOrder::Cancel { id: target_id.clone() });
        }
    }

    orders
}

fn bench_single_shard_sustained_ops(c: &mut Criterion) {
    let engine = ShardedEngine::new();
    engine
        .register_symbol("BTC-USDT", Some(UDecimal::new(dec!(65000)).unwrap()))
        .unwrap();

    let price = Some(UDecimal::new(dec!(64000)).unwrap());
    let qty = UDecimal::new(dec!(1.0)).unwrap();

    let mut group = c.benchmark_group("single_shard_sustained_ops");
    group.throughput(Throughput::Elements(1));
    group.bench_function("submit_order", |b| {
        b.iter(|| {
            let id = next_order_id();
            black_box(
                engine
                    .submit_order(
                        id,
                        "user-single".to_string(),
                        "BTC-USDT",
                        Side::Buy,
                        OrderType::Limit,
                        price,
                        qty,
                    )
                    .unwrap(),
            );
        });
    });
    group.finish();
}

fn bench_multi_shard_concurrent_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_shard_concurrent_scaling");
    let ops_per_shard = 200;

    for &num_shards in &[1, 2, 4, 8] {
        let engine = Arc::new(ShardedEngine::new());
        for s in 0..num_shards {
            engine
                .register_symbol(&format!("SYM-{s}"), Some(UDecimal::new(dec!(100)).unwrap()))
                .unwrap();
        }

        group.throughput(Throughput::Elements((num_shards * ops_per_shard) as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{num_shards}_shards")),
            &num_shards,
            |b, &n| {
                let price = Some(UDecimal::new(dec!(90)).unwrap());
                let qty = UDecimal::new(dec!(1.0)).unwrap();

                b.iter(|| {
                    std::thread::scope(|scope| {
                        for shard_idx in 0..n {
                            let eng = Arc::clone(&engine);
                            let sym = format!("SYM-{shard_idx}");
                            scope.spawn(move || {
                                for _ in 0..ops_per_shard {
                                    let id = next_order_id();
                                    black_box(
                                        eng.submit_order(
                                            id,
                                            "user-thread".to_string(),
                                            &sym,
                                            Side::Buy,
                                            OrderType::Limit,
                                            price,
                                            qty,
                                        )
                                        .unwrap(),
                                    );
                                }
                            });
                        }
                    });
                });
            },
        );
    }
    group.finish();
}

fn bench_realistic_order_flow_mix(c: &mut Criterion) {
    let batch_size = 1000;

    // Profile a sample 1,000-order execution to calculate and display matching domain metrics
    {
        let engine = ShardedEngine::new();
        engine
            .register_symbol("BTC-USDT", Some(UDecimal::new(dec!(65000)).unwrap()))
            .unwrap();
        let sample_stream = generate_order_flow_mix(batch_size);
        let mut orders_matched = 0usize;
        let mut trades_generated = 0usize;
        let mut volume_traded = Decimal::ZERO;

        for order in &sample_stream {
            match order {
                SimulatedOrder::LimitBuy { id, price, qty } => {
                    if let Ok(res) = engine.submit_order(
                        id.clone(),
                        "user-mix".to_string(),
                        "BTC-USDT",
                        Side::Buy,
                        OrderType::Limit,
                        Some(*price),
                        *qty,
                    ) {
                        if !res.trades.is_empty() {
                            orders_matched += 1;
                            trades_generated += res.trades.len();
                            for t in &res.trades {
                                volume_traded += t.quantity.get();
                            }
                        }
                    }
                }
                SimulatedOrder::LimitSell { id, price, qty } => {
                    if let Ok(res) = engine.submit_order(
                        id.clone(),
                        "user-mix".to_string(),
                        "BTC-USDT",
                        Side::Sell,
                        OrderType::Limit,
                        Some(*price),
                        *qty,
                    ) {
                        if !res.trades.is_empty() {
                            orders_matched += 1;
                            trades_generated += res.trades.len();
                            for t in &res.trades {
                                volume_traded += t.quantity.get();
                            }
                        }
                    }
                }
                SimulatedOrder::MarketBuy { id, qty } => {
                    if let Ok(res) = engine.submit_order(
                        id.clone(),
                        "user-mix".to_string(),
                        "BTC-USDT",
                        Side::Buy,
                        OrderType::Market,
                        None,
                        *qty,
                    ) {
                        if !res.trades.is_empty() {
                            orders_matched += 1;
                            trades_generated += res.trades.len();
                            for t in &res.trades {
                                volume_traded += t.quantity.get();
                            }
                        }
                    }
                }
                SimulatedOrder::MarketSell { id, qty } => {
                    if let Ok(res) = engine.submit_order(
                        id.clone(),
                        "user-mix".to_string(),
                        "BTC-USDT",
                        Side::Sell,
                        OrderType::Market,
                        None,
                        *qty,
                    ) {
                        if !res.trades.is_empty() {
                            orders_matched += 1;
                            trades_generated += res.trades.len();
                            for t in &res.trades {
                                volume_traded += t.quantity.get();
                            }
                        }
                    }
                }
                SimulatedOrder::Cancel { id } => {
                    let _ = engine.cancel_order("BTC-USDT", id, "user-mix");
                }
            }
        }

        let (bids, asks) = engine.get_depth("BTC-USDT", 100).unwrap();
        let resting_count =
            bids.iter().map(|b| b.order_count).sum::<usize>() + asks.iter().map(|a| a.order_count).sum::<usize>();

        println!(
            "\n>>> [Realistic Flow Domain Metrics] Total Orders: {} | Orders Matched: {} | Trades Generated: {} | Volume Traded: {:.2} BTC | Resting Orders on Book: {} <<<\n",
            batch_size, orders_matched, trades_generated, volume_traded, resting_count
        );
    }

    let mut group = c.benchmark_group("realistic_order_flow_mix");
    group.throughput(Throughput::Elements(batch_size as u64));

    group.bench_function("1000_mixed_orders", |b| {
        b.iter_batched(
            || {
                let engine = ShardedEngine::new();
                engine
                    .register_symbol("BTC-USDT", Some(UDecimal::new(dec!(65000)).unwrap()))
                    .unwrap();
                let stream = generate_order_flow_mix(batch_size);
                (engine, stream)
            },
            |(engine, stream)| {
                for order in &stream {
                    match order {
                        SimulatedOrder::LimitBuy { id, price, qty } => {
                            let _ = engine.submit_order(
                                id.clone(),
                                "user-mix".to_string(),
                                "BTC-USDT",
                                Side::Buy,
                                OrderType::Limit,
                                Some(*price),
                                *qty,
                            );
                        }
                        SimulatedOrder::LimitSell { id, price, qty } => {
                            let _ = engine.submit_order(
                                id.clone(),
                                "user-mix".to_string(),
                                "BTC-USDT",
                                Side::Sell,
                                OrderType::Limit,
                                Some(*price),
                                *qty,
                            );
                        }
                        SimulatedOrder::MarketBuy { id, qty } => {
                            let _ = engine.submit_order(
                                id.clone(),
                                "user-mix".to_string(),
                                "BTC-USDT",
                                Side::Buy,
                                OrderType::Market,
                                None,
                                *qty,
                            );
                        }
                        SimulatedOrder::MarketSell { id, qty } => {
                            let _ = engine.submit_order(
                                id.clone(),
                                "user-mix".to_string(),
                                "BTC-USDT",
                                Side::Sell,
                                OrderType::Market,
                                None,
                                *qty,
                            );
                        }
                        SimulatedOrder::Cancel { id } => {
                            let _ = engine.cancel_order("BTC-USDT", id, "user-mix");
                        }
                    }
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(
    name = engine_benches;
    config = Criterion::default()
        .sample_size(20)
        .measurement_time(std::time::Duration::from_secs(2));
    targets =
        bench_single_shard_sustained_ops,
        bench_multi_shard_concurrent_scaling,
        bench_realistic_order_flow_mix,
);
criterion_main!(engine_benches);
