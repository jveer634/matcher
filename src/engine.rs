use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::matcher::{MatchResult, Matcher, MatcherStats};
use crate::orderbook::{BookLevel, Order, OrderType, Side, Trade, UDecimal};
use crate::wal::{PruneStats, SnapshotWriter, SyncPolicy, WalPruner};

/// Command sent to a sharded symbol matcher
#[derive(Debug, Clone)]
pub enum ShardCommand {
    SubmitOrder {
        id: String,
        user_id: String,
        symbol: String,
        side: Side,
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
    },
    CancelOrder {
        symbol: String,
        order_id: String,
        user_id: String,
    },
}

/// Sharded Matching Engine managing independent, isolated per-symbol `Matcher` shards.
/// Each symbol is executed completely independently, eliminating cross-symbol lock contention.
#[derive(Debug, Default)]
pub struct ShardedEngine {
    shards: RwLock<HashMap<String, Arc<RwLock<Matcher>>>>,
    wal_dir: Option<PathBuf>,
    sync_policy: Option<SyncPolicy>,
}

impl ShardedEngine {
    /// Creates a purely in-memory sharded matching engine without WAL persistence.
    #[must_use]
    pub fn new() -> Self {
        ShardedEngine {
            shards: RwLock::new(HashMap::new()),
            wal_dir: None,
            sync_policy: None,
        }
    }

    /// Creates a persistent sharded matching engine backed by Write-Ahead Logs stored in `wal_dir`.
    pub fn with_wal(wal_dir: impl AsRef<Path>, sync_policy: SyncPolicy) -> Self {
        ShardedEngine {
            shards: RwLock::new(HashMap::new()),
            wal_dir: Some(wal_dir.as_ref().to_path_buf()),
            sync_policy: Some(sync_policy),
        }
    }

    /// Base directory where WAL logs are stored, if persistence is enabled.
    #[must_use]
    pub fn wal_dir(&self) -> Option<&Path> {
        self.wal_dir.as_deref()
    }

    /// Register a new symbol shard
    pub fn register_symbol(&self, symbol: &str, listing_price: Option<UDecimal>) -> Result<(), String> {
        let mut shards = self.shards.write().map_err(|e| e.to_string())?;
        if shards.contains_key(symbol) {
            return Err(format!("Symbol shard '{symbol}' already registered"));
        }

        let matcher = match (&self.wal_dir, self.sync_policy) {
            (Some(dir), Some(policy)) => {
                let wal_path = dir.join(format!("{symbol}.wal"));
                Matcher::with_wal(symbol.to_string(), listing_price, wal_path, policy)?
            }
            _ => Matcher::new(symbol.to_string(), listing_price),
        };

        shards.insert(symbol.to_string(), Arc::new(RwLock::new(matcher)));
        Ok(())
    }

    /// Recovers and registers multiple symbol shards concurrently in parallel tasks.
    ///
    /// For each symbol shard:
    /// - Discovers and loads the latest on-disk binary snapshot if available.
    /// - Streams and replays delta WAL records after the snapshot checkpoint.
    /// - Opens or resumes the append-only WAL writer for future operations.
    ///
    /// Executes all recovery agents concurrently using asynchronous blocking tasks.
    pub async fn register_symbols_parallel(
        &self,
        symbol_specs: Vec<(String, Option<UDecimal>)>,
    ) -> Result<usize, String> {
        if symbol_specs.is_empty() {
            return Ok(0);
        }

        // Verify no duplicate symbols in the input list
        let mut seen = std::collections::HashSet::new();
        for (sym, _) in &symbol_specs {
            if !seen.insert(sym) {
                return Err(format!("Duplicate symbol '{sym}' provided to parallel recovery"));
            }
        }

        // Ensure none of the symbols are already registered
        {
            let current = self.shards.read().map_err(|e| e.to_string())?;
            for (sym, _) in &symbol_specs {
                if current.contains_key(sym) {
                    return Err(format!("Symbol shard '{sym}' already registered"));
                }
            }
        }

        let mut tasks = Vec::with_capacity(symbol_specs.len());
        for (symbol, listing_price) in symbol_specs {
            let wal_dir = self.wal_dir.clone();
            let sync_policy = self.sync_policy;

            tasks.push(tokio::task::spawn_blocking(move || {
                let matcher = match (&wal_dir, sync_policy) {
                    (Some(dir), Some(policy)) => {
                        let wal_path = dir.join(format!("{symbol}.wal"));
                        Matcher::with_wal(symbol.clone(), listing_price, wal_path, policy)?
                    }
                    _ => Matcher::new(symbol.clone(), listing_price),
                };
                Ok::<(String, Matcher), String>((symbol, matcher))
            }));
        }

        let mut recovered = Vec::with_capacity(tasks.len());
        for task in tasks {
            let (sym, matcher) = task
                .await
                .map_err(|join_err| format!("Recovery agent panicked: {join_err}"))??;
            recovered.push((sym, matcher));
        }

        let count = recovered.len();
        let mut shards = self.shards.write().map_err(|e| e.to_string())?;
        for (sym, matcher) in recovered {
            shards.insert(sym, Arc::new(RwLock::new(matcher)));
        }

        Ok(count)
    }

