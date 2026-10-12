use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use matcher::grpc::{MatcherServiceImpl, MatcherServiceServer};
use matcher::wal::{SyncPolicy, WalPruner, WalPrunerConfig};
use matcher::{OrderType, ShardedEngine, Side, UDecimal};
use rust_decimal_macros::dec;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let is_demo = args.len() > 1 && (args[1] == "--demo" || args[1] == "-d");

    // Configure WAL directory and sync policy
    let wal_dir_str = env::var("WAL_DIR").unwrap_or_else(|_| "data/wal".to_string());
    let wal_dir = PathBuf::from(wal_dir_str);

    let sync_policy = match env::var("WAL_SYNC_POLICY").unwrap_or_default().to_lowercase().as_str() {
        "always" => SyncPolicy::Always,
        _ => SyncPolicy::None,
    };

    // 1. Initialize ShardedEngine with WAL persistence enabled
    let engine = Arc::new(ShardedEngine::with_wal(&wal_dir, sync_policy));

    // Register initial default symbol shards (replays existing WAL / snapshot state if present)
    engine
        .register_symbol("BTC-USDT", Some(UDecimal::new(dec!(65000.0)).unwrap()))
        .expect("Failed to register BTC-USDT shard");
    engine
        .register_symbol("ETH-USDT", Some(UDecimal::new(dec!(3500.0)).unwrap()))
        .expect("Failed to register ETH-USDT shard");
    engine
        .register_symbol("SOL-USDT", Some(UDecimal::new(dec!(150.0)).unwrap()))
        .expect("Failed to register SOL-USDT shard");

    // 2. Initialize and spawn background WAL Checkpointer & Pruner (runs on configured ENV interval, default 10m)
    let pruner_config = WalPrunerConfig::from_env(&wal_dir);
    let pruner_interval = pruner_config.interval;
    let pruner = WalPruner::new(pruner_config);
    let _pruner_handle = pruner.spawn_with_engine(Arc::clone(&engine));

    if is_demo {
        run_demo(&engine);
        return Ok(());
    }

    let port = env::var("PORT").unwrap_or_else(|_| "50051".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;

    println!("=======================================================");
    println!("  Matcher High-Performance gRPC Service");
    println!("=======================================================");
    println!("  Listening on: http://{addr}");
    println!("  Registered Symbol Shards: {:?}", engine.list_symbols());
    println!("  WAL Directory: {}", wal_dir.display());
    println!("  WAL Sync Policy: {sync_policy:?}");
    println!("  WAL Pruner Interval: {pruner_interval:?}");
    println!("  gRPC Service: engine.v1.MatcherService");
    println!("=======================================================\n");

    let service = MatcherServiceImpl::new(engine);

    tonic::transport::Server::builder()
        .add_service(MatcherServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}

fn run_demo(engine: &ShardedEngine) {
    println!("=======================================================");
    println!("  Running Matcher Sharded Execution Demo");
    println!("=======================================================");
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
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(64900.0)).unwrap()),
            UDecimal::new(dec!(1.5)).unwrap(),
        )
        .expect("Place limit buy");

    println!("  -> Placed resting ask: 2.5 BTC @ 65100.0");
    println!("  -> Placed resting bid: 1.5 BTC @ 64900.0");

    println!("\n--- [BTC-USDT Shard] Crossing the book with market order ---");
    let match_res = engine
        .submit_order(
            "btc-ord-003".into(),
            "user-trader-charlie".into(),
            "BTC-USDT",
            Side::Buy,
            OrderType::Market,
            None,
            UDecimal::new(dec!(1.0)).unwrap(),
        )
        .expect("Place market buy");

    for trade in &match_res.trades {
        println!(
            "  -> EXECUTION: Traded {} BTC @ {} USDT between Maker [{}] and Taker [{}]",
            trade.quantity, trade.price, trade.maker_order_id, trade.taker_order_id
        );
    }

    // --- Shard 2: ETH-USDT Activity ---
    println!("\n--- [ETH-USDT Shard] Placing and executing trades ---");
    engine
        .submit_order(
            "eth-ord-001".into(),
            "user-trader-dan".into(),
            "ETH-USDT",
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(3500.0)).unwrap()),
            UDecimal::new(dec!(10.0)).unwrap(),
        )
        .expect("Place ETH sell");
    let eth_match = engine
        .submit_order(
            "eth-ord-002".into(),
            "user-trader-erin".into(),
            "ETH-USDT",
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(3500.0)).unwrap()),
            UDecimal::new(dec!(5.0)).unwrap(),
        )
        .expect("Place ETH buy");

    for trade in &eth_match.trades {
        println!(
            "  -> EXECUTION: Traded {} ETH @ {} USDT between Maker [{}] and Taker [{}]",
            trade.quantity, trade.price, trade.maker_order_id, trade.taker_order_id
        );
    }

    // --- Inspect Snapshots & Depths ---
    println!("\n--- Snapshots for Shards ---");
    for symbol in ["BTC-USDT", "ETH-USDT", "SOL-USDT"] {
        let stats = engine.get_stats(symbol).unwrap();
        let (bids, asks) = engine.get_depth(symbol, 3).unwrap();
        println!("Symbol: {symbol}");
        println!(
            "  Stats: Last Traded Price: {:?}, Volume Traded: {}, Total Trades: {}",
            stats.last_traded_price, stats.total_volume_traded, stats.total_trades_count
        );
        println!("  Book Depth (Top 3):");
        println!("    Asks: {asks:?}");
        println!("    Bids: {bids:?}");
    }
}
