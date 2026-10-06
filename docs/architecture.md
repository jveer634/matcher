# Matcher Architecture & System Relationships

Visual representation of components, data flow, concurrency model, and type boundaries.

---

## 1. End-to-End System & Request Flow

```
                      +------------------------------------------+
                      |        gRPC Client / Trading API         |
                      +--------------------+---------------------+
                                           |
                   SubmitOrder / CancelOrder / GetDepth / GetStats
                                           |
                                           v
+-----------------------------------------------------------------------------------+
|  TRANSPORT & SERVICE LAYER (src/grpc)                                             |
|                                                                                   |
|   +---------------------------------------------------------------------------+   |
|   |                        Tonic gRPC Server (:50051)                         |   |
|   +-------------------------------------+-------------------------------------+   |
|                                         |                                         |
|                                         v                                         |
|   +---------------------------------------------------------------------------+   |
|   |                  MatcherServiceImpl (gRPC Handler)                        |   |
|   +-------------------------------------+-------------------------------------+   |
|                                         |                                         |
|                   Protobuf <===========> Domain Conversions                       |
|                   (TryFrom / From validates non-negative UDecimal)                |
+-----------------------------------------+-----------------------------------------+
                                          |
                        submit_order(symbol, side, price, qty)
                                          |
                                          v
+-----------------------------------------------------------------------------------+
|  SHARDED ENGINE ROUTER (src/engine.rs)                                            |
|                                                                                   |
|   ShardedEngine { shards: RwLock<HashMap<Symbol, Arc<RwLock<Matcher>>>> }         |
|   - Dynamic Symbol Registry                                                       |
|   - Routes orders to dedicated isolated symbol shards                             |
+--------------------+--------------------+--------------------+--------------------+
                     |                    |                    |
       Acquire Lock  |      Acquire Lock  |      Acquire Lock  |
                     v                    v                    v
     +---------------+----+       +-------+------------+       +---------------+----+
     |   Matcher Shard    |       |   Matcher Shard    |       |   Matcher Shard    |
     |    "BTC-USDT"      |       |    "ETH-USDT"      |       |    "SOL-USDT"      |
     +---------+----------+       +--------------------+       +--------------------+
               |
               v
+-----------------------------------------------------------------------------------+
|  CORE MATCHING ENGINE (src/orderbook/)                                            |
|                                                                                   |
|  OrderBook (Price-Time Priority FIFO)                                             |
|  +-------------------------------------+  +------------------------------------+  |
|  |  Bids Queue (Sorted Descending)     |  |  Asks Queue (Sorted Ascending)     |  |
|  |  BTreeMap<UDecimal, VecDeque<Order>>|  |  BTreeMap<UDecimal, VecDeque<Order>>|  |
|  +-------------------------------------+  +------------------------------------+  |
|                                                                                   |
|  +-------------------------------------+  +------------------------------------+  |
|  |  Order Index (O(1) Fast Lookup)      |  |  Execution History & Metrics       |  |
|  |  HashMap<OrderId, Order>            |  |  Vec<Trade>, Volumes, Notional     |  |
|  +-------------------------------------+  +------------------------------------+  |
+-----------------------------------------------------------------------------------+
```

---

## 2. Horizontal Symbol Sharding (Lock Isolation)

Operations on `BTC-USDT` execute concurrently in parallel with `ETH-USDT` with zero cross-symbol lock contention.

