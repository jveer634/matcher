use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use crate::orderbook::{Order, UDecimal};
use crate::wal::crc32::crc32;
use crate::wal::sbe::types::{
    decode_decimal, decode_fixed_str, decode_order_status, decode_order_type, decode_side,
    encode_decimal, encode_fixed_str, encode_order_status, encode_order_type, encode_side,
};

/// 4-byte magic identifier for binary snapshot files: "MSNP" (Matcher Snapshot)
pub const SNAPSHOT_MAGIC: [u8; 4] = [0x4D, 0x53, 0x4E, 0x50];
pub const SNAPSHOT_FORMAT_VERSION_1: u16 = 1;
pub const SNAPSHOT_HEADER_SIZE: usize = 16;
pub const SNAPSHOT_META_SIZE: usize = 104;
pub const SNAPSHOT_ORDER_RECORD_SIZE: usize = 136;

/// Complete state snapshot of a single matching engine symbol shard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSnapshot {
    pub symbol: String,
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub last_traded_price: Option<UDecimal>,
    pub total_volume_traded: UDecimal,
    pub total_trades_count: u64,
    pub open_orders: Vec<Order>,
}

/// Binary serialization and atomic disk persistence for order book snapshots.
#[derive(Debug, Default, Clone, Copy)]
pub struct SnapshotWriter;

impl SnapshotWriter {
    /// Generates the canonical filename for a snapshot file.
    ///
    /// Example: `BTC-USDT_snapshot_0000000500.snap`
    #[must_use]
    pub fn snapshot_filename(symbol: &str, sequence: u64) -> String {
        format!("{symbol}_snapshot_{sequence:010}.snap")
    }

    /// Serializes a [`BookSnapshot`] and writes it atomically to `target_path`.
    ///
    /// # Durability & Atomicity
    /// The snapshot is first written to a temporary file (`.snap.tmp`), flushed, and fsync'd.
    /// It is then atomically renamed to `target_path`, guaranteeing that a crash mid-write
    /// never leaves a corrupt `.snap` file on disk.
    pub fn write_snapshot(target_path: impl AsRef<Path>, snapshot: &BookSnapshot) -> Result<(), String> {
        let target_path = target_path.as_ref();
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create snapshot directory '{}': {e}", parent.display()))?;
        }

        let tmp_path = target_path.with_extension("snap.tmp");
        let payload = Self::encode_payload(snapshot)?;

        // 16-byte atomic header:
        // [0..4]: magic "MSNP"
        // [4..6]: version (u16)
        // [6..8]: flags (u16)
        // [8..12]: crc32 (u32)
        // [12..16]: payload_len (u32)
        let checksum = crc32(&payload);
        let payload_len = u32::try_from(payload.len())
            .map_err(|_| "Snapshot payload exceeds 4GB limit".to_string())?;

        let mut header = [0u8; SNAPSHOT_HEADER_SIZE];
        header[0..4].copy_from_slice(&SNAPSHOT_MAGIC);
        header[4..6].copy_from_slice(&SNAPSHOT_FORMAT_VERSION_1.to_le_bytes());
        header[6..8].copy_from_slice(&0u16.to_le_bytes()); // flags
        header[8..12].copy_from_slice(&checksum.to_le_bytes());
        header[12..16].copy_from_slice(&payload_len.to_le_bytes());

