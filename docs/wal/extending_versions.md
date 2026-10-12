# Extending Versions & Schema Evolution Guide

This guide describes how to extend the Write-Ahead Log (WAL) binary protocol and snapshot format in **Matcher** while preserving strict backward and forward compatibility.

---

## 1. Simple Binary Encoding (SBE) Framing Architecture

All WAL records are framed with an 8-byte **SBE Message Header**:

```text
+-----------------------+-----------------------+-----------------------+-----------------------+
|  block_length (u16)   |   template_id (u16)   |    schema_id (u16)    |     version (u16)     |
+-----------------------+-----------------------+-----------------------+-----------------------+
```

| Field | Type | Description |
| :--- | :--- | :--- |
| `block_length` | `u16` | Total byte size of the root fields block as encoded by the writer. |
| `template_id` | `u16` | Identifies the message payload (e.g., `0x0001` for `SubmitOrder`). |
| `schema_id` | `u16` | Identifies the schema namespace (currently `1`). |
| `version` | `u16` | Schema version used by the encoder when writing this message. |

---

## 2. Compatibility Rules

Financial exchange protocols require **zero-downtime upgrades** and the ability to replay historical logs across schema versions. The following rules govern schema evolution:

### Rule 1: Root Block Field Appending
- New fields **must always be appended** to the end of a message's root block.
- Existing fields must **never change offset, order, or data type**.
- Deprecated fields should be retained as padding or reserved bytes.

### Rule 2: Forward Compatibility (Old Reader, New Message)
When an older application reads a message encoded by a newer writer ($V_{\text{msg}} > V_{\text{reader}}$):
1. The message header indicates a `block_length` greater than the old reader's expected struct size.
2. The decoder reads up to its known struct size and skips the remaining bytes (`block_length - reader_expected_length`).
3. live replay succeeds without parsing errors or panics.

### Rule 3: Backward Compatibility (New Reader, Old Message)
When a newer application reads historical logs written by an older writer ($V_{\text{msg}} < V_{\text{reader}}$):
1. The message header indicates a `block_length` smaller than the new reader's struct size.
2. The decoder reads available fields from the historical payload and assigns safe default values (e.g. `None`, `0`, or default flags) to newly added fields.

---

## 3. Step-by-Step Extension Examples

### Example A: Adding a Field to an Existing Message (v1 $\rightarrow$ v2)

Suppose we want to add a `time_in_force` enum byte (`GTC`, `IOC`, `FOK`) to `SubmitOrder`.

#### 1. Define the Version 2 Constant
In [`src/wal/sbe/types.rs`](../../src/wal/sbe/types.rs):
```rust
pub const SCHEMA_VERSION_2: u16 = 2;
pub const SBE_SUBMIT_ORDER_BLOCK_LENGTH_V2: u16 = SBE_SUBMIT_ORDER_BLOCK_LENGTH_V1 + 1; // +1 byte for TIF
```

#### 2. Update the SBE Struct
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitOrderSbe {
    // Existing V1 fields (offsets 0..120)
    pub order_id: [u8; 32],
    pub user_id: [u8; 32],
    pub side: u8,
    pub order_type: u8,
    pub price_mantissa: i128,
    pub price_scale: u32,
    pub qty_mantissa: i128,
    pub qty_scale: u32,
    pub timestamp_ns: u64,

    // Newly added V2 field (offset 120)
    pub time_in_force: u8,
}
```

#### 3. Update the Decoder with Fallback
In [`src/wal/sbe/decoder.rs`](../../src/wal/sbe/decoder.rs):
```rust
let tif = if header.version >= SCHEMA_VERSION_2 && header.block_length >= SBE_SUBMIT_ORDER_BLOCK_LENGTH_V2 {
    payload[120]
} else {
    0 // Default: Good-'Til-Cancelled (GTC)
};
```

---

### Example B: Adding a Brand New Message Template

Suppose we want to add an `AmendOrder` message (`template_id = 0x0004`).

#### 1. Register New Template ID
In [`src/wal/sbe/types.rs`](../../src/wal/sbe/types.rs):
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum TemplateId {
    SubmitOrder = 0x0001,
    CancelOrder = 0x0002,
    SnapshotMarker = 0x0003,
    AmendOrder = 0x0004, // New
}
```

#### 2. Define Payload SBE Struct
```rust
pub const SBE_AMEND_ORDER_BLOCK_LENGTH: u16 = 88;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendOrderSbe {
    pub order_id: [u8; 32],
    pub user_id: [u8; 32],
    pub new_price_mantissa: i128,
    pub new_price_scale: u32,
    pub timestamp_ns: u64,
}
```

#### 3. Update Writer and Reader
Add `append_amend_order` in [`src/wal/writer.rs`](../../src/wal/writer.rs) and handle `WalPayload::AmendOrder` in [`src/matcher.rs`](../../src/matcher.rs) replay.

---

### Example C: Evolving Snapshot Format

Snapshot files contain a 16-byte fixed header:
```text
[0..4]: b"MSNP" magic
[4..6]: version (u16)
[6..8]: flags (u16)
[8..12]: crc32 (u32)
[12..16]: payload_len (u32)
```

To evolve the snapshot format:
1. Define `SNAPSHOT_FORMAT_VERSION_2 = 2`.
2. In [`SnapshotReader::read_snapshot`](../../src/wal/snapshot.rs), inspect `header.version`:
   ```rust
   match version {
       SNAPSHOT_FORMAT_VERSION_1 => Self::decode_payload_v1(&payload),
       SNAPSHOT_FORMAT_VERSION_2 => Self::decode_payload_v2(&payload),
       _ => Err(format!("Unsupported snapshot version: {version}")),
   }
   ```
3. This allows seamless reading of existing snapshots while emitting new formats during periodic checkpointer runs.

---

## 4. Testing Schema Evolution

To guarantee that version extensions never break existing deployments, every schema change must include automated compatibility tests:

```rust
#[test]
fn test_schema_evolution_v2_backward_compatibility() {
    // 1. Encode with a simulated V2 writer (block length + 16 extra bytes)
    // 2. Decode with standard V1 decoder
    // 3. Verify all V1 fields parse with 100% fidelity and extra bytes are safely skipped
}
```

Refer to [`tests/wal_test.rs`](../../tests/wal_test.rs) for verified test patterns.

---

## 5. Deployment & Rollout Strategy

1. **Deploy Reader First**: Deploy services with the new reader code before turning on the new writer. The new reader will cleanly parse older V1 records.
2. **Enable New Writer**: Enable new message fields/templates via configuration or rolling upgrade.
3. **Verify Checkpointer**: Observe that snapshots and pruned WAL segments continue recycling without alerts.
