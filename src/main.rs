use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use matcher::wal::WalPruner;
use matcher::{AppConfig, Cli, OrderType, Server, ShardedEngine, Side, UDecimal};
use rust_decimal_macros::dec;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // 1. Load and validate application configuration
    let mut config = AppConfig::load_with_path(cli.config.as_deref())?;
    if let Some(port) = cli.port {
        config.server.port = port;
        config
            .server
            .validate()
            .map_err(matcher::ConfigError::Validation)?;
    }

    // 2. Initialize ShardedEngine (matching engine) with WAL persistence
    let engine = Arc::new(ShardedEngine::with_wal(
        &config.wal.dir,
        config.wal.sync_policy,
    ));

    // Register configured symbol shards (replays existing WAL / snapshot state if present)
    for sym_cfg in &config.symbols {
        let initial_price = sym_cfg
            .initial_udecimal_price()
            .map_err(|e| format!("Invalid initial price for {}: {e}", sym_cfg.symbol))?;
        engine
            .register_symbol(&sym_cfg.symbol, initial_price)
            .map_err(|e| format!("Failed to register {} shard: {e}", sym_cfg.symbol))?;
    }

    // 3. Initialize WAL Pruner service
    let pruner_config = config.pruner.to_wal_pruner_config(&config.wal.dir);
    let pruner = WalPruner::new(pruner_config);

    // 4. Spawn background WAL checkpointer & pruner attached to engine
    let pruner_handle = pruner.clone().spawn_with_engine(Arc::clone(&engine));

    // 5. Initialize Server and attach all services
    let addr: SocketAddr = config.server.socket_addr()?;
    let server = Server::new(addr, engine, pruner, Some(pruner_handle));

    if cli.demo {
        run_demo(server.engine());
        return Ok(());
    }

    // 6. Run the server
    server.serve().await?;

    Ok(())
}

fn run_demo(engine: &ShardedEngine) {
    let run_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    println!("=======================================================");
    println!("  Running Matcher Sharded Execution Demo");
    println!("=======================================================");
    println!("Registered Shards: {:?}", engine.list_symbols());

    // --- Shard 1: BTC-USDT Activity ---
    println!("\n--- [BTC-USDT Shard] Placing resting limit orders ---");
    engine
        .submit_order(
            format!("btc-ord-{run_id}-001"),
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
            format!("btc-ord-{run_id}-002"),
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
            format!("btc-ord-{run_id}-003"),
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
            format!("eth-ord-{run_id}-001"),
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
            format!("eth-ord-{run_id}-002"),
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
