use crate::orderbook::{OrderType, Side, UDecimal};
use rust_decimal::Decimal;

pub const SBE_SCHEMA_ID: u16 = 1;
pub const SBE_SCHEMA_VERSION_1: u16 = 1;
pub const SBE_SCHEMA_VERSION_2: u16 = 2;

pub const TEMPLATE_SUBMIT_ORDER: u16 = 1;
pub const TEMPLATE_CANCEL_ORDER: u16 = 2;
pub const TEMPLATE_SNAPSHOT_MARKER: u16 = 3;

pub const SBE_HEADER_SIZE: usize = 8;
pub const SUBMIT_ORDER_V1_BLOCK_LENGTH: u16 = 108;
pub const SUBMIT_ORDER_V2_BLOCK_LENGTH: u16 = 132;
pub const CANCEL_ORDER_BLOCK_LENGTH: u16 = 64;
pub const SNAPSHOT_MARKER_BLOCK_LENGTH: u16 = 16;
pub const FRAME_HEADER_SIZE: usize = 24;

/// SBE Standard Message Header (8 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SbeHeader {
    pub block_length: u16,
    pub template_id: u16,
    pub schema_id: u16,
    pub version: u16,
}

impl SbeHeader {
    pub const SIZE: usize = SBE_HEADER_SIZE;

    #[must_use]
    pub fn new(block_length: u16, template_id: u16, version: u16) -> Self {
        Self {
            block_length,
            template_id,
            schema_id: SBE_SCHEMA_ID,
            version,
        }
    }

    #[must_use]
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..2].copy_from_slice(&self.block_length.to_le_bytes());
        buf[2..4].copy_from_slice(&self.template_id.to_le_bytes());
        buf[4..6].copy_from_slice(&self.schema_id.to_le_bytes());
        buf[6..8].copy_from_slice(&self.version.to_le_bytes());
        buf
    }

    pub fn decode(buf: &[u8]) -> Result<Self, String> {
        if buf.len() < Self::SIZE {
            return Err(format!(
                "Buffer too short for SbeHeader: expected {}, got {}",
                Self::SIZE,
                buf.len()
            ));
        }
        let block_length = u16::from_le_bytes([buf[0], buf[1]]);
        let template_id = u16::from_le_bytes([buf[2], buf[3]]);
        let schema_id = u16::from_le_bytes([buf[4], buf[5]]);
        let version = u16::from_le_bytes([buf[6], buf[7]]);

        Ok(Self {
            block_length,
            template_id,
            schema_id,
            version,
        })
    }
}

/// WAL Atomic Frame Header (24 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalFrameHeader {
    pub frame_len: u32,
    pub crc32: u32,
    pub sequence: u64,
    pub timestamp_ns: u64,
}

impl WalFrameHeader {
    pub const SIZE: usize = FRAME_HEADER_SIZE;

    #[must_use]
    pub fn new(frame_len: u32, crc32: u32, sequence: u64, timestamp_ns: u64) -> Self {
        Self {
            frame_len,
            crc32,
            sequence,
            timestamp_ns,
        }
    }

    #[must_use]
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.frame_len.to_le_bytes());
        buf[4..8].copy_from_slice(&self.crc32.to_le_bytes());
        buf[8..16].copy_from_slice(&self.sequence.to_le_bytes());
        buf[16..24].copy_from_slice(&self.timestamp_ns.to_le_bytes());
        buf
    }

    pub fn decode(buf: &[u8]) -> Result<Self, String> {
        if buf.len() < Self::SIZE {
            return Err(format!(
                "Buffer too short for WalFrameHeader: expected {}, got {}",
                Self::SIZE,
                buf.len()
            ));
        }
        let frame_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let crc32 = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let sequence = u64::from_le_bytes([
            buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]);
        let timestamp_ns = u64::from_le_bytes([
            buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
        ]);

        Ok(Self {
            frame_len,
            crc32,
            sequence,
            timestamp_ns,
        })
    }
}

/// SBE representation of `SubmitOrder` command
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitOrderSbe {
    pub order_id: [u8; 32],
    pub user_id: [u8; 32],
    pub side: u8,
    pub order_type: u8,
    pub price_mantissa: i128,
    pub price_scale: u32,
    pub qty_mantissa: i128,
    pub qty_scale: u32,
    // V2 optional fields
    pub time_in_force: u8,
    pub trigger_price_mantissa: i128,
    pub trigger_price_scale: u32,
}

/// SBE representation of `CancelOrder` command
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOrderSbe {
    pub order_id: [u8; 32],
    pub user_id: [u8; 32],
}

/// SBE representation of `SnapshotMarker`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMarkerSbe {
    pub snapshot_seq: u64,
    pub open_orders_count: u64,
}

/// High-level enum representing any decoded WAL message
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalPayload {
    SubmitOrder(SubmitOrderSbe),
    CancelOrder(CancelOrderSbe),
    SnapshotMarker(SnapshotMarkerSbe),
}

/// Decoded record containing frame metadata + SBE payload
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecord {
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub payload: WalPayload,
}

// === String Encoding Helpers ===

pub fn encode_fixed_str(s: &str) -> Result<[u8; 32], String> {
    if s.len() > 32 {
        return Err(format!(
            "String exceeds maximum fixed SBE length of 32 bytes (len={})",
            s.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr[..s.len()].copy_from_slice(s.as_bytes());
    Ok(arr)
}

pub fn decode_fixed_str(arr: &[u8; 32]) -> Result<String, String> {
    let len = arr.iter().position(|&b| b == 0).unwrap_or(32);
    std::str::from_utf8(&arr[..len])
        .map(std::string::ToString::to_string)
        .map_err(|e| format!("Invalid UTF-8 string in SBE fixed array: {e}"))
}

// === Decimal Conversion Helpers ===

#[must_use]
pub fn encode_decimal(d: &Option<UDecimal>) -> (i128, u32) {
    match d {
        Some(dec) => {
            let inner = dec.get();
            (inner.mantissa(), inner.scale())
        }
        None => (0, 0),
    }
}

pub fn decode_decimal(mantissa: i128, scale: u32) -> Result<Option<UDecimal>, String> {
    if mantissa == 0 && scale == 0 {
        return Ok(None);
    }
    let dec = Decimal::from_i128_with_scale(mantissa, scale);
    let udec = UDecimal::new(dec)?;
    Ok(Some(udec))
}

#[must_use]
pub fn encode_side(side: Side) -> u8 {
    match side {
        Side::Buy => 1,
        Side::Sell => 2,
    }
}

pub fn decode_side(val: u8) -> Result<Side, String> {
    match val {
        1 => Ok(Side::Buy),
        2 => Ok(Side::Sell),
        _ => Err(format!("Invalid SBE Side discriminant: {val}")),
    }
}

#[must_use]
pub fn encode_order_type(ot: OrderType) -> u8 {
    match ot {
        OrderType::Limit => 1,
        OrderType::Market => 2,
    }
}

pub fn decode_order_type(val: u8) -> Result<OrderType, String> {
    match val {
        1 => Ok(OrderType::Limit),
        2 => Ok(OrderType::Market),
        _ => Err(format!("Invalid SBE OrderType discriminant: {val}")),
    }
}
