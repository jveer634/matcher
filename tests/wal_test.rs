use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use matcher::orderbook::{OrderType, Side, UDecimal};
use matcher::wal::{
    decode_decimal, decode_fixed_str, decode_order_type, decode_side, SyncPolicy, WalPayload,
    WalReadError, WalReader, WalWriter,
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
        assert_eq!(
            decode_decimal(sbe.price_mantissa, sbe.price_scale).unwrap(),
            price
        );
        assert_eq!(
            decode_decimal(sbe.qty_mantissa, sbe.qty_scale).unwrap(),
            Some(qty)
        );
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
            .append_submit_order(
                "ord-1",
                "user-1",
                Side::Buy,
                OrderType::Market,
                None,
                qty,
                100,
            )
            .unwrap();
        writer
            .append_submit_order(
                "ord-2",
                "user-2",
                Side::Sell,
                OrderType::Market,
                None,
                qty,
                101,
            )
            .unwrap();
        writer.flush().unwrap();
    }

    // Reopen and continue appending
    {
        let mut writer = WalWriter::open_or_create(&guard.path, SyncPolicy::None).unwrap();
        assert_eq!(writer.next_sequence(), 3);

        let seq = writer
            .append_submit_order(
                "ord-3",
                "user-3",
                Side::Buy,
                OrderType::Market,
                None,
                qty,
                102,
            )
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
