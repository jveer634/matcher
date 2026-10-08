use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::matcher::{MatchResult, Matcher, MatcherStats};
use crate::orderbook::{BookLevel, Order, OrderType, Side, Trade, UDecimal};

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
}

impl ShardedEngine {
    pub fn new() -> Self {
        ShardedEngine {
            shards: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new symbol shard
    pub fn register_symbol(
        &self,
        symbol: &str,
        listing_price: Option<UDecimal>,
    ) -> Result<(), String> {
        let mut shards = self.shards.write().map_err(|e| e.to_string())?;
        if shards.contains_key(symbol) {
            return Err(format!("Symbol shard '{}' already registered", symbol));
        }

        let matcher = Matcher::new(symbol.to_string(), listing_price);
        shards.insert(symbol.to_string(), Arc::new(RwLock::new(matcher)));
        Ok(())
    }

    /// Check if a symbol is registered
    pub fn has_symbol(&self, symbol: &str) -> bool {
        self.shards
            .read()
            .map(|s| s.contains_key(symbol))
            .unwrap_or(false)
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
    pub fn cancel_order(
        &self,
        symbol: &str,
        order_id: &str,
        user_id: &str,
    ) -> Result<Order, String> {
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
    pub fn get_depth(
        &self,
        symbol: &str,
        limit: usize,
    ) -> Result<(Vec<BookLevel>, Vec<BookLevel>), String> {
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

        assert_eq!(
            btc_stats.total_volume_traded,
            UDecimal::new(dec!(1.0)).unwrap()
        );
        assert_eq!(
            eth_stats.total_volume_traded,
            UDecimal::new(dec!(5.0)).unwrap()
        );
        assert_eq!(
            sol_stats.total_volume_traded,
            UDecimal::new(dec!(40.0)).unwrap()
        );
    }
}