        // Write to tmp file with fdatasync
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)
                .map_err(|e| format!("Failed to create temp snapshot file '{}': {e}", tmp_path.display()))?;

            file.write_all(&header)
                .map_err(|e| format!("Failed to write snapshot header: {e}"))?;
            file.write_all(&payload)
                .map_err(|e| format!("Failed to write snapshot payload: {e}"))?;
            file.flush()
                .map_err(|e| format!("Failed to flush temp snapshot: {e}"))?;
            file.sync_data()
                .map_err(|e| format!("Failed to sync temp snapshot: {e}"))?;
        }

        // Atomic rename into target path
        std::fs::rename(&tmp_path, target_path).map_err(|e| {
            format!(
                "Failed to atomically rename temp snapshot '{}' to '{}': {e}",
                tmp_path.display(),
                target_path.display()
            )
        })?;

        Ok(())
    }

    /// Encodes metadata and all open order records into a contiguous byte buffer.
    fn encode_payload(snapshot: &BookSnapshot) -> Result<Vec<u8>, String> {
        let open_orders_count = snapshot.open_orders.len();
        let total_size = SNAPSHOT_META_SIZE + (open_orders_count * SNAPSHOT_ORDER_RECORD_SIZE);
        let mut buf = Vec::with_capacity(total_size);

        // 1. Metadata Block (88 bytes)
        buf.extend_from_slice(&snapshot.sequence.to_le_bytes());
        buf.extend_from_slice(&snapshot.timestamp_ns.to_le_bytes());

        let fixed_sym = encode_fixed_str(&snapshot.symbol)?;
        buf.extend_from_slice(&fixed_sym); // 32 bytes

        let (ltp_mant, ltp_scale) = encode_decimal(&snapshot.last_traded_price);
        buf.extend_from_slice(&ltp_mant.to_le_bytes()); // 16 bytes
        buf.extend_from_slice(&ltp_scale.to_le_bytes()); // 4 bytes

        let (vol_mant, vol_scale) = encode_decimal(&Some(snapshot.total_volume_traded));
        buf.extend_from_slice(&vol_mant.to_le_bytes()); // 16 bytes
        buf.extend_from_slice(&vol_scale.to_le_bytes()); // 4 bytes

        buf.extend_from_slice(&snapshot.total_trades_count.to_le_bytes()); // 8 bytes
        let orders_len_u64 = u64::try_from(open_orders_count).unwrap_or(0);
        buf.extend_from_slice(&orders_len_u64.to_le_bytes()); // 8 bytes

        // 2. Open Order Records (136 bytes each)
        for order in &snapshot.open_orders {
            let raw_order_id = encode_fixed_str(order.id())?;
            buf.extend_from_slice(&raw_order_id); // 32 bytes

            let raw_user_id = encode_fixed_str(order.user_id())?;
            buf.extend_from_slice(&raw_user_id); // 32 bytes

            buf.push(encode_side(order.side())); // 1 byte
            buf.push(encode_order_type(order.order_type())); // 1 byte
            buf.push(encode_order_status(order.status())); // 1 byte
            buf.push(0); // 1 byte reserved padding

            let (p_mant, p_scale) = encode_decimal(&order.price());
            buf.extend_from_slice(&p_mant.to_le_bytes()); // 16 bytes
            buf.extend_from_slice(&p_scale.to_le_bytes()); // 4 bytes

            let (orig_mant, orig_scale) = encode_decimal(&Some(order.remaining()));
            buf.extend_from_slice(&orig_mant.to_le_bytes()); // 16 bytes
            buf.extend_from_slice(&orig_scale.to_le_bytes()); // 4 bytes

            let (rem_mant, rem_scale) = encode_decimal(&Some(order.quantity()));
            buf.extend_from_slice(&rem_mant.to_le_bytes()); // 16 bytes
            buf.extend_from_slice(&rem_scale.to_le_bytes()); // 4 bytes

            buf.extend_from_slice(&order.timestamp().to_le_bytes()); // 8 bytes
        }

        Ok(buf)
    }
}

/// Binary deserialization and validation for order book snapshots.
#[derive(Debug, Default, Clone, Copy)]
pub struct SnapshotReader;

impl SnapshotReader {
    /// Reads and verifies a [`BookSnapshot`] from the specified file path.
    pub fn read_snapshot(path: impl AsRef<Path>) -> Result<BookSnapshot, String> {
        let path = path.as_ref();
        let mut file = File::open(path)
            .map_err(|e| format!("Failed to open snapshot file '{}': {e}", path.display()))?;

        let mut header_buf = [0u8; SNAPSHOT_HEADER_SIZE];
        file.read_exact(&mut header_buf)
            .map_err(|e| format!("Failed to read snapshot header in '{}': {e}", path.display()))?;

        // 1. Verify Magic
        if header_buf[0..4] != SNAPSHOT_MAGIC {
            return Err(format!(
                "Invalid snapshot magic: expected {:?}, got {:?}",
                SNAPSHOT_MAGIC,
                &header_buf[0..4]
            ));
        }

        let version = u16::from_le_bytes([header_buf[4], header_buf[5]]);
        if version != SNAPSHOT_FORMAT_VERSION_1 {
            return Err(format!("Unsupported snapshot version: {version}"));
        }

        let expected_crc = u32::from_le_bytes([
            header_buf[8],
            header_buf[9],
            header_buf[10],
            header_buf[11],
        ]);
        let payload_len = u32::from_le_bytes([
            header_buf[12],
            header_buf[13],
            header_buf[14],
            header_buf[15],
        ]) as usize;

        let mut payload = vec![0u8; payload_len];
        file.read_exact(&mut payload)
            .map_err(|e| format!("Failed to read snapshot payload: {e}"))?;

        // 2. Verify CRC32 Checksum
        let calculated_crc = crc32(&payload);
        if calculated_crc != expected_crc {
            return Err(format!(
                "Snapshot CRC32 mismatch: expected {expected_crc:#010x}, got {calculated_crc:#010x}"
            ));
        }

        Self::decode_payload(&payload)
    }