    /// Check if a symbol is registered
    pub fn has_symbol(&self, symbol: &str) -> bool {
        self.shards.read().map(|s| s.contains_key(symbol)).unwrap_or(false)
    }

    /// Get list of all registered symbols
    pub fn list_symbols(&self) -> Vec<String> {
        self.shards
            .read()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Get a cloned Arc handle to a specific symbol shard for direct concurrent access
    pub fn get_shard(&self, symbol: &str) -> Result<Arc<RwLock<Matcher>>, String> {
        let shards = self.shards.read().map_err(|e| e.to_string())?;
        shards
            .get(symbol)
            .cloned()
            .ok_or_else(|| format!("Unknown symbol shard '{}'", symbol))
    }

    /// Submit an order to the dedicated symbol shard
    pub fn submit_order(
        &self,
        id: String,
        user_id: String,
        symbol: &str,
        side: Side,
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
    ) -> Result<MatchResult, String> {
        let shard = self.get_shard(symbol)?;
        let mut matcher = shard.write().map_err(|e| e.to_string())?;
        matcher.submit_order(id, user_id, side, order_type, price, quantity)
    }

    /// Cancel an order in the dedicated symbol shard
    pub fn cancel_order(&self, symbol: &str, order_id: &str, user_id: &str) -> Result<Order, String> {
        let shard = self.get_shard(symbol)?;
        let mut matcher = shard.write().map_err(|e| e.to_string())?;
        matcher.cancel_order(order_id, user_id)
    }

    /// Get a cloned order from the dedicated symbol shard
    pub fn get_order(&self, symbol: &str, order_id: &str) -> Result<Option<Order>, String> {
        let shard = self.get_shard(symbol)?;
        let matcher = shard.read().map_err(|e| e.to_string())?;
        Ok(matcher.get_order(order_id).cloned())
    }

    /// Get order depth for a given symbol
    pub fn get_depth(&self, symbol: &str, limit: usize) -> Result<(Vec<BookLevel>, Vec<BookLevel>), String> {
        let shard = self.get_shard(symbol)?;
        let matcher = shard.read().map_err(|e| e.to_string())?;
        Ok(matcher.get_depth(limit))
    }

    /// Get statistics for a specific symbol shard
    pub fn get_stats(&self, symbol: &str) -> Result<MatcherStats, String> {
        let shard = self.get_shard(symbol)?;
        let matcher = shard.read().map_err(|e| e.to_string())?;
        Ok(matcher.stats())
    }

    /// Get trade history for a given symbol
    pub fn get_trade_history(&self, symbol: &str) -> Result<Vec<Trade>, String> {
        let shard = self.get_shard(symbol)?;
        let matcher = shard.read().map_err(|e| e.to_string())?;
        Ok(matcher.trade_history().to_vec())
    }

    /// Captures in-memory snapshots for all registered symbol shards,
    /// releasing shard locks in microseconds, and persists binary snapshot files
    /// to disk concurrently via asynchronous blocking tasks.
    ///
    /// Also emits a [`SnapshotMarker`](crate::wal::SnapshotMarkerSbe) to each shard's WAL stream recording the checkpoint.
    pub async fn create_snapshots(&self, target_dir: impl AsRef<Path>) -> Result<usize, String> {
        let target_dir = target_dir.as_ref().to_path_buf();
        let shards: Vec<(String, Arc<RwLock<Matcher>>)> = {
            let map = self.shards.read().map_err(|e| e.to_string())?;
            map.iter().map(|(k, v)| (k.clone(), Arc::clone(v))).collect()
        };

        if shards.is_empty() {
            return Ok(0);
        }

        // 1. Capture in-memory snapshots with microsecond lock duration
        let mut snapshot_tasks = Vec::with_capacity(shards.len());

        for (symbol, shard) in shards {
            let (snapshot, snap_seq) = {
                let mut matcher = shard.write().map_err(|e| e.to_string())?;
                let snapshot = matcher.capture_snapshot();
                let seq = snapshot.sequence;
                let open_orders_count = snapshot.open_orders.len() as u64;
                let _ = matcher.emit_snapshot_marker(seq, open_orders_count)?;
                (snapshot, seq)
            };

            let out_dir = target_dir.clone();
            snapshot_tasks.push(tokio::task::spawn_blocking(move || {
                let snap_filename = SnapshotWriter::snapshot_filename(&symbol, snap_seq);
                let snap_path = out_dir.join(snap_filename);
                SnapshotWriter::write_snapshot(&snap_path, &snapshot)?;
                Ok::<(), String>(())
            }));
        }

        // 2. Await all disk writes concurrently
        let mut created_count = 0;
        for task in snapshot_tasks {
            match task.await {
                Ok(Ok(())) => created_count += 1,
                Ok(Err(e)) => return Err(e),
                Err(join_err) => return Err(format!("Snapshot task panicked: {join_err}")),
            }
        }

        Ok(created_count)
    }

    /// Executes a full checkpoint and pruning cycle:
    /// 1. Concurrently writes snapshots for all shards.
    /// 2. Runs the WAL pruner to reclaim obsolete segments and old snapshots.
    pub async fn checkpoint_and_prune(&self, pruner: &WalPruner) -> Result<PruneStats, String> {
        let base_dir = self.wal_dir.as_deref().unwrap_or_else(|| &pruner.config().base_dir);

        let _ = self.create_snapshots(base_dir).await?;
        pruner.prune_once()
    }

    /// Executes parallel shutdown agents across all active symbol shards:
    /// 1. Concurrently writes snapshots for all symbol shards in parallel tasks (if `create_final_snapshot` is true).
    /// 2. Concurrently flushes all shard WAL writer buffers to disk in parallel tasks.
    pub async fn shutdown_parallel(&self, create_final_snapshot: bool) -> Result<(), String> {
        let shards: Vec<(String, Arc<RwLock<Matcher>>)> = {
            let map = self.shards.read().map_err(|e| e.to_string())?;
            map.iter().map(|(k, v)| (k.clone(), Arc::clone(v))).collect()
        };

        if shards.is_empty() {
            return Ok(());
        }

        // 1. Parallel final snapshot write across all shards if requested
        if create_final_snapshot {
            if let Some(dir) = &self.wal_dir {
                let _ = self.create_snapshots(dir).await?;
            }
        }

        // 2. Parallel WAL writer buffer flush across all shards
        let mut flush_tasks = Vec::with_capacity(shards.len());
        for (symbol, shard) in shards {
            flush_tasks.push(tokio::task::spawn_blocking(move || {
                let mut matcher = shard.write().map_err(|e| e.to_string())?;
                matcher
                    .flush_wal()
                    .map_err(|e| format!("Failed to flush WAL for {symbol}: {e}"))?;
                Ok::<(), String>(())
            }));
        }

        for task in flush_tasks {
            task.await
                .map_err(|join_err| format!("Shutdown flush agent panicked: {join_err}"))??;
        }

        Ok(())
    }

    /// Flushes the Write-Ahead Log writer buffers for all registered symbol shards.
    pub fn flush_all(&self) -> Result<(), String> {
        let shards = self.shards.read().map_err(|e| e.to_string())?;
        for (symbol, shard) in shards.iter() {
            let mut matcher = shard.write().map_err(|e| e.to_string())?;
            matcher
                .flush_wal()
                .map_err(|e| format!("Failed to flush WAL for {symbol}: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use std::thread;

    #[test]
    pub fn test_sharded_engine_concurrent_symbols() {
        let engine = ShardedEngine::new();
        engine
            .register_symbol("BTC-USDT", Some(UDecimal::new(dec!(60000.0)).unwrap()))
            .unwrap();
        engine
            .register_symbol("ETH-USDT", Some(UDecimal::new(dec!(3000.0)).unwrap()))
            .unwrap();
        engine
            .register_symbol("SOL-USDT", Some(UDecimal::new(dec!(150.0)).unwrap()))
            .unwrap();

        let engine = Arc::new(engine);

        // Spawn parallel threads working on different symbol shards
        let mut handles = vec![];

        let btc_engine = Arc::clone(&engine);
        handles.push(thread::spawn(move || {
            btc_engine
                .submit_order(
                    "btc-sell-1".into(),
                    "user-btc-maker".into(),
                    "BTC-USDT",
                    Side::Sell,
                    OrderType::Limit,
                    Some(UDecimal::new(dec!(60000.0)).unwrap()),
                    UDecimal::new(dec!(1.0)).unwrap(),
                )
                .unwrap();
            let res = btc_engine
                .submit_order(
                    "btc-buy-1".into(),
                    "user-btc-taker".into(),
                    "BTC-USDT",
                    Side::Buy,
                    OrderType::Limit,
                    Some(UDecimal::new(dec!(60000.0)).unwrap()),
                    UDecimal::new(dec!(1.0)).unwrap(),
                )
                .unwrap();
            assert_eq!(res.trades.len(), 1);
            assert_eq!(res.trades[0].maker_order_id, "btc-sell-1");
            assert_eq!(res.trades[0].taker_order_id, "btc-buy-1");
        }));

        let eth_engine = Arc::clone(&engine);
        handles.push(thread::spawn(move || {
            eth_engine
                .submit_order(
                    "eth-sell-1".into(),
                    "user-eth-maker".into(),
                    "ETH-USDT",
                    Side::Sell,
                    OrderType::Limit,
                    Some(UDecimal::new(dec!(3000.0)).unwrap()),
                    UDecimal::new(dec!(10.0)).unwrap(),
                )
                .unwrap();
            let res = eth_engine
                .submit_order(
                    "eth-buy-1".into(),
                    "user-eth-taker".into(),
                    "ETH-USDT",
                    Side::Buy,
                    OrderType::Limit,
                    Some(UDecimal::new(dec!(3000.0)).unwrap()),
                    UDecimal::new(dec!(5.0)).unwrap(),
                )
                .unwrap();
            assert_eq!(res.trades.len(), 1);
        }));

        let sol_engine = Arc::clone(&engine);
        handles.push(thread::spawn(move || {
            sol_engine
                .submit_order(
                    "sol-sell-1".into(),
                    "user-sol-maker".into(),
                    "SOL-USDT",
                    Side::Sell,
                    OrderType::Limit,
                    Some(UDecimal::new(dec!(150.0)).unwrap()),
                    UDecimal::new(dec!(100.0)).unwrap(),
                )
                .unwrap();
            let res = sol_engine
                .submit_order(
                    "sol-buy-1".into(),
                    "user-sol-taker".into(),
                    "SOL-USDT",
                    Side::Buy,
                    OrderType::Market,
                    None,
                    UDecimal::new(dec!(40.0)).unwrap(),
                )
                .unwrap();
            assert_eq!(res.trades.len(), 1);
        }));

        for h in handles {
            h.join().unwrap();
        }

        let btc_stats = engine.get_stats("BTC-USDT").unwrap();
        let eth_stats = engine.get_stats("ETH-USDT").unwrap();
        let sol_stats = engine.get_stats("SOL-USDT").unwrap();

        assert_eq!(btc_stats.total_volume_traded, UDecimal::new(dec!(1.0)).unwrap());
        assert_eq!(eth_stats.total_volume_traded, UDecimal::new(dec!(5.0)).unwrap());
        assert_eq!(sol_stats.total_volume_traded, UDecimal::new(dec!(40.0)).unwrap());
    }

    #[tokio::test]
    async fn test_parallel_recovery_and_shutdown_agents() {
        let temp_dir = std::env::temp_dir().join(format!(
            "parallel_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        // 1. Initial run: parallel register 3 symbols and place orders
        let engine = Arc::new(ShardedEngine::with_wal(&temp_dir, SyncPolicy::Always));
        let specs = vec![
            ("BTC-USDT".to_string(), Some(UDecimal::new(dec!(65000.0)).unwrap())),
            ("ETH-USDT".to_string(), Some(UDecimal::new(dec!(3500.0)).unwrap())),
            ("SOL-USDT".to_string(), Some(UDecimal::new(dec!(150.0)).unwrap())),
        ];

        let count = engine.register_symbols_parallel(specs.clone()).await.unwrap();
        assert_eq!(count, 3);

        // Place resting limit orders on all 3 shards
        engine
            .submit_order(
                "btc-1".into(),
                "user-1".into(),
                "BTC-USDT",
                Side::Sell,
                OrderType::Limit,
                Some(UDecimal::new(dec!(65100.0)).unwrap()),
                UDecimal::new(dec!(2.0)).unwrap(),
            )
            .unwrap();
        engine
            .submit_order(
                "eth-1".into(),
                "user-2".into(),
                "ETH-USDT",
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(3500.0)).unwrap()),
                UDecimal::new(dec!(10.0)).unwrap(),
            )
            .unwrap();
        engine
            .submit_order(
                "sol-1".into(),
                "user-3".into(),
                "SOL-USDT",
                Side::Sell,
                OrderType::Limit,
                Some(UDecimal::new(dec!(150.0)).unwrap()),
                UDecimal::new(dec!(50.0)).unwrap(),
            )
            .unwrap();

        // 2. Parallel shutdown: concurrently snapshots & flushes all shard WALs
        engine.shutdown_parallel(true).await.unwrap();

        // 3. New engine instance: test parallel recovery agents restoring all 3 shards concurrently
        let recovered_engine = Arc::new(ShardedEngine::with_wal(&temp_dir, SyncPolicy::Always));
        let recovered_count = recovered_engine.register_symbols_parallel(specs).await.unwrap();
        assert_eq!(recovered_count, 3);

        // Verify open orders were recovered across all shards
        assert!(recovered_engine.get_order("BTC-USDT", "btc-1").unwrap().is_some());
        assert!(recovered_engine.get_order("ETH-USDT", "eth-1").unwrap().is_some());
        assert!(recovered_engine.get_order("SOL-USDT", "sol-1").unwrap().is_some());

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
