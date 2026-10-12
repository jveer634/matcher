use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use crate::wal::crc32::crc32;
use crate::wal::sbe::decoder::SbeDecoder;
use crate::wal::sbe::types::{WalFrameHeader, WalRecord};

/// Errors encountered while reading or recovering from a WAL file.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum WalReadError {
    /// Standard I/O failure while reading from the file.
    Io(String),
    /// Data corruption detected: CRC32 checksum mismatch.
    CorruptedCrc {
        sequence: u64,
        expected: u32,
        actual: u32,
    },
    /// Incomplete or truncated frame at the end of the file (e.g. crash during write).
    TornWrite {
        bytes_available: usize,
        bytes_expected: usize,
    },
    /// Frame decoding failed (e.g. unsupported schema or invalid fields).
    InvalidFrame(String),
}

impl std::fmt::Display for WalReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "WAL I/O error: {err}"),
            Self::CorruptedCrc {
                sequence,
                expected,
                actual,
            } => write!(
                f,
                "WAL CRC mismatch at seq {sequence}: expected {expected:#010x}, calculated {actual:#010x}"
            ),
            Self::TornWrite {
                bytes_available,
                bytes_expected,
            } => write!(
                f,
                "WAL torn write: expected {bytes_expected} bytes, but only {bytes_available} bytes available before EOF"
            ),
            Self::InvalidFrame(err) => write!(f, "WAL invalid frame: {err}"),
        }
    }
}

impl std::error::Error for WalReadError {}

/// Sequential streaming reader for Write-Ahead Log files.
#[derive(Debug)]
pub struct WalReader {
    reader: BufReader<File>,
    path: PathBuf,
    decoder: SbeDecoder,
    last_valid_offset: u64,
    last_sequence: Option<u64>,
    eof_reached: bool,
}

impl WalReader {
    /// Opens an existing WAL file for sequential reading.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path_buf = path.as_ref().to_path_buf();
        let file = File::open(&path_buf)
            .map_err(|e| format!("Failed to open WAL file '{}': {e}", path_buf.display()))?;

        Ok(Self {
            reader: BufReader::new(file),
            path: path_buf,
            decoder: SbeDecoder::new(),
            last_valid_offset: 0,
            last_sequence: None,
            eof_reached: false,
        })
    }

    /// Reads the next [`WalRecord`] from the file.
    ///
    /// Returns:
    /// - `Ok(Some(record))` if a complete valid frame was parsed.
    /// - `Ok(None)` if clean EOF was reached.
    /// - `Err(WalReadError)` if an error occurred (e.g. torn write, CRC mismatch).
    pub fn next_record(&mut self) -> Result<Option<WalRecord>, WalReadError> {
        if self.eof_reached {
            return Ok(None);
        }

        // 1. Read Frame Header (24 bytes)
        let mut header_buf = [0u8; WalFrameHeader::SIZE];
        let mut header_bytes_read = 0;

        while header_bytes_read < WalFrameHeader::SIZE {
            match self.reader.read(&mut header_buf[header_bytes_read..]) {
                Ok(0) => break, // EOF encountered
                Ok(n) => header_bytes_read += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(WalReadError::Io(e.to_string())),
            }
        }

        if header_bytes_read == 0 {
            // Clean EOF
            self.eof_reached = true;
            return Ok(None);
        }

        if header_bytes_read < WalFrameHeader::SIZE {
            self.eof_reached = true;
            return Err(WalReadError::TornWrite {
                bytes_available: header_bytes_read,
                bytes_expected: WalFrameHeader::SIZE,
            });
        }

        let frame_header = WalFrameHeader::decode(&header_buf).map_err(WalReadError::InvalidFrame)?;
        let payload_len = frame_header.frame_len as usize;

        // 2. Read Frame Payload (SBE Header + Body)
        let mut payload_buf = vec![0u8; payload_len];
        let mut payload_bytes_read = 0;

        while payload_bytes_read < payload_len {
            match self.reader.read(&mut payload_buf[payload_bytes_read..]) {
                Ok(0) => break, // EOF reached before reading complete payload
                Ok(n) => payload_bytes_read += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(WalReadError::Io(e.to_string())),
            }
        }

        if payload_bytes_read < payload_len {
            self.eof_reached = true;
            return Err(WalReadError::TornWrite {
                bytes_available: payload_bytes_read,
                bytes_expected: payload_len,
            });
        }

        // 3. Verify CRC32 Checksum
        let calculated_crc = crc32(&payload_buf);
        if calculated_crc != frame_header.crc32 {
            return Err(WalReadError::CorruptedCrc {
                sequence: frame_header.sequence,
                expected: frame_header.crc32,
                actual: calculated_crc,
            });
        }

        // 4. Decode SBE Payload into WalRecord
        // Reconstruct full frame slice for SbeDecoder
        let mut full_frame = Vec::with_capacity(WalFrameHeader::SIZE + payload_len);
        full_frame.extend_from_slice(&header_buf);
        full_frame.extend_from_slice(&payload_buf);

        let record = self
            .decoder
            .decode_frame(&full_frame)
            .map_err(WalReadError::InvalidFrame)?;

        // Update offsets and sequences
        let frame_total_len = (WalFrameHeader::SIZE + payload_len) as u64;
        self.last_valid_offset += frame_total_len;
        self.last_sequence = Some(record.sequence);

        Ok(Some(record))
    }

    /// Reads all records until EOF or first failure.
    /// Returns the vector of successfully decoded records and an optional error if reading stopped due to an error.
    pub fn read_all(&mut self) -> (Vec<WalRecord>, Option<WalReadError>) {
        let mut records = Vec::new();
        loop {
            match self.next_record() {
                Ok(Some(record)) => records.push(record),
                Ok(None) => return (records, None),
                Err(err) => return (records, Some(err)),
            }
        }
    }

    /// Returns the byte offset of the last known valid record in the file.
    #[must_use]
    pub fn last_valid_offset(&self) -> u64 {
        self.last_valid_offset
    }

    /// Path to the WAL file being read.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the highest sequence number decoded so far.
    #[must_use]
    pub fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }

    /// Truncates a WAL file to a specified valid offset, discarding any trailing torn writes or corrupted tail bytes.
    pub fn truncate_to_offset(path: impl AsRef<Path>, valid_offset: u64) -> Result<(), String> {
        let path = path.as_ref();
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| format!("Failed to open WAL file '{}' for truncation: {e}", path.display()))?;

        file.set_len(valid_offset)
            .map_err(|e| format!("Failed to truncate WAL file to {valid_offset} bytes: {e}"))?;

        file.sync_all()
            .map_err(|e| format!("Failed to sync truncated WAL file '{}': {e}", path.display()))?;

        Ok(())
    }
}

impl Iterator for WalReader {
    type Item = Result<WalRecord, WalReadError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof_reached {
            return None;
        }

        match self.next_record() {
            Ok(Some(record)) => Some(Ok(record)),
            Ok(None) => None,
            Err(err) => {
                self.eof_reached = true;
                Some(Err(err))
            }
        }
    }
}
