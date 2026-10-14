#![allow(clippy::too_many_lines, clippy::assert_is_empty)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use matcher::orderbook::{OrderStatus, OrderType, Side, UDecimal};
use matcher::wal::SyncPolicy;
use matcher::ShardedEngine;
use rust_decimal_macros::dec;

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(test_name: &str) -> Self {
        let mut path = std::env::temp_dir();
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("matcher_crash_recovery_{test_name}_{unique_id}"));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[tokio::test]
async fn test_end_to_end_abrupt_crash_and_zero_divergence_recovery() {
    let dir_guard = TempDirGuard::new("e2e_crash");

    // Phase 1: Initialize Engine with WAL persistence (SyncPolicy::Always ensures flushed records)
    {
        let engine = ShardedEngine::with_wal(&dir_guard.path, SyncPolicy::Always);

        // Register two distinct symbol shards concurrently
        let symbols = vec![
            ("BTC-USDT".to_string(), Some(UDecimal::new(dec!(65000.0)).unwrap())),
            ("ETH-USDT".to_string(), Some(UDecimal::new(dec!(3500.0)).unwrap())),
        ];
        engine
            .register_symbols_parallel(symbols)
            .await
            .expect("register symbols parallel");

        // --- BTC-USDT Transactions ---
        // 1. Alice places resting limit sell: 2.0 BTC @ 65100.0
        let res_alice = engine
            .submit_order(
                "btc-alice-01".into(),
                "alice".into(),
                "BTC-USDT",
                Side::Sell,
                OrderType::Limit,
                Some(UDecimal::new(dec!(65100.0)).unwrap()),
                UDecimal::new(dec!(2.0)).unwrap(),
            )
            .expect("Alice sell");
        assert_eq!(res_alice.order_id, "btc-alice-01");
        assert!(res_alice.trades.is_empty());

        // 2. Bob places resting limit buy: 1.0 BTC @ 64900.0
        let res_bob = engine
            .submit_order(
                "btc-bob-01".into(),
                "bob".into(),
                "BTC-USDT",
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(64900.0)).unwrap()),
                UDecimal::new(dec!(1.0)).unwrap(),
            )
            .expect("Bob buy");
        assert_eq!(res_bob.order_id, "btc-bob-01");
        assert!(res_bob.trades.is_empty());

        // 3. Charlie places crossing limit buy: 0.5 BTC @ 65100.0 -> partial match against Alice
        let res_charlie = engine
            .submit_order(
                "btc-charlie-01".into(),
                "charlie".into(),
                "BTC-USDT",
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(65100.0)).unwrap()),
                UDecimal::new(dec!(0.5)).unwrap(),
            )
            .expect("Charlie buy");
        assert_eq!(res_charlie.trades.len(), 1);
        assert_eq!(res_charlie.trades[0].quantity, UDecimal::new(dec!(0.5)).unwrap());
        assert_eq!(res_charlie.trades[0].price, UDecimal::new(dec!(65100.0)).unwrap());

        // 4. Dave places limit buy: 0.75 BTC @ 64800.0
        engine
            .submit_order(
                "btc-dave-01".into(),
                "dave".into(),
                "BTC-USDT",
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(64800.0)).unwrap()),
                UDecimal::new(dec!(0.75)).unwrap(),
            )
            .expect("Dave buy");

        // 5. Dave cancels the order
        let cancel_res = engine
            .cancel_order("BTC-USDT", "btc-dave-01", "dave")
            .expect("Dave cancel");
        assert_eq!(cancel_res.status(), OrderStatus::Cancelled);

        // --- ETH-USDT Transactions ---
        // 1. Eve places resting limit sell: 10.0 ETH @ 3500.0
        engine
            .submit_order(
                "eth-eve-01".into(),
                "eve".into(),
                "ETH-USDT",
                Side::Sell,
                OrderType::Limit,
                Some(UDecimal::new(dec!(3500.0)).unwrap()),
                UDecimal::new(dec!(10.0)).unwrap(),
            )
            .expect("Eve sell");

        // 2. Frank places market buy: 4.0 ETH -> executes against Eve
        let res_frank = engine
            .submit_order(
                "eth-frank-01".into(),
                "frank".into(),
                "ETH-USDT",
                Side::Buy,
                OrderType::Market,
                None,
                UDecimal::new(dec!(4.0)).unwrap(),
            )
            .expect("Frank market buy");
        assert_eq!(res_frank.trades.len(), 1);
        assert_eq!(res_frank.trades[0].quantity, UDecimal::new(dec!(4.0)).unwrap());

        // Verify pre-crash state
        let btc_stats = engine.get_stats("BTC-USDT").unwrap();
        assert_eq!(btc_stats.total_trades_count, 1);
        assert_eq!(btc_stats.total_volume_traded, UDecimal::new(dec!(0.5)).unwrap());
        assert_eq!(btc_stats.active_sell_volume, UDecimal::new(dec!(1.5)).unwrap());
        assert_eq!(btc_stats.active_buy_volume, UDecimal::new(dec!(1.0)).unwrap());

        let eth_stats = engine.get_stats("ETH-USDT").unwrap();
        assert_eq!(eth_stats.total_trades_count, 1);
        assert_eq!(eth_stats.total_volume_traded, UDecimal::new(dec!(4.0)).unwrap());
        assert_eq!(eth_stats.active_sell_volume, UDecimal::new(dec!(6.0)).unwrap());

        // --- SIMULATE ABRUPT CRASH ---
        // Drop engine abruptly WITHOUT calling shutdown_and_flush() or checkpoint_and_prune()!
        // Also simulate an interrupted/torn trailing write on the BTC-USDT WAL file.
        let btc_wal_file = std::fs::read_dir(&dir_guard.path)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                let name = p.file_name().unwrap().to_string_lossy();
                name.starts_with("BTC-USDT") && name.ends_with(".wal")
            })
            .expect("BTC WAL file should exist");

        let mut wal_handle = OpenOptions::new()
            .append(true)
            .open(&btc_wal_file)
            .expect("open WAL for appending torn bytes");

        // Append 13 corrupted / uncommitted bytes simulating torn power cut
        wal_handle
            .write_all(b"\xFE\xED\xFA\xCE\x00\x00\x01\x02\x03\x04\x05\x06\x07")
            .expect("append torn bytes");
        wal_handle.sync_all().expect("sync torn bytes");
        drop(wal_handle);

        // Engine scope terminates abruptly here:
        drop(engine);
    }

    // Phase 2: Recovery Verification (New Engine Instance Startup)
    let recovered_engine = Arc::new(ShardedEngine::with_wal(&dir_guard.path, SyncPolicy::Always));

    let symbols = vec![
        ("BTC-USDT".to_string(), Some(UDecimal::new(dec!(65000.0)).unwrap())),
        ("ETH-USDT".to_string(), Some(UDecimal::new(dec!(3500.0)).unwrap())),
    ];

    // Replay WAL & repair torn tail
    recovered_engine
        .register_symbols_parallel(symbols)
        .await
        .expect("recover parallel shards");

    // Phase 3: Assert ZERO State Divergence
    // 1. BTC-USDT stats & depth
    let btc_stats = recovered_engine.get_stats("BTC-USDT").expect("btc stats");
    assert_eq!(btc_stats.total_trades_count, 1, "BTC total trades must match pre-crash");
    assert_eq!(
        btc_stats.total_volume_traded,
        UDecimal::new(dec!(0.5)).unwrap(),
        "BTC traded volume must match pre-crash"
    );
    assert_eq!(
        btc_stats.active_sell_volume,
        UDecimal::new(dec!(1.5)).unwrap(),
        "BTC active sell volume must reflect Alice's partial fill"
    );
    assert_eq!(
        btc_stats.active_buy_volume,
        UDecimal::new(dec!(1.0)).unwrap(),
        "BTC active buy volume must reflect Bob's resting bid (Dave was cancelled)"
    );

    let (btc_bids, btc_asks) = recovered_engine.get_depth("BTC-USDT", 5).expect("btc depth");
    assert_eq!(btc_bids.len(), 1, "Only Bob's bid level exists");
    assert_eq!(btc_bids[0].price, UDecimal::new(dec!(64900.0)).unwrap());
    assert_eq!(btc_bids[0].quantity, UDecimal::new(dec!(1.0)).unwrap());
    assert_eq!(btc_bids[0].order_count, 1);

    assert_eq!(btc_asks.len(), 1, "Only Alice's ask level exists");
    assert_eq!(btc_asks[0].price, UDecimal::new(dec!(65100.0)).unwrap());
    assert_eq!(btc_asks[0].quantity, UDecimal::new(dec!(1.5)).unwrap());
    assert_eq!(btc_asks[0].order_count, 1);

    // 2. ETH-USDT stats & depth
    let eth_stats = recovered_engine.get_stats("ETH-USDT").expect("eth stats");
    assert_eq!(eth_stats.total_trades_count, 1, "ETH total trades must match pre-crash");
    assert_eq!(
        eth_stats.total_volume_traded,
        UDecimal::new(dec!(4.0)).unwrap(),
        "ETH traded volume must match pre-crash"
    );
    assert_eq!(
        eth_stats.active_sell_volume,
        UDecimal::new(dec!(6.0)).unwrap(),
        "ETH active sell volume must reflect Eve's partial fill (10 - 4 = 6)"
    );
    assert_eq!(eth_stats.active_buy_volume, UDecimal::ZERO, "ETH has no resting bids");

    let (eth_bids, eth_asks) = recovered_engine.get_depth("ETH-USDT", 5).expect("eth depth");
    assert!(eth_bids.is_empty(), "ETH bids should be empty");
    assert_eq!(eth_asks.len(), 1, "Only Eve's ask level exists");
    assert_eq!(eth_asks[0].price, UDecimal::new(dec!(3500.0)).unwrap());
    assert_eq!(eth_asks[0].quantity, UDecimal::new(dec!(6.0)).unwrap());
    assert_eq!(eth_asks[0].order_count, 1);

    // 3. Resting Order State Checks (order_index retains active resting orders)
    let alice_ord = recovered_engine
        .get_order("BTC-USDT", "btc-alice-01")
        .expect("query order")
        .expect("Alice order must exist as resting order");
    assert_eq!(alice_ord.status(), OrderStatus::PartiallyExecuted);
    assert_eq!(alice_ord.filled_quantity(), UDecimal::new(dec!(0.5)).unwrap());

    let bob_ord = recovered_engine
        .get_order("BTC-USDT", "btc-bob-01")
        .expect("query order")
        .expect("Bob order must exist as resting order");
    assert_eq!(bob_ord.status(), OrderStatus::Open);
    assert_eq!(bob_ord.filled_quantity(), UDecimal::ZERO);

    let charlie_ord = recovered_engine
        .get_order("BTC-USDT", "btc-charlie-01")
        .expect("query order");
    assert!(
        charlie_ord.is_none(),
        "Fully filled taker orders are purged from active order_index"
    );

    let dave_ord = recovered_engine
        .get_order("BTC-USDT", "btc-dave-01")
        .expect("query order");
    assert!(
        dave_ord.is_none(),
        "Cancelled orders are purged from active order_index"
    );

    let eve_ord = recovered_engine
        .get_order("ETH-USDT", "eth-eve-01")
        .expect("query order")
        .expect("Eve order must exist as resting order");
    assert_eq!(eve_ord.status(), OrderStatus::PartiallyExecuted);
    assert_eq!(eve_ord.filled_quantity(), UDecimal::new(dec!(4.0)).unwrap());

    // Phase 4: Operational Integrity After Recovery
    // Submit an order against the recovered book to ensure live matching functions flawlessly
    let post_recovery_trade = recovered_engine
        .submit_order(
            "btc-george-01".into(),
            "george".into(),
            "BTC-USDT",
            Side::Buy,
            OrderType::Market,
            None,
            UDecimal::new(dec!(1.0)).unwrap(),
        )
        .expect("George market buy post-crash");

    assert_eq!(post_recovery_trade.trades.len(), 1);
    assert_eq!(post_recovery_trade.trades[0].maker_order_id, "btc-alice-01");
    assert_eq!(
        post_recovery_trade.trades[0].quantity,
        UDecimal::new(dec!(1.0)).unwrap()
    );
    assert_eq!(
        post_recovery_trade.trades[0].price,
        UDecimal::new(dec!(65100.0)).unwrap()
    );

    let btc_stats_final = recovered_engine.get_stats("BTC-USDT").unwrap();
    assert_eq!(btc_stats_final.total_trades_count, 2);
    assert_eq!(btc_stats_final.total_volume_traded, UDecimal::new(dec!(1.5)).unwrap());
    assert_eq!(btc_stats_final.active_sell_volume, UDecimal::new(dec!(0.5)).unwrap());
}
