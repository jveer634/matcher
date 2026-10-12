# Write-Ahead Log (WAL) Architecture & Durability

The **Matcher** Write-Ahead Log (WAL) persistence layer delivers deterministic crash recovery, high throughput, and bounded memory usage for the matching engine. It is built around **Simple Binary Encoding (SBE)**, **per-symbol isolation**, **zero-lock disk snapshots**, and an automated **background checkpointer/pruner**.

---

## 1. Architectural Highlights

```mermaid
flowchart TD
    Client[Incoming Order / Cancel Request] -->|Hot Path| Shard[Symbol Shard RwLock]
    
    subgraph Hot Path [Hot Path (Low Latency)]
        Shard -->|1. Write-Ahead Append| WALWriter[WalWriter Buffer]
        WALWriter -->|2. In-Memory Matching| OrderBook[In-Memory OrderBook]
    end

    subgraph Durability & Flush
        WALWriter -->|SyncPolicy| DiskWAL[data/wal/{symbol}.wal]
    end

    subgraph Periodic Background Worker [Background Checkpointer & Pruner]
        Timer[10-min Timer / ENV Config] --> SnapTask[Parallel Snapshot Task]
        SnapTask -->|Microsecond read lock| Shard
        SnapTask -->|tokio::task::spawn_blocking| SnapFile[data/wal/{symbol}_snapshot_{seq}.snap]
        SnapTask --> Pruner[WalPruner]
        Pruner -->|Delete closed segments < snapshot seq| DiskWAL
        Pruner -->|Retain newest 2 snapshots| SnapFile
    end
```

### Key Principles

1. **Write-Ahead Invariant**:
   Every state mutation (`SubmitOrder`, `CancelOrder`) is serialized into an SBE binary frame and appended to the symbol's WAL **before** the in-memory order book executes the trade or modifies rests.
2. **Per-Symbol Sharded Persistence**:
   Each trading pair (e.g. `BTC-USDT`, `ETH-USDT`) maintains its own independent WAL file and snapshot sequence counter. High trading activity on one shard never introduces lock contention or I/O stalls on another.
3. **Zero-Lock Snapshotting**:
   Order book locks are held purely in memory for microsecond durations to clone resting orders and metrics into a [`BookSnapshot`](../../src/wal/snapshot.rs). The heavy disk serialization, CRC-32 computation, `fsync`, and atomic rename take place asynchronously on background thread pools, allowing live matching to resume immediately.
4. **Fast Crash Recovery**:
   On startup, the engine locates the latest binary snapshot, loads open orders and trade volumes in $O(N)$ contiguous time, and replays only the delta WAL frames recorded after the snapshot checkpoint.
5. **Automated Pruning**:
   A background task runs periodically (default: 10 minutes) to create parallel snapshots across all shards, prune obsolete historical `.wal` segments, and retain the two newest `.snap` files for fail-safe rollback.

---

## 2. Directory & File Organization

By default, logs and snapshots are stored under `data/wal/` (configurable via `WAL_DIR`):

```text
data/wal/
├── BTC-USDT.wal                           # Active WAL append segment
├── BTC-USDT_snapshot_0000000500.snap      # Checkpoint snapshot at sequence 500
├── BTC-USDT_snapshot_0000001000.snap      # Checkpoint snapshot at sequence 1000 (newest)
├── ETH-USDT.wal                           # Independent ETH shard log
└── ETH-USDT_snapshot_0000000250.snap      # Checkpoint snapshot for ETH
```

---

## 3. Wire Protocol & SBE Framing

WAL files use a framed binary stream format with CRC-32 data integrity checks:

### Frame Header (20 bytes)
```text
+-------------------+-------------------+-------------------+-------------------+
|  Magic (4 bytes)  |  Length (4 bytes) |  CRC-32 (4 bytes) |  Sequence (8 bytes|
|   "WAL1" (0x57..) |  payload length   | IEEE 802.3 CRC32  | Monotonic counter |
+-------------------+-------------------+-------------------+-------------------+
```

### SBE Message Header (8 bytes)
```text
+-----------------------+-----------------------+-----------------------+-----------------------+
|  block_length (u16)   |   template_id (u16)   |    schema_id (u16)    |     version (u16)     |
+-----------------------+-----------------------+-----------------------+-----------------------+
```

### Supported Message Templates
- `TemplateId::SubmitOrder` (`0x0001`): Order ID, User ID, Side, OrderType, Price mantissa/scale, Quantity mantissa/scale, Timestamp.
- `TemplateId::CancelOrder` (`0x0002`): Order ID, User ID, Timestamp.
- `TemplateId::SnapshotMarker` (`0x0003`): Snapshot sequence checkpoint, Open orders count, Timestamp.

---

## 4. Binary Snapshot Format (`.snap`)

Snapshots are written atomically using a `.snap.tmp` staging file followed by an `fsync` and atomic `rename`.

### Snapshot File Layout
```text
+--------------------------------------------------------------------------+
| 16-byte Header: Magic b"MSNP" (4B) | Version (2B) | Flags (2B) | CRC32 (4B) | Length (4B) |
+--------------------------------------------------------------------------+
| 88-byte Metadata: Sequence (8B) | Timestamp (8B) | Symbol (32B) | LTP (20B)|
|                  Total Volume (20B) | Total Trades (8B) | Orders Count (8B) |
+--------------------------------------------------------------------------+
| Open Order Records (136 bytes each):                                     |
|  - Order ID (32 bytes)                                                   |
|  - User ID (32 bytes)                                                    |
|  - Side (1 byte), OrderType (1 byte), Status (1 byte), Padding (1 byte)  |
|  - Price Mantissa (16 bytes), Scale (4 bytes)                            |
|  - Original Qty Mantissa (16 bytes), Scale (4 bytes)                     |
|  - Remaining Qty Mantissa (16 bytes), Scale (4 bytes)                    |
|  - Creation Timestamp (8 bytes)                                          |
+--------------------------------------------------------------------------+
```

---

## 5. Configuration & Environment Variables

| Variable | Default | Description |
| :--- | :--- | :--- |
| `WAL_DIR` | `data/wal` | Base directory path where `.wal` segments and `.snap` files are stored. |
| `WAL_SYNC_POLICY` | `none` | Durability mode: `always` (calls `fdatasync` after every append) or `none` (buffered OS page cache writes for ultra-low latency). |
| `WAL_PRUNE_INTERVAL_SECS` | `600` | Periodic checkpoint and pruning interval in seconds (default: 10 minutes). |
| `WAL_PRUNE_INTERVAL_MINS` | `10` | Alternative configuration in minutes. |

---

## 6. Extending Versions & Schema Evolution

For detailed instructions on modifying message templates, adding new message types, and maintaining forward/backward compatibility, consult:
- **[Extending Versions & Schema Evolution Guide](./extending_versions.md)**
