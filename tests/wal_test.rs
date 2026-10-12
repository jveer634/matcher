use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use matcher::orderbook::{OrderType, Side, UDecimal};
use matcher::wal::{
    decode_decimal, decode_fixed_str, decode_order_type, decode_side, SyncPolicy, WalPayload, WalReadError, WalReader,
    WalWriter,
};
use rust_decimal_macros::dec;

struct TempWalGuard {
    path: PathBuf,
}

impl TempWalGuard {
    fn new(test_name: &str) -> Self {
        let mut path = std::env::temp_dir();
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("matcher_wal_{test_name}_{unique_id}.wal"));
        Self { path }
    }
}

impl Drop for TempWalGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn test_wal_writer_and_reader_roundtrip() {
    let guard = TempWalGuard::new("roundtrip");
    let mut writer = WalWriter::create(&guard.path, SyncPolicy::None).expect("create WAL writer");

    let price = Some(UDecimal::new(dec!(65500.25)).unwrap());
    let qty = UDecimal::new(dec!(1.5)).unwrap();

    let seq1 = writer
        .append_submit_order(
            "ord-101",
            "user-alice",
            Side::Buy,
            OrderType::Limit,
            price,
            qty,
            1_700_000_001_000,
        )
        .expect("append submit order");
    assert_eq!(seq1, 1);

    let seq2 = writer
        .append_cancel_order("ord-101", "user-alice", 1_700_000_002_000)
        .expect("append cancel order");
    assert_eq!(seq2, 2);

    let trigger = Some(UDecimal::new(dec!(65000.00)).unwrap());
    let seq3 = writer
        .append_submit_order_v2(
            "ord-102",
            "user-bob",
            Side::Sell,
            OrderType::Limit,
            price,
            qty,
            1, // GTC
            trigger,
            1_700_000_003_000,
        )
        .expect("append submit order v2");
    assert_eq!(seq3, 3);

    let seq4 = writer
        .append_snapshot_marker(1, 10, 1_700_000_004_000)
        .expect("append snapshot marker");
    assert_eq!(seq4, 4);

    writer.flush().expect("flush writer");

    // Read and verify with WalReader
    let mut reader = WalReader::open(&guard.path).expect("open WAL reader");
    let (records, err) = reader.read_all();
    assert!(err.is_none(), "Unexpected error during read_all: {err:?}");
    assert_eq!(records.len(), 4);

    // Record 1: SubmitOrder V1
    assert_eq!(records[0].sequence, 1);
    if let WalPayload::SubmitOrder(sbe) = &records[0].payload {
        assert_eq!(decode_fixed_str(&sbe.order_id).unwrap(), "ord-101");
        assert_eq!(decode_fixed_str(&sbe.user_id).unwrap(), "user-alice");
        assert_eq!(decode_side(sbe.side).unwrap(), Side::Buy);
        assert_eq!(decode_order_type(sbe.order_type).unwrap(), OrderType::Limit);
        assert_eq!(decode_decimal(sbe.price_mantissa, sbe.price_scale).unwrap(), price);
        assert_eq!(decode_decimal(sbe.qty_mantissa, sbe.qty_scale).unwrap(), Some(qty));
    } else {
        panic!("Expected SubmitOrder payload");
    }

    // Record 2: CancelOrder
    assert_eq!(records[1].sequence, 2);
    if let WalPayload::CancelOrder(sbe) = &records[1].payload {
        assert_eq!(decode_fixed_str(&sbe.order_id).unwrap(), "ord-101");
        assert_eq!(decode_fixed_str(&sbe.user_id).unwrap(), "user-alice");
    } else {
        panic!("Expected CancelOrder payload");
    }

    // Record 3: SubmitOrder V2
    assert_eq!(records[2].sequence, 3);
    if let WalPayload::SubmitOrder(sbe) = &records[2].payload {
        assert_eq!(decode_fixed_str(&sbe.order_id).unwrap(), "ord-102");
        assert_eq!(decode_fixed_str(&sbe.user_id).unwrap(), "user-bob");
        assert_eq!(sbe.time_in_force, 1);
        assert_eq!(
            decode_decimal(sbe.trigger_price_mantissa, sbe.trigger_price_scale).unwrap(),
            trigger
        );
    } else {
        panic!("Expected SubmitOrder payload");
    }

