use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::orderbook::{OrderType, Side, UDecimal};
use crate::wal::reader::WalReader;
use crate::wal::sbe::encoder::SbeEncoder;

/// Persistence sync policy for the Write-Ahead Log writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncPolicy {
    /// Calls `file.sync_data()` after every append for zero-data-loss durability.
    Always,
    /// Buffers writes in OS page cache; explicit calls to `flush` or `sync` are required.
    #[default]
    None,
}

impl std::str::FromStr for SyncPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "always" => Ok(SyncPolicy::Always),
            "none" => Ok(SyncPolicy::None),
            other => Err(format!(
                "Invalid sync policy '{other}'. Expected 'always' or 'none'"
            )),
        }
    }
}

impl std::fmt::Display for SyncPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncPolicy::Always => write!(f, "always"),
            SyncPolicy::None => write!(f, "none"),
        }
    }
}

impl<'de> serde::Deserialize<'de> for SyncPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<SyncPolicy>().map_err(serde::de::Error::custom)
    }
}

impl serde::Serialize for SyncPolicy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Append-only Write-Ahead Log writer backed by an SBE encoder.
#[derive(Debug)]
pub struct WalWriter {
    writer: BufWriter<File>,
    path: PathBuf,
    encoder: SbeEncoder,
    sync_policy: SyncPolicy,
    next_sequence: u64,
    bytes_written: u64,
}

