pub mod decoder;
pub mod encoder;
pub mod types;

pub use decoder::SbeDecoder;
pub use encoder::SbeEncoder;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::{OrderType, Side, UDecimal};
    use rust_decimal_macros::dec;

    #[test]
    fn test_sbe_submit_order_roundtrip() {
        let encoder = SbeEncoder::new();
        let decoder = SbeDecoder::new();

        let order_id = "ord-btc-1001";
        let user_id = "user-trader-alice";
        let side = Side::Buy;
        let order_type = OrderType::Limit;
        let price = Some(UDecimal::new(dec!(65432.50)).unwrap());
        let quantity = UDecimal::new(dec!(2.75)).unwrap();
        let sequence = 42;
        let timestamp_ns = 1_700_000_000_123_456_789;

        let frame_bytes = encoder
            .encode_submit_order(
                order_id,
                user_id,
                side,
                order_type,
                price,
                quantity,
                sequence,
                timestamp_ns,
            )
            .expect("encode submit order");

        assert_eq!(frame_bytes.len(), WalFrameHeader::SIZE + SBE_HEADER_SIZE + SUBMIT_ORDER_V1_BLOCK_LENGTH as usize);

        let record = decoder.decode_frame(&frame_bytes).expect("decode submit order");
        assert_eq!(record.sequence, sequence);
        assert_eq!(record.timestamp_ns, timestamp_ns);

        match record.payload {
            WalPayload::SubmitOrder(sbe) => {
                assert_eq!(decode_fixed_str(&sbe.order_id).unwrap(), order_id);
                assert_eq!(decode_fixed_str(&sbe.user_id).unwrap(), user_id);
                assert_eq!(decode_side(sbe.side).unwrap(), side);
                assert_eq!(decode_order_type(sbe.order_type).unwrap(), order_type);
                assert_eq!(decode_decimal(sbe.price_mantissa, sbe.price_scale).unwrap(), price);
                assert_eq!(decode_decimal(sbe.qty_mantissa, sbe.qty_scale).unwrap(), Some(quantity));
                assert_eq!(sbe.time_in_force, 0); // V1 default
            }
            _ => panic!("Expected SubmitOrder payload"),
        }
    }

    #[test]
    fn test_sbe_market_order_roundtrip() {
        let encoder = SbeEncoder::new();
        let decoder = SbeDecoder::new();

        let frame_bytes = encoder
            .encode_submit_order(
                "market-ord-1",
                "trader-bob",
                Side::Sell,
                OrderType::Market,
                None,
                UDecimal::new(dec!(10.0)).unwrap(),
                100,
                999999999,
            )
            .unwrap();

        let record = decoder.decode_frame(&frame_bytes).unwrap();
        if let WalPayload::SubmitOrder(sbe) = record.payload {
            assert_eq!(decode_order_type(sbe.order_type).unwrap(), OrderType::Market);
            assert_eq!(decode_decimal(sbe.price_mantissa, sbe.price_scale).unwrap(), None);
            assert_eq!(decode_decimal(sbe.qty_mantissa, sbe.qty_scale).unwrap(), Some(UDecimal::new(dec!(10.0)).unwrap()));
        } else {
            panic!("Expected SubmitOrder");
        }
    }

    #[test]
    fn test_sbe_cancel_order_roundtrip() {
        let encoder = SbeEncoder::new();
        let decoder = SbeDecoder::new();

        let order_id = "ord-to-cancel-99";
        let user_id = "user-owner-bob";
        let sequence = 105;
        let timestamp_ns = 1_700_000_000_999;

        let frame_bytes = encoder
            .encode_cancel_order(order_id, user_id, sequence, timestamp_ns)
            .expect("encode cancel order");

        assert_eq!(frame_bytes.len(), WalFrameHeader::SIZE + SBE_HEADER_SIZE + CANCEL_ORDER_BLOCK_LENGTH as usize);

        let record = decoder.decode_frame(&frame_bytes).expect("decode cancel order");
        assert_eq!(record.sequence, sequence);
        assert_eq!(record.timestamp_ns, timestamp_ns);

        match record.payload {
            WalPayload::CancelOrder(sbe) => {
                assert_eq!(decode_fixed_str(&sbe.order_id).unwrap(), order_id);
                assert_eq!(decode_fixed_str(&sbe.user_id).unwrap(), user_id);
            }
            _ => panic!("Expected CancelOrder payload"),
        }
    }

    #[test]
    fn test_sbe_snapshot_marker_roundtrip() {
        let encoder = SbeEncoder::new();
        let decoder = SbeDecoder::new();

        let frame_bytes = encoder.encode_snapshot_marker(50000, 1200, 50001, 12345678);
        let record = decoder.decode_frame(&frame_bytes).expect("decode snapshot marker");

        assert_eq!(record.sequence, 50001);
        match record.payload {
            WalPayload::SnapshotMarker(sbe) => {
                assert_eq!(sbe.snapshot_seq, 50000);
                assert_eq!(sbe.open_orders_count, 1200);
            }
            _ => panic!("Expected SnapshotMarker payload"),
        }
    }

    #[test]
    fn test_corrupted_crc_detection() {
        let encoder = SbeEncoder::new();
        let decoder = SbeDecoder::new();

        let mut frame_bytes = encoder
            .encode_cancel_order("ord-1", "user-1", 1, 100)
            .unwrap();

        // Corrupt 1 byte in the payload
        let last_idx = frame_bytes.len() - 1;
        frame_bytes[last_idx] ^= 0xFF;

        let res = decoder.decode_frame(&frame_bytes);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("CRC32 checksum mismatch"));
    }

    #[test]
    fn test_schema_evolution_v2_backward_compatibility() {
        let encoder = SbeEncoder::new();
        let decoder = SbeDecoder::new();

        let order_id = "ord-v2-test";
        let user_id = "user-v2-test";
        let side = Side::Buy;
        let order_type = OrderType::Limit;
        let price = Some(UDecimal::new(dec!(100.0)).unwrap());
        let quantity = UDecimal::new(dec!(5.0)).unwrap();
        let time_in_force = 1; // IOC
        let trigger_price = Some(UDecimal::new(dec!(99.5)).unwrap());
        let sequence = 999;
        let timestamp_ns = 11111;

        let frame_bytes = encoder
            .encode_submit_order_v2(
                order_id,
                user_id,
                side,
                order_type,
                price,
                quantity,
                time_in_force,
                trigger_price,
                sequence,
                timestamp_ns,
            )
            .expect("encode v2 submit order");

        assert_eq!(
            frame_bytes.len(),
            WalFrameHeader::SIZE + SBE_HEADER_SIZE + SUBMIT_ORDER_V2_BLOCK_LENGTH as usize
        );

        let record = decoder.decode_frame(&frame_bytes).expect("decode V2 frame");
        if let WalPayload::SubmitOrder(sbe) = record.payload {
            assert_eq!(decode_fixed_str(&sbe.order_id).unwrap(), order_id);
            assert_eq!(decode_fixed_str(&sbe.user_id).unwrap(), user_id);
            assert_eq!(sbe.time_in_force, 1);
            let trig = decode_decimal(sbe.trigger_price_mantissa, sbe.trigger_price_scale).unwrap();
            assert_eq!(trig, trigger_price);
        } else {
            panic!("Expected SubmitOrder");
        }
    }
}
