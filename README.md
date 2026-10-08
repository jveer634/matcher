# Matcher

A high-performance, in-memory, price-time priority trading matching engine written in Rust, designed with a **symbol-sharded architecture**.

## Architecture & Design

> For visual Mermaid diagrams of request flows, sharding concurrency, and protobuf pipelines, see [Architecture & Relationships](docs/architecture.md).

### Per-Symbol Sharding Model
In high-frequency exchange architectures, trading pairs operate independently. A cross-symbol lock creates unnecessary contention and bottlenecks.

This codebase uses **per-symbol isolation**:
- **Independent Shards (`Matcher`)**: Each `Matcher` instance is dedicated to exactly **one** trading pair (e.g. `BTC-USDT`, `ETH-USDT`). It encapsulates its own `OrderBook`, ID generation, trade history, and execution metrics.
- **Concurrent Sharded Router (`ShardedEngine`)**: Routes incoming orders and queries to their respective symbol shard. Shards execute concurrently in parallel threads with isolated locks (`Arc<RwLock<Matcher>>`), guaranteeing zero cross-symbol contention.

```
                  +-----------------------------------+
                  |           ShardedEngine           |
                  |     (Multi-Symbol Dispatcher)     |
                  +-----------------+-----------------+
                                    |
          +-------------------------+-------------------------+
          |                         |                         |
          v                         v                         v
+-------------------+     +-------------------+     +-------------------+
|  Matcher Shard 1  |     |  Matcher Shard 2  |     |  Matcher Shard 3  |
|    "BTC-USDT"     |     |    "ETH-USDT"     |     |    "SOL-USDT"     |
+-------------------+     +-------------------+     +-------------------+
| - OrderBook       |     | - OrderBook       |     | - OrderBook       |
| - Trade History   |     | - Trade History   |     | - Trade History   |
| - Metrics / Stats |     | - Metrics / Stats |     | - Metrics / Stats |
+-------------------+     +-------------------+     +-------------------+
```

### Core Components

1. **[`Matcher`](src/matcher.rs)**:
   - Manages state, order book lifecycle, trade generation, and metrics for a single symbol.
   - Methods: `submit_order`, `cancel_order`, `get_depth`, `get_order`, `stats`.

2. **[`ShardedEngine`](src/engine.rs)**:
   - Dynamic registry for per-symbol shards.
   - Thread-safe concurrent access across symbols.

3. **[`OrderBook`](src/orderbook/orderbook.rs)**:
   - **Price-Time Priority (FIFO)** matching algorithm.
   - Bids: `BTreeMap<UDecimal, VecDeque<Order>>` (sorted descending for matching).
   - Asks: `BTreeMap<UDecimal, VecDeque<Order>>` (sorted ascending for matching).
   - O(1) order lookup and indexing via `HashMap<String, Order>`.

4. **[`Order`](src/orderbook/order.rs)**:
   - Supports `Side` (`Buy`, `Sell`) and `OrderType` (`Market`, `Limit`).
   - Precise non-negative financial values using `UDecimal`.
   - Lifecycle tracking: `Open`, `PartiallyExecuted`, `Executed`, `Cancelled`.

5. **[`Trade`](src/orderbook/trade.rs)** & **[`IdGenerator`](src/orderbook/id_generator.rs)**:
   - Atomic lock-free counter and timestamp-based unique ID generation for orders and trades prefixed by pair ID.

---

## Getting Started

### Prerequisites
- Rust 1.70+ (`cargo`)

### Build & Run
```bash
# Run the demo
cargo run

# Run tests
cargo test
```

---

## Usage Example

### 1. Using a Standalone Single-Symbol Matcher
```rust
use matcher::{Matcher, OrderType, Side, UDecimal};
use rust_decimal_macros::dec;

// Initialize an isolated matcher for a single symbol
let mut btc_matcher = Matcher::new("BTC-USDT".to_string(), Some(UDecimal::new(dec!(65000.0)).unwrap()));

// Submit a Limit Sell order: 2.0 BTC @ $65,100
let sell_res = btc_matcher
    .submit_order("ord-1".into(), "user-alice".into(), Side::Sell, OrderType::Limit, Some(UDecimal::new(dec!(65100.0)).unwrap()), UDecimal::new(dec!(2.0)).unwrap())
    .expect("Submit limit sell");

// Submit a matching Limit Buy order: 1.0 BTC @ $65,100
let buy_res = btc_matcher
    .submit_order("ord-2".into(), "user-bob".into(), Side::Buy, OrderType::Limit, Some(UDecimal::new(dec!(65100.0)).unwrap()), UDecimal::new(dec!(1.0)).unwrap())
    .expect("Submit limit buy");

// Inspect generated trades
for trade in &buy_res.trades {
    println!("Trade executed: {} @ {} (value: {})", trade.quantity, trade.price, trade.notional());
}

// Get top 5 levels of order book depth
let (bids, asks) = btc_matcher.get_depth(5);
```

