use crate::wal::crc32::crc32;
use crate::wal::sbe::types::{
    CancelOrderSbe, SbeHeader, SnapshotMarkerSbe, SubmitOrderSbe, WalFrameHeader, WalPayload,
    WalRecord, CANCEL_ORDER_BLOCK_LENGTH, SBE_HEADER_SIZE, SNAPSHOT_MARKER_BLOCK_LENGTH,
    SUBMIT_ORDER_V1_BLOCK_LENGTH, SUBMIT_ORDER_V2_BLOCK_LENGTH, TEMPLATE_CANCEL_ORDER,
    TEMPLATE_SNAPSHOT_MARKER, TEMPLATE_SUBMIT_ORDER,
};

#[derive(Debug, Default)]
pub struct SbeDecoder;

impl SbeDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Decodes a complete raw WAL frame (Frame Header + SBE Header + Payload)
    pub fn decode_frame(&self, frame_bytes: &[u8]) -> Result<WalRecord, String> {
        if frame_bytes.len() < WalFrameHeader::SIZE + SBE_HEADER_SIZE {
            return Err(format!(
                "Frame buffer too short: expected at least {}, got {}",
                WalFrameHeader::SIZE + SBE_HEADER_SIZE,
                frame_bytes.len()
            ));
        }

        // 1. Decode Frame Header
        let frame_header = WalFrameHeader::decode(&frame_bytes[..WalFrameHeader::SIZE])?;
        let payload_start = WalFrameHeader::SIZE;
        let expected_frame_size = WalFrameHeader::SIZE + frame_header.frame_len as usize;

        if frame_bytes.len() < expected_frame_size {
            return Err(format!(
                "Incomplete frame: expected {} bytes, got {} bytes",
                expected_frame_size,
                frame_bytes.len()
            ));
        }

        let payload_bytes = &frame_bytes[payload_start..expected_frame_size];

        // 2. Validate CRC32 Checksum
        let computed_crc = crc32(payload_bytes);
        if computed_crc != frame_header.crc32 {
            return Err(format!(
                "CRC32 checksum mismatch for seq {}: expected 0x{:08X}, computed 0x{:08X}",
                frame_header.sequence, frame_header.crc32, computed_crc
            ));
        }

        // 3. Decode SBE Header
        let sbe_header = SbeHeader::decode(&payload_bytes[..SBE_HEADER_SIZE])?;
        let body_bytes = &payload_bytes[SBE_HEADER_SIZE..];

        // 4. Decode SBE Body based on (template_id, version)
        let payload = match (sbe_header.template_id, sbe_header.version) {
            (TEMPLATE_SUBMIT_ORDER, 1) => {
                WalPayload::SubmitOrder(self.decode_submit_order_v1(body_bytes)?)
            }
            (TEMPLATE_SUBMIT_ORDER, v) if v >= 2 => {
                WalPayload::SubmitOrder(self.decode_submit_order_v2(body_bytes)?)
            }
            (TEMPLATE_CANCEL_ORDER, 1..=u16::MAX) => {
                WalPayload::CancelOrder(self.decode_cancel_order_v1(body_bytes)?)
            }
            (TEMPLATE_SNAPSHOT_MARKER, 1..=u16::MAX) => {
                WalPayload::SnapshotMarker(self.decode_snapshot_marker_v1(body_bytes)?)
            }
            (template, ver) => {
                return Err(format!(
                    "Unsupported SBE template_id: {template}, version: {ver}"
                ));
            }
        };

        Ok(WalRecord {
            sequence: frame_header.sequence,
            timestamp_ns: frame_header.timestamp_ns,
            payload,
        })
    }

    /// Decodes Version 1 of SubmitOrderSbe (108 bytes fixed payload)
    pub fn decode_submit_order_v1(&self, body: &[u8]) -> Result<SubmitOrderSbe, String> {
        if body.len() < SUBMIT_ORDER_V1_BLOCK_LENGTH as usize {
            return Err(format!(
                "SubmitOrder V1 body too short: expected at least {}, got {}",
                SUBMIT_ORDER_V1_BLOCK_LENGTH,
                body.len()
            ));
        }

        let mut order_id = [0u8; 32];
        order_id.copy_from_slice(&body[0..32]);

        let mut user_id = [0u8; 32];
        user_id.copy_from_slice(&body[32..64]);

        let side = body[64];
        let order_type = body[65];
        // body[66..68] is reserved padding

        let mut p_mant_bytes = [0u8; 16];
        p_mant_bytes.copy_from_slice(&body[68..84]);
        let price_mantissa = i128::from_le_bytes(p_mant_bytes);

        let mut p_scale_bytes = [0u8; 4];
        p_scale_bytes.copy_from_slice(&body[84..88]);
        let price_scale = u32::from_le_bytes(p_scale_bytes);

        let mut q_mant_bytes = [0u8; 16];
        q_mant_bytes.copy_from_slice(&body[88..104]);
        let qty_mantissa = i128::from_le_bytes(q_mant_bytes);

        let mut q_scale_bytes = [0u8; 4];
        q_scale_bytes.copy_from_slice(&body[104..108]);
        let qty_scale = u32::from_le_bytes(q_scale_bytes);

        Ok(SubmitOrderSbe {
            order_id,
            user_id,
            side,
            order_type,
            price_mantissa,
            price_scale,
            qty_mantissa,
            qty_scale,
            time_in_force: 0,
            trigger_price_mantissa: 0,
            trigger_price_scale: 0,
        })
    }

    /// Decodes Version 2 of SubmitOrderSbe (132 bytes fixed payload with appended conditional fields)
    pub fn decode_submit_order_v2(&self, body: &[u8]) -> Result<SubmitOrderSbe, String> {
        if body.len() < SUBMIT_ORDER_V2_BLOCK_LENGTH as usize {
            return Err(format!(
                "SubmitOrder V2 body too short: expected at least {}, got {}",
                SUBMIT_ORDER_V2_BLOCK_LENGTH,
                body.len()
            ));
        }

        let mut base_v1 = self.decode_submit_order_v1(body)?;

        // V2 appended fields:
        // 108: time_in_force (u8)
        // 109..112: padding (3 bytes)
        // 112..128: trigger_price_mantissa (i128)
        // 128..132: trigger_price_scale (u32)
        base_v1.time_in_force = body[108];

        let mut trig_mant_bytes = [0u8; 16];
        trig_mant_bytes.copy_from_slice(&body[112..128]);
        base_v1.trigger_price_mantissa = i128::from_le_bytes(trig_mant_bytes);

        let mut trig_scale_bytes = [0u8; 4];
        trig_scale_bytes.copy_from_slice(&body[128..132]);
        base_v1.trigger_price_scale = u32::from_le_bytes(trig_scale_bytes);

        Ok(base_v1)
    }

    /// Decodes Version 1 of CancelOrderSbe (64 bytes payload)
    pub fn decode_cancel_order_v1(&self, body: &[u8]) -> Result<CancelOrderSbe, String> {
        if body.len() < CANCEL_ORDER_BLOCK_LENGTH as usize {
            return Err(format!(
                "CancelOrder V1 body too short: expected at least {}, got {}",
                CANCEL_ORDER_BLOCK_LENGTH,
                body.len()
            ));
        }

        let mut order_id = [0u8; 32];
        order_id.copy_from_slice(&body[0..32]);

        let mut user_id = [0u8; 32];
        user_id.copy_from_slice(&body[32..64]);

        Ok(CancelOrderSbe { order_id, user_id })
    }

    /// Decodes Version 1 of SnapshotMarkerSbe (16 bytes payload)
    pub fn decode_snapshot_marker_v1(&self, body: &[u8]) -> Result<SnapshotMarkerSbe, String> {
        if body.len() < SNAPSHOT_MARKER_BLOCK_LENGTH as usize {
            return Err(format!(
                "SnapshotMarker V1 body too short: expected at least {}, got {}",
                SNAPSHOT_MARKER_BLOCK_LENGTH,
                body.len()
            ));
        }

        let snapshot_seq = u64::from_le_bytes([
            body[0], body[1], body[2], body[3], body[4], body[5], body[6], body[7],
        ]);
        let open_orders_count = u64::from_le_bytes([
            body[8], body[9], body[10], body[11], body[12], body[13], body[14], body[15],
        ]);

        Ok(SnapshotMarkerSbe {
            snapshot_seq,
            open_orders_count,
        })
    }
}