    // Record 4: SnapshotMarker
    assert_eq!(records[3].sequence, 4);
    if let WalPayload::SnapshotMarker(sbe) = &records[3].payload {
        assert_eq!(sbe.snapshot_seq, 1);
        assert_eq!(sbe.open_orders_count, 10);
    } else {
        panic!("Expected SnapshotMarker payload");
    }
}

#[test]
fn test_wal_open_or_create_resumption() {
    let guard = TempWalGuard::new("resume");
    let qty = UDecimal::new(dec!(2.0)).unwrap();

    {
        let mut writer = WalWriter::create(&guard.path, SyncPolicy::None).unwrap();
        writer
            .append_submit_order("ord-1", "user-1", Side::Buy, OrderType::Market, None, qty, 100)
            .unwrap();
        writer
            .append_submit_order("ord-2", "user-2", Side::Sell, OrderType::Market, None, qty, 101)
            .unwrap();
        writer.flush().unwrap();
    }

    // Reopen and continue appending
    {
        let mut writer = WalWriter::open_or_create(&guard.path, SyncPolicy::None).unwrap();
        assert_eq!(writer.next_sequence(), 3);

        let seq = writer
            .append_submit_order("ord-3", "user-3", Side::Buy, OrderType::Market, None, qty, 102)
            .unwrap();
        assert_eq!(seq, 3);
        writer.flush().unwrap();
    }

    // Verify all 3 records persisted in sequential order
    let mut reader = WalReader::open(&guard.path).unwrap();
    let (records, err) = reader.read_all();
    assert!(err.is_none());
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].sequence, 1);
    assert_eq!(records[1].sequence, 2);
    assert_eq!(records[2].sequence, 3);
}

#[test]
fn test_wal_torn_write_tail_truncation_and_repair() {
    let guard = TempWalGuard::new("torn_write");
    let qty = UDecimal::new(dec!(1.0)).unwrap();

    // 1. Write 2 valid records
    {
        let mut writer = WalWriter::create(&guard.path, SyncPolicy::None).unwrap();
        writer
            .append_submit_order("ord-1", "user-1", Side::Buy, OrderType::Market, None, qty, 100)
            .unwrap();
        writer
            .append_submit_order("ord-2", "user-2", Side::Sell, OrderType::Market, None, qty, 101)
            .unwrap();
        writer.flush().unwrap();
    }

    // 2. Corrupt with a trailing partial write (e.g. power failure wrote only 10 bytes of a header)
    {
        let mut file = OpenOptions::new().append(true).open(&guard.path).unwrap();
        let partial_bytes = [0xAA; 10];
        file.write_all(&partial_bytes).unwrap();
        file.sync_all().unwrap();
    }

    // 3. Reader should read 2 records and then detect TornWrite
    {
        let mut reader = WalReader::open(&guard.path).unwrap();
        let (records, err) = reader.read_all();
        assert_eq!(records.len(), 2);
        assert_eq!(
            err,
            Some(WalReadError::TornWrite {
                bytes_available: 10,
                bytes_expected: 24,
            })
        );
    }

    // 4. open_or_create should automatically repair/truncate to last valid offset and resume
    {
        let mut writer = WalWriter::open_or_create(&guard.path, SyncPolicy::None).unwrap();
        assert_eq!(writer.next_sequence(), 3);

        let seq = writer
            .append_submit_order("ord-3", "user-3", Side::Buy, OrderType::Market, None, qty, 102)
            .unwrap();
        assert_eq!(seq, 3);
        writer.flush().unwrap();
    }

    // 5. Reader should now read all 3 records cleanly with no error!
    {
        let mut reader = WalReader::open(&guard.path).unwrap();
        let (records, err) = reader.read_all();
        assert!(err.is_none());
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].sequence, 1);
        assert_eq!(records[1].sequence, 2);
        assert_eq!(records[2].sequence, 3);
    }
}