### 2. Using the Sharded Engine for Multi-Symbol Concurrency
```rust
use matcher::{OrderType, ShardedEngine, Side, UDecimal};
use rust_decimal_macros::dec;
use std::sync::Arc;
use std::thread;

let mut engine = ShardedEngine::new();
engine.register_symbol("BTC-USDT", Some(UDecimal::new(dec!(65000.0)).unwrap())).unwrap();
engine.register_symbol("ETH-USDT", Some(UDecimal::new(dec!(3500.0)).unwrap())).unwrap();

let engine = Arc::new(engine);

// Process orders concurrently across different symbol shards without blocking
let btc_engine = Arc::clone(&engine);
let eth_engine = Arc::clone(&engine);

let btc_handle = thread::spawn(move || {
    btc_engine.submit_order("btc-1".into(), "user-alice".into(), "BTC-USDT", Side::Sell, OrderType::Limit, Some(UDecimal::new(dec!(65000.0)).unwrap()), UDecimal::new(dec!(1.0)).unwrap()).unwrap();
    btc_engine.submit_order("btc-2".into(), "user-bob".into(), "BTC-USDT", Side::Buy, OrderType::Market, None, UDecimal::new(dec!(1.0)).unwrap()).unwrap();
});

let eth_handle = thread::spawn(move || {
    eth_engine.submit_order("eth-1".into(), "user-carol".into(), "ETH-USDT", Side::Sell, OrderType::Limit, Some(UDecimal::new(dec!(3500.0)).unwrap()), UDecimal::new(dec!(10.0)).unwrap()).unwrap();
    eth_engine.submit_order("eth-2".into(), "user-dave".into(), "ETH-USDT", Side::Buy, OrderType::Limit, Some(UDecimal::new(dec!(3500.0)).unwrap()), UDecimal::new(dec!(5.0)).unwrap()).unwrap();
});

btc_handle.join().unwrap();
eth_handle.join().unwrap();
```

---

## gRPC & Protobuf Service

The matching engine exposes a high-performance **gRPC interface** powered by `tonic` and `prost`, managed with the **Buf CLI**.

### Protobuf Schemas

Protobuf definitions are modularized under [`proto/`](./proto/):
- [`proto/order/v1/order.proto`](./proto/order/v1/order.proto): Order model, enums (`Side`, `OrderType`, `OrderStatus`).
- [`proto/trade/v1/trade.proto`](./proto/trade/v1/trade.proto): Traded execution events and trade history.
- [`proto/orderbook/v1/orderbook.proto`](./proto/orderbook/v1/orderbook.proto): L2 order book depth price levels and metrics.
- [`proto/engine/v1/engine.proto`](./proto/engine/v1/engine.proto): Request/Response payload messages.
- [`proto/engine/v1/engine_service.proto`](./proto/engine/v1/engine_service.proto): `MatcherService` RPC definitions:
  - `SubmitOrder(SubmitOrderRequest) -> SubmitOrderResponse`
  - `CancelOrder(CancelOrderRequest) -> CancelOrderResponse`
  - `GetDepth(GetDepthRequest) -> GetDepthResponse`
  - `GetStats(GetStatsRequest) -> GetStatsResponse`
  - `GetTradeHistory(GetTradeHistoryRequest) -> GetTradeHistoryResponse`
  - `RegisterSymbol(RegisterSymbolRequest) -> RegisterSymbolResponse`
  - `ListSymbols(ListSymbolsRequest) -> ListSymbolsResponse`

### Protobuf Generation with Buf CLI

```bash
# Lint Protobuf schemas against standard conventions
buf lint

# Generate Prost and Tonic Rust bindings into src/gen
buf generate
```

### Starting the gRPC Server

```bash
# Start the gRPC server (default port: 50051)
cargo run

# Run with custom port
PORT=50055 cargo run

# Run in demo showcase mode
cargo run -- --demo
```

---

## Testing
Run unit and integration tests covering limit orders, market orders, order cancellations, depth snapshots, multi-threaded sharding, and gRPC endpoints:

```bash
cargo test
```