impl WalWriter {
    /// Creates a new WAL file at the given path, truncating any existing content.
    /// The sequence number starts at 1.
    pub fn create(path: impl AsRef<Path>, sync_policy: SyncPolicy) -> Result<Self, String> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create WAL directory '{}': {e}", parent.display()))?;
        }

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path_buf)
            .map_err(|e| format!("Failed to create WAL file '{}': {e}", path_buf.display()))?;

        Ok(Self {
            writer: BufWriter::new(file),
            path: path_buf,
            encoder: SbeEncoder::new(),
            sync_policy,
            next_sequence: 1,
            bytes_written: 0,
        })
    }

    /// Opens an existing WAL file for appending, or creates it if it does not exist.
    ///
    /// Reads and validates all existing records in the file:
    /// - Sets `next_sequence = last_sequence + 1` (or 1 if empty).
    /// - If a trailing torn write is detected, automatically truncates the file to `last_valid_offset`
    ///   so that newly appended writes remain consistent.
    pub fn open_or_create(path: impl AsRef<Path>, sync_policy: SyncPolicy) -> Result<Self, String> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create WAL directory '{}': {e}", parent.display()))?;
        }

        if !path_buf.exists() {
            return Self::create(path_buf, sync_policy);
        }

        // Validate existing content and locate the last valid offset & sequence
        let mut reader = WalReader::open(&path_buf)?;
        let (_, read_err) = reader.read_all();

        let valid_offset = reader.last_valid_offset();
        let last_seq = reader.last_sequence().unwrap_or(0);

        // If there was a trailing error, repair/truncate to last valid offset
        if read_err.is_some() {
            WalReader::truncate_to_offset(&path_buf, valid_offset)?;
        }

        let mut file = OpenOptions::new()
            .write(true)
            .open(&path_buf)
            .map_err(|e| format!("Failed to open WAL file '{}': {e}", path_buf.display()))?;

        file.seek(SeekFrom::Start(valid_offset)).map_err(|e| {
            format!(
                "Failed to seek to end of valid WAL records in '{}': {e}",
                path_buf.display()
            )
        })?;

        Ok(Self {
            writer: BufWriter::new(file),
            path: path_buf,
            encoder: SbeEncoder::new(),
            sync_policy,
            next_sequence: last_seq + 1,
            bytes_written: valid_offset,
        })
    }

    /// Appends a V1 `SubmitOrder` record to the WAL.
    ///
    /// Returns the sequence number assigned to this record.
    #[allow(clippy::too_many_arguments)]
    pub fn append_submit_order(
        &mut self,
        order_id: &str,
        user_id: &str,
        side: Side,
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
        timestamp_ns: u64,
    ) -> Result<u64, String> {
        let seq = self.next_sequence;
        let frame_bytes = self.encoder.encode_submit_order(
            order_id,
            user_id,
            side,
            order_type,
            price,
            quantity,
            seq,
            timestamp_ns,
        )?;

        self.write_frame(&frame_bytes)?;
        self.next_sequence += 1;
        Ok(seq)
    }

    /// Appends a V2 `SubmitOrder` record (with conditional `time_in_force` and `trigger_price`) to the WAL.
    ///
    /// Returns the sequence number assigned to this record.
    #[allow(clippy::too_many_arguments)]
    pub fn append_submit_order_v2(
        &mut self,
        order_id: &str,
        user_id: &str,
        side: Side,
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
        time_in_force: u8,
        trigger_price: Option<UDecimal>,
        timestamp_ns: u64,
    ) -> Result<u64, String> {
        let seq = self.next_sequence;
        let frame_bytes = self.encoder.encode_submit_order_v2(
            order_id,
            user_id,
            side,
            order_type,
            price,
            quantity,
            time_in_force,
            trigger_price,
            seq,
            timestamp_ns,
        )?;

        self.write_frame(&frame_bytes)?;
        self.next_sequence += 1;
        Ok(seq)
    }

    /// Appends a `CancelOrder` record to the WAL.
    ///
    /// Returns the sequence number assigned to this record.
    pub fn append_cancel_order(
        &mut self,
        order_id: &str,
        user_id: &str,
        timestamp_ns: u64,
    ) -> Result<u64, String> {
        let seq = self.next_sequence;
        let frame_bytes = self.encoder.encode_cancel_order(order_id, user_id, seq, timestamp_ns)?;

        self.write_frame(&frame_bytes)?;
        self.next_sequence += 1;
        Ok(seq)
    }

    /// Appends a `SnapshotMarker` record to the WAL.
    ///
    /// Returns the sequence number assigned to this record.
    pub fn append_snapshot_marker(
        &mut self,
        snapshot_seq: u64,
        open_orders_count: u64,
        timestamp_ns: u64,
    ) -> Result<u64, String> {
        let seq = self.next_sequence;
        let frame_bytes =
            self.encoder
                .encode_snapshot_marker(snapshot_seq, open_orders_count, seq, timestamp_ns);

        self.write_frame(&frame_bytes)?;
        self.next_sequence += 1;
        Ok(seq)
    }

    /// Low-level write helper for raw frame bytes.
    fn write_frame(&mut self, frame_bytes: &[u8]) -> Result<(), String> {
        self.writer
            .write_all(frame_bytes)
            .map_err(|e| format!("Failed to write WAL frame: {e}"))?;

        self.bytes_written += frame_bytes.len() as u64;

        if self.sync_policy == SyncPolicy::Always {
            self.writer
                .flush()
                .map_err(|e| format!("Failed to flush WAL frame buffer: {e}"))?;
            self.writer
                .get_ref()
                .sync_data()
                .map_err(|e| format!("Failed to fdatasync WAL file: {e}"))?;
        }

        Ok(())
    }

    /// Explicitly flushes the internal write buffer into operating system page cache.
    pub fn flush(&mut self) -> Result<(), String> {
        self.writer
            .flush()
            .map_err(|e| format!("Failed to flush WAL writer: {e}"))
    }

    /// Flushes write buffer and performs `sync_data` (fdatasync) to ensure all bytes are committed to disk.
    pub fn sync(&mut self) -> Result<(), String> {
        self.writer
            .flush()
            .map_err(|e| format!("Failed to flush WAL writer before sync: {e}"))?;
        self.writer
            .get_ref()
            .sync_data()
            .map_err(|e| format!("Failed to fdatasync WAL file: {e}"))
    }

    /// Next sequence number that will be assigned.
    #[must_use]
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Total count of bytes written and acknowledged in this WAL file.
    #[must_use]
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Path to the current WAL file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}