#[test]
fn test_wal_corrupted_crc_detection() {
    let guard = TempWalGuard::new("corrupted_crc");
    let qty = UDecimal::new(dec!(1.0)).unwrap();

    {
        let mut writer = WalWriter::create(&guard.path, SyncPolicy::None).unwrap();
        writer
            .append_submit_order("ord-1", "user-1", Side::Buy, OrderType::Market, None, qty, 100)
            .unwrap();
        writer.flush().unwrap();
    }

    // Tamper with a payload byte (offset 30, inside the payload)
    {
        let mut bytes = std::fs::read(&guard.path).unwrap();
        assert!(bytes.len() > 30);
        bytes[30] ^= 0xFF; // flip bits
        std::fs::write(&guard.path, &bytes).unwrap();
    }

    // Reader must catch CRC corruption
    let mut reader = WalReader::open(&guard.path).unwrap();
    let res = reader.next_record();
    assert!(
        matches!(res, Err(WalReadError::CorruptedCrc { sequence: 1, .. })),
        "Expected CorruptedCrc, got: {res:?}"
    );
}

#[test]
fn test_wal_iterator_streaming() {
    let guard = TempWalGuard::new("iterator");
    let qty = UDecimal::new(dec!(1.0)).unwrap();

    let mut writer = WalWriter::create(&guard.path, SyncPolicy::None).unwrap();
    for i in 1..=5 {
        writer
            .append_submit_order(
                &format!("ord-{i}"),
                "user-1",
                Side::Buy,
                OrderType::Market,
                None,
                qty,
                100 + i,
            )
            .unwrap();
    }
    writer.flush().unwrap();

    let reader = WalReader::open(&guard.path).unwrap();
    let records: Vec<_> = reader.collect::<Result<Vec<_>, _>>().expect("collect records");
    assert_eq!(records.len(), 5);
    for (idx, record) in records.iter().enumerate() {
        assert_eq!(record.sequence, (idx + 1) as u64);
    }
}

#[test]
fn test_wal_sync_policy_always() {
    let guard = TempWalGuard::new("sync_always");
    let qty = UDecimal::new(dec!(1.0)).unwrap();

    let mut writer = WalWriter::create(&guard.path, SyncPolicy::Always).unwrap();
    writer
        .append_submit_order("ord-sync", "user-1", Side::Buy, OrderType::Market, None, qty, 500)
        .unwrap();

    // Without explicit flush, data should already be committed to disk
    let mut reader = WalReader::open(&guard.path).unwrap();
    let record = reader.next_record().unwrap().expect("record must exist");
    assert_eq!(record.sequence, 1);
}

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
        path.push(format!("matcher_wal_dir_{test_name}_{unique_id}"));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_wal_pruner_config_from_env() {
    use matcher::wal::{
        WalPrunerConfig, DEFAULT_PRUNE_INTERVAL_SECS, ENV_PRUNE_INTERVAL_MINS, ENV_PRUNE_INTERVAL_SECS,
    };
    use std::time::Duration;

    let dir = PathBuf::from("/tmp/wal_test");

    // 1. Default (no env set) -> 10 minutes (600 seconds)
    std::env::remove_var(ENV_PRUNE_INTERVAL_MINS);
    std::env::remove_var(ENV_PRUNE_INTERVAL_SECS);
    let cfg = WalPrunerConfig::from_env(&dir);
    assert_eq!(cfg.interval, Duration::from_secs(DEFAULT_PRUNE_INTERVAL_SECS));
    assert_eq!(cfg.interval, Duration::from_secs(600));

    // 2. Set interval in minutes
    std::env::set_var(ENV_PRUNE_INTERVAL_MINS, "5");
    let cfg_mins = WalPrunerConfig::from_env(&dir);
    assert_eq!(cfg_mins.interval, Duration::from_secs(300));

    // 3. Set interval in seconds (takes precedence)
    std::env::set_var(ENV_PRUNE_INTERVAL_SECS, "45");
    let cfg_secs = WalPrunerConfig::from_env(&dir);
    assert_eq!(cfg_secs.interval, Duration::from_secs(45));

    // Clean up
    std::env::remove_var(ENV_PRUNE_INTERVAL_MINS);
    std::env::remove_var(ENV_PRUNE_INTERVAL_SECS);
}

