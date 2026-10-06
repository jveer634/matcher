use matcher::{OrderType, ShardedEngine, Side, UDecimal};
use rust_decimal_macros::dec;

fn main() {
    println!("=======================================================");
    println!("  Starting Sharded High-Performance Matching Engine");
    println!("  (Each shard dedicated to an isolated symbol copy)   ");
    println!("=======================================================\n");

    let mut engine = ShardedEngine::new();

    // Register independent per-symbol shards
    engine
        .register_symbol(
            "BTC-USDT",
            Some(UDecimal::new(dec!(65000.0)).unwrap()),
        )
        .expect("Failed to register BTC-USDT shard");
    engine
        .register_symbol(
            "ETH-USDT",
            Some(UDecimal::new(dec!(3500.0)).unwrap()),
        )
        .expect("Failed to register ETH-USDT shard");
    engine
        .register_symbol(
            "SOL-USDT",
            Some(UDecimal::new(dec!(150.0)).unwrap()),
        )
        .expect("Failed to register SOL-USDT shard");

    println!("Registered Shards: {:?}", engine.list_symbols());

    // --- Shard 1: BTC-USDT Activity ---
    println!("\n--- [BTC-USDT Shard] Placing resting limit orders ---");
    engine
        .submit_order(
            "btc-ord-001".into(),
            "user-trader-alice".into(),
            "BTC-USDT",
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(65100.0)).unwrap()),
            UDecimal::new(dec!(2.5)).unwrap(),
        )
        .expect("Place limit sell");
    engine
        .submit_order(
            "btc-ord-002".into(),
            "user-trader-bob".into(),
            "BTC-USDT",
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(65200.0)).unwrap()),
            UDecimal::new(dec!(1.0)).unwrap(),
        )
        .expect("Place limit sell");
    engine
        .submit_order(
            "btc-ord-003".into(),
            "user-trader-carol".into(),
            "BTC-USDT",
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(64900.0)).unwrap()),
            UDecimal::new(dec!(3.0)).unwrap(),
        )
        .expect("Place limit buy");

    println!("--- [BTC-USDT Shard] Submitting matching Market Buy ---");
    let btc_match = engine
        .submit_order(
            "btc-ord-004".into(),
            "user-trader-dave".into(),
            "BTC-USDT",
            Side::Buy,
            OrderType::Market,
            None,
            UDecimal::new(dec!(2.0)).unwrap(),
        )
        .expect("Place market buy");
    println!(
        "BTC-USDT Execution Trades Generated ({}):",
        btc_match.trades.len()
    );
    for t in &btc_match.trades {
        println!(
            "  -> Trade ID: {}, Price: {}, Quantity: {}, Value: {}, Maker: {}, Taker: {}",
            t.id,
            t.price,
            t.quantity,
            t.notional(),
            t.maker_order_id,
            t.taker_order_id
        );
    }

    // --- Shard 2: ETH-USDT Activity ---
    println!("\n--- [ETH-USDT Shard] Placing Limit Sell & Crossing Limit Buy ---");
    engine
        .submit_order(
            "eth-ord-001".into(),
            "user-trader-alice".into(),
            "ETH-USDT",
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(3500.0)).unwrap()),
            UDecimal::new(dec!(10.0)).unwrap(),
        )
        .expect("Place ETH limit sell");

    let eth_match = engine
        .submit_order(
            "eth-ord-002".into(),
            "user-trader-eve".into(),
            "ETH-USDT",
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(3500.0)).unwrap()),
            UDecimal::new(dec!(4.0)).unwrap(),
        )
        .expect("Place ETH limit buy");
    println!(
        "ETH-USDT Execution Trades Generated ({}):",
        eth_match.trades.len()
    );
    for t in &eth_match.trades {
        println!(
            "  -> Trade ID: {}, Price: {}, Quantity: {}, Value: {}",
            t.id,
            t.price,
            t.quantity,
            t.notional()
        );
    }

    // --- Inspect Snapshots & Depths ---
    println!("\n--- Snapshots for Shards ---");
    for symbol in ["BTC-USDT", "ETH-USDT", "SOL-USDT"] {
        let stats = engine.get_stats(symbol).unwrap();
        let (bids, asks) = engine.get_depth(symbol, 3).unwrap();
        println!("Symbol: {}", symbol);
        println!(
            "  Stats: Last Traded Price: {:?}, Volume Traded: {}, Total Trades: {}",
            stats.last_traded_price, stats.total_volume_traded, stats.total_trades_count
        );
        println!("  Book Depth (Top 3):");
        println!("    Asks: {:?}", asks);
        println!("    Bids: {:?}", bids);
    }
}
