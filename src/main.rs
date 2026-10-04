use matcher::{
    OrderType, ShardedEngine,
};

fn main() {
    println!("=======================================================");
    println!("  Starting Sharded High-Performance Matching Engine");
    println!("  (Each shard dedicated to an isolated symbol copy)   ");
    println!("=======================================================\n");

    let mut engine = ShardedEngine::new();

    // Register independent per-symbol shards
    engine
        .register_symbol("BTC-USDT", Some(65000.0))
        .expect("Failed to register BTC-USDT shard");
    engine
        .register_symbol("ETH-USDT", Some(3500.0))
        .expect("Failed to register ETH-USDT shard");
    engine
        .register_symbol("SOL-USDT", Some(150.0))
        .expect("Failed to register SOL-USDT shard");

    println!("Registered Shards: {:?}", engine.list_symbols());

    // --- Shard 1: BTC-USDT Activity ---
    println!("\n--- [BTC-USDT Shard] Placing resting limit orders ---");
    engine
        .submit_order("BTC-USDT", OrderType::LimitSell, Some(65100.0), 2.5)
        .expect("Place limit sell");
    engine
        .submit_order("BTC-USDT", OrderType::LimitSell, Some(65200.0), 1.0)
        .expect("Place limit sell");
    engine
        .submit_order("BTC-USDT", OrderType::LimitBuy, Some(64900.0), 3.0)
        .expect("Place limit buy");

    println!("--- [BTC-USDT Shard] Submitting matching Market Buy ---");
    let btc_match = engine
        .submit_order("BTC-USDT", OrderType::Buy, None, 2.0)
        .expect("Place market buy");
    println!(
        "BTC-USDT Execution Trades Generated ({}):",
        btc_match.trades.len()
    );
    for t in &btc_match.trades {
        println!(
            "  -> Trade ID: {}, Price: {}, Quantity: {}, Maker: {}, Taker: {}",
            t.id, t.price, t.quantity, t.maker_order_id, t.taker_order_id
        );
    }

    // --- Shard 2: ETH-USDT Activity ---
    println!("\n--- [ETH-USDT Shard] Placing Limit Sell & Crossing Limit Buy ---");
    engine
        .submit_order("ETH-USDT", OrderType::LimitSell, Some(3500.0), 10.0)
        .expect("Place ETH limit sell");

    let eth_match = engine
        .submit_order("ETH-USDT", OrderType::LimitBuy, Some(3500.0), 4.0)
        .expect("Place ETH limit buy");
    println!(
        "ETH-USDT Execution Trades Generated ({}):",
        eth_match.trades.len()
    );
    for t in &eth_match.trades {
        println!(
            "  -> Trade ID: {}, Price: {}, Quantity: {}",
            t.id, t.price, t.quantity
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