#[test]
fn test_wal_pruner_safety_no_prune_without_snapshot() {
    use matcher::wal::{WalPruner, WalPrunerConfig};

    let dir_guard = TempDirGuard::new("no_snapshot");

    // Create 3 WAL segment dummy files without any snapshot
    std::fs::write(dir_guard.path.join("segment_0000000010.wal"), b"dummy-frame-1").unwrap();
    std::fs::write(dir_guard.path.join("segment_0000000020.wal"), b"dummy-frame-2").unwrap();
    std::fs::write(dir_guard.path.join("segment_0000000030.wal"), b"dummy-frame-3").unwrap();

    let pruner = WalPruner::new(WalPrunerConfig::from_env(&dir_guard.path));
    let stats = pruner.prune_once().expect("prune_once");

    // Invariant: without a snapshot, no WAL segments should ever be pruned
    assert_eq!(stats.segments_pruned, 0);
    assert_eq!(stats.snapshots_pruned, 0);
    assert!(dir_guard.path.join("segment_0000000010.wal").exists());
    assert!(dir_guard.path.join("segment_0000000020.wal").exists());
    assert!(dir_guard.path.join("segment_0000000030.wal").exists());
}

#[test]
fn test_wal_pruner_obsolete_segments_and_snapshots() {
    use matcher::wal::{WalPruner, WalPrunerConfig};

    let dir_guard = TempDirGuard::new("obsolete_pruning");

    // Write segments with sequences 100, 200, 300
    std::fs::write(dir_guard.path.join("segment_0000000100.wal"), b"wal-100").unwrap();
    std::fs::write(dir_guard.path.join("segment_0000000200.wal"), b"wal-200").unwrap();
    std::fs::write(dir_guard.path.join("segment_0000000300.wal"), b"wal-300").unwrap();

    // Write snapshots at 50, 150, 250
    std::fs::write(dir_guard.path.join("snapshot_0000000050.snap"), b"snap-50").unwrap();
    std::fs::write(dir_guard.path.join("snapshot_0000000150.snap"), b"snap-150").unwrap();
    std::fs::write(dir_guard.path.join("snapshot_0000000250.snap"), b"snap-250").unwrap();

    // Config: retain 2 snapshots
    let pruner = WalPruner::new(WalPrunerConfig::from_env(&dir_guard.path).with_retain_snapshots(2));
    let stats = pruner.prune_once().expect("prune_once");

    // Segments with max seq < 250 (100 and 200) should be pruned
    assert_eq!(stats.segments_pruned, 2);
    // Snapshots: 50 should be pruned, 150 and 250 kept
    assert_eq!(stats.snapshots_pruned, 1);
    assert!(stats.bytes_freed > 0);

    // Verify disk state
    assert!(!dir_guard.path.join("segment_0000000100.wal").exists());
    assert!(!dir_guard.path.join("segment_0000000200.wal").exists());
    assert!(dir_guard.path.join("segment_0000000300.wal").exists()); // Active segment kept!

    assert!(!dir_guard.path.join("snapshot_0000000050.snap").exists());
    assert!(dir_guard.path.join("snapshot_0000000150.snap").exists());
    assert!(dir_guard.path.join("snapshot_0000000250.snap").exists());
}

#[tokio::test]
async fn test_wal_pruner_background_spawn_and_stop() {
    use matcher::wal::{WalPruner, WalPrunerConfig};
    use std::time::Duration;

    let dir_guard = TempDirGuard::new("bg_runner");
    let pruner = WalPruner::new(WalPrunerConfig::from_env(&dir_guard.path).with_interval(Duration::from_millis(20)));

    let handle = pruner.spawn_background();

    // Allow background task to initialize
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Gracefully stop the task
    handle.stop().await;
}

