use crate::orderbook::{OrderType, Side, UDecimal};
use crate::wal::crc32::crc32;
use crate::wal::sbe::types::{
    encode_decimal, encode_fixed_str, encode_order_type, encode_side, SbeHeader,
    WalFrameHeader, CANCEL_ORDER_BLOCK_LENGTH, SBE_HEADER_SIZE, SBE_SCHEMA_VERSION_1,
    SNAPSHOT_MARKER_BLOCK_LENGTH, SUBMIT_ORDER_V1_BLOCK_LENGTH, SUBMIT_ORDER_V2_BLOCK_LENGTH,
    TEMPLATE_CANCEL_ORDER, TEMPLATE_SNAPSHOT_MARKER, TEMPLATE_SUBMIT_ORDER,
};

#[derive(Debug, Default)]
pub struct SbeEncoder;

impl SbeEncoder {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Encodes a `SubmitOrder` command into a complete WAL frame (Frame Header + SBE Header + Payload)
    #[allow(clippy::too_many_arguments)]
    pub fn encode_submit_order(
        &self,
        order_id: &str,
        user_id: &str,
        side: Side,
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
        sequence: u64,
        timestamp_ns: u64,
    ) -> Result<Vec<u8>, String> {
        let order_id_bytes = encode_fixed_str(order_id)?;
        let user_id_bytes = encode_fixed_str(user_id)?;
        let side_byte = encode_side(side);
        let order_type_byte = encode_order_type(order_type);
        let (price_mantissa, price_scale) = encode_decimal(&price);
        let (qty_mantissa, qty_scale) = encode_decimal(&Some(quantity));

        let sbe_header = SbeHeader::new(
            SUBMIT_ORDER_V1_BLOCK_LENGTH,
            TEMPLATE_SUBMIT_ORDER,
            SBE_SCHEMA_VERSION_1,
        );

        let mut payload = Vec::with_capacity(SBE_HEADER_SIZE + SUBMIT_ORDER_V1_BLOCK_LENGTH as usize);
        payload.extend_from_slice(&sbe_header.encode());

        // Body: 108 bytes
        payload.extend_from_slice(&order_id_bytes);               // 0..32
        payload.extend_from_slice(&user_id_bytes);                // 32..64
        payload.push(side_byte);                                   // 64
        payload.push(order_type_byte);                             // 65
        payload.extend_from_slice(&[0u8; 2]);                      // 66..68 (padding)
        payload.extend_from_slice(&price_mantissa.to_le_bytes());  // 68..84 (16 bytes i128)
        payload.extend_from_slice(&price_scale.to_le_bytes());     // 84..88 (4 bytes u32)
        payload.extend_from_slice(&qty_mantissa.to_le_bytes());    // 88..104 (16 bytes i128)
        payload.extend_from_slice(&qty_scale.to_le_bytes());       // 104..108 (4 bytes u32)

        let checksum = crc32(&payload);
        let frame_len = u32::try_from(payload.len()).map_err(|e| e.to_string())?;
        let frame_header = WalFrameHeader::new(frame_len, checksum, sequence, timestamp_ns);

        let mut frame = Vec::with_capacity(WalFrameHeader::SIZE + payload.len());
        frame.extend_from_slice(&frame_header.encode());
        frame.extend_from_slice(&payload);

        Ok(frame)
    }