    /// Decodes a verified payload buffer into a [`BookSnapshot`].
    fn decode_payload(buf: &[u8]) -> Result<BookSnapshot, String> {
        if buf.len() < SNAPSHOT_META_SIZE {
            return Err(format!(
                "Snapshot payload too short: expected at least {SNAPSHOT_META_SIZE} bytes, got {}",
                buf.len()
            ));
        }

        let sequence = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        let timestamp_ns = u64::from_le_bytes(buf[8..16].try_into().unwrap());

        let mut fixed_sym = [0u8; 32];
        fixed_sym.copy_from_slice(&buf[16..48]);
        let symbol = decode_fixed_str(&fixed_sym)?;

        let ltp_mant = i128::from_le_bytes(buf[48..64].try_into().unwrap());
        let ltp_scale = u32::from_le_bytes(buf[64..68].try_into().unwrap());
        let last_traded_price = decode_decimal(ltp_mant, ltp_scale)?;

        let vol_mant = i128::from_le_bytes(buf[68..84].try_into().unwrap());
        let vol_scale = u32::from_le_bytes(buf[84..88].try_into().unwrap());
        let total_volume_traded = decode_decimal(vol_mant, vol_scale)?
            .unwrap_or(UDecimal::ZERO);

        let total_trades_count = u64::from_le_bytes(buf[88..96].try_into().unwrap());
        let orders_count = usize::try_from(u64::from_le_bytes(buf[96..104].try_into().unwrap()))
            .map_err(|e| format!("Invalid orders count in snapshot: {e}"))?;

        let expected_total_size = SNAPSHOT_META_SIZE + (orders_count * SNAPSHOT_ORDER_RECORD_SIZE);
        if buf.len() < expected_total_size {
            return Err(format!(
                "Snapshot buffer truncated: expected {expected_total_size} bytes for {orders_count} orders, got {}",
                buf.len()
            ));
        }

        let mut open_orders = Vec::with_capacity(orders_count);
        let mut offset = SNAPSHOT_META_SIZE;

        for _ in 0..orders_count {
            let record = &buf[offset..offset + SNAPSHOT_ORDER_RECORD_SIZE];

            let mut raw_order_id = [0u8; 32];
            raw_order_id.copy_from_slice(&record[0..32]);
            let order_id = decode_fixed_str(&raw_order_id)?;

            let mut raw_user_id = [0u8; 32];
            raw_user_id.copy_from_slice(&record[32..64]);
            let user_id = decode_fixed_str(&raw_user_id)?;

            let side = decode_side(record[64])?;
            let order_type = decode_order_type(record[65])?;
            let status = decode_order_status(record[66])?;

            let p_mant = i128::from_le_bytes(record[68..84].try_into().unwrap());
            let p_scale = u32::from_le_bytes(record[84..88].try_into().unwrap());
            let price = decode_decimal(p_mant, p_scale)?;

            let orig_mant = i128::from_le_bytes(record[88..104].try_into().unwrap());
            let orig_scale = u32::from_le_bytes(record[104..108].try_into().unwrap());
            let original_quantity = decode_decimal(orig_mant, orig_scale)?
                .unwrap_or(UDecimal::ZERO);

            let rem_mant = i128::from_le_bytes(record[108..124].try_into().unwrap());
            let rem_scale = u32::from_le_bytes(record[124..128].try_into().unwrap());
            let remaining_quantity = decode_decimal(rem_mant, rem_scale)?
                .unwrap_or(UDecimal::ZERO);

            let timestamp = i64::from_le_bytes(record[128..136].try_into().unwrap());

            let order = Order::reconstruct(
                order_id,
                user_id,
                symbol.clone(),
                side,
                order_type,
                remaining_quantity,
                original_quantity,
                price,
                status,
                timestamp,
            );

            open_orders.push(order);

            offset += SNAPSHOT_ORDER_RECORD_SIZE;
        }

        Ok(BookSnapshot {
            symbol,
            sequence,
            timestamp_ns,
            last_traded_price,
            total_volume_traded,
            total_trades_count,
            open_orders,
        })
    }
}