#[test]
fn test_snapshot_roundtrip_empty_and_populated() {
    use matcher::orderbook::{Order, OrderStatus, OrderType, Side, UDecimal};
    use matcher::wal::{BookSnapshot, SnapshotReader, SnapshotWriter};
    use rust_decimal_macros::dec;

    let dir_guard = TempDirGuard::new("snapshot_roundtrip");
    let snap_path = dir_guard.path.join(SnapshotWriter::snapshot_filename("BTC-USDT", 42));

    let o1 = Order::reconstruct(
        "ord-101".into(),
        "user-alice".into(),
        "BTC-USDT".into(),
        Side::Buy,
        OrderType::Limit,
        UDecimal::new(dec!(2.5)).unwrap(),
        UDecimal::new(dec!(5.0)).unwrap(),
        Some(UDecimal::new(dec!(65100.50)).unwrap()),
        OrderStatus::PartiallyExecuted,
        1_700_000_100_000,
    );

    let o2 = Order::reconstruct(
        "ord-102".into(),
        "user-bob".into(),
        "BTC-USDT".into(),
        Side::Sell,
        OrderType::Limit,
        UDecimal::new(dec!(1.0)).unwrap(),
        UDecimal::new(dec!(1.0)).unwrap(),
        Some(UDecimal::new(dec!(65200.00)).unwrap()),
        OrderStatus::Open,
        1_700_000_200_000,
    );

    let snapshot = BookSnapshot {
        symbol: "BTC-USDT".into(),
        sequence: 42,
        timestamp_ns: 1_700_000_000_123_456_789,
        last_traded_price: Some(UDecimal::new(dec!(65150.00)).unwrap()),
        total_volume_traded: UDecimal::new(dec!(125.75)).unwrap(),
        total_trades_count: 18,
        open_orders: vec![o1, o2],
    };

    // 1. Write snapshot atomically
    SnapshotWriter::write_snapshot(&snap_path, &snapshot).expect("write snapshot");
    assert!(snap_path.exists());

    // 2. Read back and verify
    let loaded = SnapshotReader::read_snapshot(&snap_path).expect("read snapshot");
    assert_eq!(loaded.symbol, "BTC-USDT");
    assert_eq!(loaded.sequence, 42);
    assert_eq!(loaded.timestamp_ns, 1_700_000_000_123_456_789);
    assert_eq!(loaded.last_traded_price, Some(UDecimal::new(dec!(65150.00)).unwrap()));
    assert_eq!(loaded.total_volume_traded, UDecimal::new(dec!(125.75)).unwrap());
    assert_eq!(loaded.total_trades_count, 18);
    assert_eq!(loaded.open_orders.len(), 2);

    // Verify reconstructed orders directly
    assert_eq!(loaded.open_orders[0].id(), "ord-101");
    assert_eq!(loaded.open_orders[0].quantity(), UDecimal::new(dec!(2.5)).unwrap());
    assert_eq!(loaded.open_orders[0].remaining(), UDecimal::new(dec!(5.0)).unwrap());
    assert_eq!(loaded.open_orders[0].status(), OrderStatus::PartiallyExecuted);

    assert_eq!(loaded.open_orders[1].id(), "ord-102");
    assert_eq!(loaded.open_orders[1].quantity(), UDecimal::new(dec!(1.0)).unwrap());
    assert_eq!(loaded.open_orders[1].status(), OrderStatus::Open);
}