    /// Encodes a Version 2 `SubmitOrder` command with conditional/extended fields into a complete WAL frame
    #[allow(clippy::too_many_arguments)]
    pub fn encode_submit_order_v2(
        &self,
        order_id: &str,
        user_id: &str,
        side: Side,
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
        time_in_force: u8,
        trigger_price: Option<UDecimal>,
        sequence: u64,
        timestamp_ns: u64,
    ) -> Result<Vec<u8>, String> {
        let order_id_bytes = encode_fixed_str(order_id)?;
        let user_id_bytes = encode_fixed_str(user_id)?;
        let side_byte = encode_side(side);
        let order_type_byte = encode_order_type(order_type);
        let (price_mantissa, price_scale) = encode_decimal(&price);
        let (qty_mantissa, qty_scale) = encode_decimal(&Some(quantity));
        let (trigger_mantissa, trigger_scale) = encode_decimal(&trigger_price);

        let sbe_header = SbeHeader::new(
            SUBMIT_ORDER_V2_BLOCK_LENGTH,
            TEMPLATE_SUBMIT_ORDER,
            crate::wal::sbe::types::SBE_SCHEMA_VERSION_2,
        );

        let mut payload = Vec::with_capacity(SBE_HEADER_SIZE + SUBMIT_ORDER_V2_BLOCK_LENGTH as usize);
        payload.extend_from_slice(&sbe_header.encode());

        // Body: 108 bytes (V1 fields)
        payload.extend_from_slice(&order_id_bytes);               // 0..32
        payload.extend_from_slice(&user_id_bytes);                // 32..64
        payload.push(side_byte);                                   // 64
        payload.push(order_type_byte);                             // 65
        payload.extend_from_slice(&[0u8; 2]);                      // 66..68 (padding)
        payload.extend_from_slice(&price_mantissa.to_le_bytes());  // 68..84 (16 bytes i128)
        payload.extend_from_slice(&price_scale.to_le_bytes());     // 84..88 (4 bytes u32)
        payload.extend_from_slice(&qty_mantissa.to_le_bytes());    // 88..104 (16 bytes i128)
        payload.extend_from_slice(&qty_scale.to_le_bytes());       // 104..108 (4 bytes u32)

        // V2 Appended fields: 24 bytes (108..132)
        payload.push(time_in_force);                               // 108
        payload.extend_from_slice(&[0u8; 3]);                      // 109..112 (padding)
        payload.extend_from_slice(&trigger_mantissa.to_le_bytes());// 112..128 (16 bytes i128)
        payload.extend_from_slice(&trigger_scale.to_le_bytes());   // 128..132 (4 bytes u32)

        let checksum = crc32(&payload);
        let frame_len = u32::try_from(payload.len()).map_err(|e| e.to_string())?;
        let frame_header = WalFrameHeader::new(frame_len, checksum, sequence, timestamp_ns);

        let mut frame = Vec::with_capacity(WalFrameHeader::SIZE + payload.len());
        frame.extend_from_slice(&frame_header.encode());
        frame.extend_from_slice(&payload);

        Ok(frame)
    }

    /// Encodes a `CancelOrder` command into a complete WAL frame
    pub fn encode_cancel_order(
        &self,
        order_id: &str,
        user_id: &str,
        sequence: u64,
        timestamp_ns: u64,
    ) -> Result<Vec<u8>, String> {
        let order_id_bytes = encode_fixed_str(order_id)?;
        let user_id_bytes = encode_fixed_str(user_id)?;

        let sbe_header = SbeHeader::new(
            CANCEL_ORDER_BLOCK_LENGTH,
            TEMPLATE_CANCEL_ORDER,
            SBE_SCHEMA_VERSION_1,
        );

        let mut payload = Vec::with_capacity(SBE_HEADER_SIZE + CANCEL_ORDER_BLOCK_LENGTH as usize);
        payload.extend_from_slice(&sbe_header.encode());

        // Body: 64 bytes (32 bytes order_id + 32 bytes user_id)
        payload.extend_from_slice(&order_id_bytes);
        payload.extend_from_slice(&user_id_bytes);

        let checksum = crc32(&payload);
        let frame_len = u32::try_from(payload.len()).map_err(|e| e.to_string())?;
        let frame_header = WalFrameHeader::new(frame_len, checksum, sequence, timestamp_ns);

        let mut frame = Vec::with_capacity(WalFrameHeader::SIZE + payload.len());
        frame.extend_from_slice(&frame_header.encode());
        frame.extend_from_slice(&payload);

        Ok(frame)
    }

    /// Encodes a `SnapshotMarker` into a complete WAL frame
    #[must_use]
    pub fn encode_snapshot_marker(
        &self,
        snapshot_seq: u64,
        open_orders_count: u64,
        sequence: u64,
        timestamp_ns: u64,
    ) -> Vec<u8> {
        let sbe_header = SbeHeader::new(
            SNAPSHOT_MARKER_BLOCK_LENGTH,
            TEMPLATE_SNAPSHOT_MARKER,
            SBE_SCHEMA_VERSION_1,
        );

        let mut payload = Vec::with_capacity(SBE_HEADER_SIZE + SNAPSHOT_MARKER_BLOCK_LENGTH as usize);
        payload.extend_from_slice(&sbe_header.encode());

        // Body: 16 bytes
        payload.extend_from_slice(&snapshot_seq.to_le_bytes());
        payload.extend_from_slice(&open_orders_count.to_le_bytes());

        let checksum = crc32(&payload);
        let frame_len = u32::try_from(payload.len()).unwrap_or(0);
        let frame_header = WalFrameHeader::new(frame_len, checksum, sequence, timestamp_ns);

        let mut frame = Vec::with_capacity(WalFrameHeader::SIZE + payload.len());
        frame.extend_from_slice(&frame_header.encode());
        frame.extend_from_slice(&payload);

        frame
    }
}