```
Trader Alice (BTC-USDT)                            Trader Bob (ETH-USDT)
       |                                                    |
       | 1. Submit Limit Sell ($65,100)                     | 1. Submit Market Buy
       v                                                    v
+---------------+                                    +---------------+
| ShardedEngine |                                    | ShardedEngine |
+-------+-------+                                    +-------+-------+
        |                                                    |
        | [Acquires BTC Shard Lock]                          | [Acquires ETH Shard Lock]
        v                                                    v
+---------------+                                    +---------------+
| Matcher Shard |                                    | Matcher Shard |
|   BTC-USDT    |                                    |   ETH-USDT    |
+-------+-------+                                    +-------+-------+
        |                                                    |
        | 2. Insert to Resting Bids                          | 2. Match Crossing Asks
        |    (Zero trades generated)                         |    (Executes trade immediately)
        v                                                    v
+---------------+                                    +---------------+
| OrderBook BTC |                                    | OrderBook ETH |
+-------+-------+                                    +-------+-------+
        |                                                    |
        | 3. Return MatchResult (Resting)                    | 3. Return MatchResult (Traded)
        v                                                    v
  SubmitOrderResponse                                  SubmitOrderResponse
```

---

## 3. OrderBook Data Structures & Memory Layout

```
                              OrderBook
  +------------------------------------------------------------------+
  | symbol: "BTC-USDT"                                               |
  | last_traded_price: Some(UDecimal(65100.00))                      |
  | buy_volume:  UDecimal(3.0)                                       |
  | sell_volume: UDecimal(1.5)                                       |
  +---------------------------------+--------------------------------+
                                    |
            +-----------------------+-----------------------+
            |                                               |
            v                                               v
    Bids (BTreeMap Descending)                      Asks (BTreeMap Ascending)
+-------------------------------+               +-------------------------------+
| Price: 64900.00               |               | Price: 65100.00               |
|  -> [Order: alice, 2.0 BTC]   |               |  -> [Order: carol, 0.5 BTC]   |
|  -> [Order: bob,   1.0 BTC]   |               | Price: 65200.00               |
| Price: 64800.00               |               |  -> [Order: dave,  1.0 BTC]   |
|  -> [Order: eve,   5.0 BTC]   |               +-------------------------------+
+-------------------------------+
            |
            +-----------------------+
                                    v
                        O(1) Fast Order Index
            +-----------------------------------------------+
            | HashMap<OrderId, Order>                       |
            |  "ord-001" => Order { Alice, Buy, 64900.0 }   |
            |  "ord-002" => Order { Bob,   Buy, 64900.0 }   |
            |  "ord-003" => Order { Carol, Sell, 65100.0 }  |
            +-----------------------------------------------+
```

---

## 4. Protobuf Pipeline & Boundary Translation

```
   SCHEMAS (proto/)                    BUILD PIPELINE (buf)                 GENERATED RUST (src/gen/)
+-----------------------+          +-------------------------+          +-------------------------------+
| order/v1/order.proto  |          | buf.yaml                |          | gen::order::v1::Order         |
| trade/v1/trade.proto  | -------> | (Modules / Linting)     | -------> | gen::trade::v1::Trade         |
| orderbook/v1/*.proto  |          |                         |          | gen::orderbook::v1::BookLevel |
| engine/v1/engine.proto|          | buf.gen.yaml            |          | gen::engine::v1::*            |
| engine_service.proto  |          | (Prost & Tonic Plugins) |          | MatcherServiceClient / Server |
+-----------------------+          +-------------------------+          +---------------+---------------+
                                                                                        |
                                                                                        |
                                                                                        v
                                                                        +-------------------------------+
                                                                        | BOUNDARY LAYER                |
                                                                        | src/grpc/conversions.rs       |
                                                                        |                               |
                                                                        | - TryFrom<ProtoOrder> -> Order|
                                                                        | - From<&Order> -> ProtoOrder  |
                                                                        | - Validates UDecimal > 0      |
                                                                        | - Strips UNSPECIFIED Enums    |
                                                                        +---------------+---------------+
                                                                                        |
                                                                                        |
                                                                                        v
                                                                        +-------------------------------+
                                                                        | CORE DOMAIN (src/orderbook/)  |
                                                                        |                               |
                                                                        | - Order, Trade, BookLevel     |
                                                                        | - Side, OrderType, OrderStatus|
                                                                        | - Invariant-Safe UDecimal     |
                                                                        +-------------------------------+
```