#[test]
fn test_snapshot_crc_corruption_rejection() {
    use matcher::wal::{BookSnapshot, SnapshotReader, SnapshotWriter};

    let dir_guard = TempDirGuard::new("snap_corrupt");
    let snap_path = dir_guard.path.join(SnapshotWriter::snapshot_filename("SOL-USDT", 1));

    let snapshot = BookSnapshot {
        symbol: "SOL-USDT".into(),
        sequence: 1,
        timestamp_ns: 100,
        last_traded_price: None,
        total_volume_traded: matcher::orderbook::UDecimal::ZERO,
        total_trades_count: 0,
        open_orders: Vec::new(),
    };

    SnapshotWriter::write_snapshot(&snap_path, &snapshot).expect("write snapshot");

    // Corrupt a byte in the payload
    let mut bytes = std::fs::read(&snap_path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    std::fs::write(&snap_path, &bytes).unwrap();

    let err = SnapshotReader::read_snapshot(&snap_path).expect_err("must fail CRC check");
    assert!(err.contains("Snapshot CRC32 mismatch"));
}

#[test]
fn test_snapshot_and_pruner_interoperability() {
    use matcher::wal::{BookSnapshot, SnapshotWriter, WalPruner, WalPrunerConfig};

    let dir_guard = TempDirGuard::new("snap_pruner_interop");

    // Write 3 snapshots: sequences 100, 200, 300
    for seq in [100, 200, 300] {
        let snap_path = dir_guard.path.join(SnapshotWriter::snapshot_filename("ETH-USDT", seq));
        let snap = BookSnapshot {
            symbol: "ETH-USDT".into(),
            sequence: seq,
            timestamp_ns: 1_000 + seq,
            last_traded_price: None,
            total_volume_traded: matcher::orderbook::UDecimal::ZERO,
            total_trades_count: 0,
            open_orders: Vec::new(),
        };
        SnapshotWriter::write_snapshot(&snap_path, &snap).unwrap();
    }

    // Write segments with sequences 150, 250, 350
    std::fs::write(dir_guard.path.join("ETH-USDT_segment_0000000150.wal"), b"wal-150").unwrap();
    std::fs::write(dir_guard.path.join("ETH-USDT_segment_0000000250.wal"), b"wal-250").unwrap();
    std::fs::write(dir_guard.path.join("ETH-USDT_segment_0000000350.wal"), b"wal-350").unwrap();

    // Prune with retain_snapshots: 2
    let pruner = WalPruner::new(WalPrunerConfig::from_env(&dir_guard.path).with_retain_snapshots(2));
    let stats = pruner.prune_once().expect("prune_once");

    // Snapshots: seq 100 pruned, 200 and 300 retained
    assert_eq!(stats.snapshots_pruned, 1);
    // WAL Segments: 150 and 250 (< 300) pruned, 350 (active) retained
    assert_eq!(stats.segments_pruned, 2);

    assert!(!dir_guard.path.join("ETH-USDT_snapshot_0000000100.snap").exists());
    assert!(dir_guard.path.join("ETH-USDT_snapshot_0000000200.snap").exists());
    assert!(dir_guard.path.join("ETH-USDT_snapshot_0000000300.snap").exists());

    assert!(!dir_guard.path.join("ETH-USDT_segment_0000000150.wal").exists());
    assert!(!dir_guard.path.join("ETH-USDT_segment_0000000250.wal").exists());
    assert!(dir_guard.path.join("ETH-USDT_segment_0000000350.wal").exists());
}

#[tokio::test]
async fn test_sharded_engine_checkpoint_and_recovery() {
    use matcher::orderbook::{OrderType, Side, UDecimal};
    use matcher::wal::{SyncPolicy, WalPruner, WalPrunerConfig};
    use matcher::ShardedEngine;
    use rust_decimal_macros::dec;

    let dir_guard = TempDirGuard::new("engine_snap_recovery");

    // Phase 1: Run Engine with WAL persistence, submit orders, and snapshot
    {
        let engine = ShardedEngine::with_wal(&dir_guard.path, SyncPolicy::None);
        engine
            .register_symbol("BTC-USDT", Some(UDecimal::new(dec!(60000.0)).unwrap()))
            .expect("register BTC");
        engine
            .register_symbol("ETH-USDT", Some(UDecimal::new(dec!(3000.0)).unwrap()))
            .expect("register ETH");

        // 1. Submit initial resting orders
        engine
            .submit_order(
                "btc-sell-1".into(),
                "alice".into(),
                "BTC-USDT",
                Side::Sell,
                OrderType::Limit,
                Some(UDecimal::new(dec!(60000.0)).unwrap()),
                UDecimal::new(dec!(2.0)).unwrap(),
            )
            .expect("btc sell");

        engine
            .submit_order(
                "btc-buy-1".into(),
                "bob".into(),
                "BTC-USDT",
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(59000.0)).unwrap()),
                UDecimal::new(dec!(1.0)).unwrap(),
            )
            .expect("btc buy");

        // ETH execution: sell 10, buy 5 -> 5 matched, 5 resting
        engine
            .submit_order(
                "eth-sell-1".into(),
                "carol".into(),
                "ETH-USDT",
                Side::Sell,
                OrderType::Limit,
                Some(UDecimal::new(dec!(3000.0)).unwrap()),
                UDecimal::new(dec!(10.0)).unwrap(),
            )
            .expect("eth sell");

        let eth_match = engine
            .submit_order(
                "eth-buy-1".into(),
                "dave".into(),
                "ETH-USDT",
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(3000.0)).unwrap()),
                UDecimal::new(dec!(5.0)).unwrap(),
            )
            .expect("eth buy");
        assert_eq!(eth_match.trades.len(), 1);

        // 2. Parallel snapshot creation across both shards
        let snap_count = engine
            .create_snapshots(&dir_guard.path)
            .await
            .expect("create snapshots");
        assert_eq!(snap_count, 2);

        // 3. Post-snapshot delta orders (to verify replay on top of snapshot)
        // BTC: market buy 0.5 against alice's 60000 ask
        let btc_delta_match = engine
            .submit_order(
                "btc-buy-2".into(),
                "charlie".into(),
                "BTC-USDT",
                Side::Buy,
                OrderType::Market,
                None,
                UDecimal::new(dec!(0.5)).unwrap(),
            )
            .expect("btc market buy");
        assert_eq!(btc_delta_match.trades.len(), 1);

        // ETH: cancel carol's remaining 5.0 sell order
        engine
            .cancel_order("ETH-USDT", "eth-sell-1", "carol")
            .expect("cancel eth-sell-1");

        // Flush wal buffers
        let btc_shard = engine.get_shard("BTC-USDT").unwrap();
        btc_shard.write().unwrap().flush_wal().unwrap();
        let eth_shard = engine.get_shard("ETH-USDT").unwrap();
        eth_shard.write().unwrap().flush_wal().unwrap();
    }

    // Phase 2: Create a fresh ShardedEngine instance and recover state from disk
    {
        let recovered_engine = ShardedEngine::with_wal(&dir_guard.path, SyncPolicy::None);
        recovered_engine
            .register_symbol("BTC-USDT", Some(UDecimal::new(dec!(60000.0)).unwrap()))
            .expect("recover BTC");
        recovered_engine
            .register_symbol("ETH-USDT", Some(UDecimal::new(dec!(3000.0)).unwrap()))
            .expect("recover ETH");

        // Verify BTC-USDT state:
        let btc_stats = recovered_engine.get_stats("BTC-USDT").expect("btc stats");
        assert_eq!(btc_stats.total_trades_count, 1);
        assert_eq!(btc_stats.total_volume_traded, UDecimal::new(dec!(0.5)).unwrap());
        // Alice had 2.0, 0.5 was filled -> 1.5 remaining
        assert_eq!(btc_stats.active_sell_volume, UDecimal::new(dec!(1.5)).unwrap());
        // Bob had 1.0 resting buy
        assert_eq!(btc_stats.active_buy_volume, UDecimal::new(dec!(1.0)).unwrap());

        // Verify ETH-USDT state:
        let eth_stats = recovered_engine.get_stats("ETH-USDT").expect("eth stats");
        assert_eq!(eth_stats.total_trades_count, 1);
        assert_eq!(eth_stats.total_volume_traded, UDecimal::new(dec!(5.0)).unwrap());
        // Remaining 5.0 was cancelled -> 0 active sell volume
        assert_eq!(eth_stats.active_sell_volume, UDecimal::ZERO);
        assert_eq!(eth_stats.active_buy_volume, UDecimal::ZERO);

        // Phase 3: Test Checkpoint + Prune on the recovered engine
        let pruner = WalPruner::new(WalPrunerConfig::from_env(&dir_guard.path).with_retain_snapshots(2));
        let prune_stats = recovered_engine
            .checkpoint_and_prune(&pruner)
            .await
            .expect("checkpoint and prune");
        // Checkpoint created a second snapshot for each symbol
        assert_eq!(prune_stats.snapshots_pruned, 0);
    }
}

#[tokio::test]
async fn test_pruner_spawn_with_engine() {
    use matcher::orderbook::{OrderType, Side, UDecimal};
    use matcher::wal::{SyncPolicy, WalPruner, WalPrunerConfig};
    use matcher::ShardedEngine;
    use rust_decimal_macros::dec;
    use std::sync::Arc;
    use std::time::Duration;

    let dir_guard = TempDirGuard::new("spawn_with_engine");

    let engine = Arc::new(ShardedEngine::with_wal(&dir_guard.path, SyncPolicy::None));
    engine
        .register_symbol("BTC-USDT", Some(UDecimal::new(dec!(65000.0)).unwrap()))
        .expect("register BTC");

    engine
        .submit_order(
            "btc-ord-1".into(),
            "trader-1".into(),
            "BTC-USDT",
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(64000.0)).unwrap()),
            UDecimal::new(dec!(1.0)).unwrap(),
        )
        .expect("submit order");

    let pruner = WalPruner::new(WalPrunerConfig::from_env(&dir_guard.path).with_interval(Duration::from_millis(25)));

    let handle = pruner.spawn_with_engine(Arc::clone(&engine));

    // Wait for at least one checkpoint tick to fire
    tokio::time::sleep(Duration::from_millis(80)).await;
    handle.stop().await;

    // Check that at least one snapshot file was written
    let snap_exists = std::fs::read_dir(&dir_guard.path)
        .unwrap()
        .flatten()
        .any(|e| e.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("snap")));

    assert!(snap_exists, "Snapshot should be created by background task");
}
