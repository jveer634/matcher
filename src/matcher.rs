use std::path::{Path, PathBuf};

use crate::orderbook::{BookLevel, Order, OrderBook, OrderType, Side, Trade, UDecimal};
use crate::wal::{
    decode_decimal, decode_fixed_str, decode_order_type, decode_side, BookSnapshot, SnapshotReader, SyncPolicy,
    WalPayload, WalReader, WalWriter,
};

#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    pub order_id: String,
    pub trades: Vec<Trade>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatcherStats {
    pub symbol: String,
    pub last_traded_price: Option<UDecimal>,
    pub total_trades_count: usize,
    pub total_volume_traded: UDecimal,
    pub active_buy_volume: UDecimal,
    pub active_sell_volume: UDecimal,
    pub best_bid: Option<(UDecimal, UDecimal)>,
    pub best_ask: Option<(UDecimal, UDecimal)>,
}

/// A dedicated matching engine shard for a single symbol / trading pair.
/// Each instance handles orders, state, and trades strictly for one symbol.
#[derive(Debug)]
pub struct Matcher {
    symbol: String,
    orderbook: OrderBook,
    trade_history: Vec<Trade>,
    total_volume_traded: UDecimal,
    total_trades_count: usize,
    wal_writer: Option<WalWriter>,
}

impl Matcher {
    /// Creates a purely in-memory matcher shard without disk WAL persistence.
    #[must_use]
    pub fn new(symbol: String, listing_price: Option<UDecimal>) -> Self {
        Matcher {
            symbol: symbol.clone(),
            orderbook: OrderBook::new(symbol, listing_price),
            trade_history: Vec::new(),
            total_volume_traded: UDecimal::ZERO,
            total_trades_count: 0,
            wal_writer: None,
        }
    }

    /// Creates a persistent matcher shard backed by a Write-Ahead Log.
    ///
    /// If an existing snapshot exists, it is loaded first to restore the base order book state instantly,
    /// and then only delta WAL records after the snapshot checkpoint are replayed.
    pub fn with_wal(
        symbol: String,
        listing_price: Option<UDecimal>,
        wal_path: impl AsRef<Path>,
        sync_policy: SyncPolicy,
    ) -> Result<Self, String> {
        let wal_path = wal_path.as_ref();
        let mut orderbook = OrderBook::new(symbol.clone(), listing_price);
        let mut trade_history = Vec::new();
        let mut total_volume_traded = UDecimal::ZERO;
        let mut total_trades_count = 0;
        let mut min_wal_seq = 0;

        // 1. Check for latest valid snapshot in parent directory
        if let Some(parent) = wal_path.parent() {
            let mut latest_snap: Option<(u64, PathBuf)> = None;
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                    if path.is_file()
                        && name.starts_with(&format!("{symbol}_snapshot_"))
                        && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("snap"))
                    {
                        if let Some(seq) = name.split(['_', '.']).find_map(|s| s.parse::<u64>().ok()) {
                            if latest_snap.as_ref().map_or(true, |(s, _)| seq > *s) {
                                latest_snap = Some((seq, path));
                            }
                        }
                    }
                }
            }

            if let Some((snap_seq, snap_path)) = latest_snap {
                if let Ok(snapshot) = SnapshotReader::read_snapshot(&snap_path) {
                    orderbook.set_last_traded_price(snapshot.last_traded_price);
                    total_volume_traded = snapshot.total_volume_traded;
                    total_trades_count = usize::try_from(snapshot.total_trades_count).unwrap_or(0);
                    orderbook.restore_open_orders(snapshot.open_orders);
                    min_wal_seq = snap_seq;
                }
            }
        }

        // 2. Replay historical WAL records strictly after the snapshot checkpoint
        if wal_path.exists() {
            let mut reader = WalReader::open(wal_path)?;
            let (records, _) = reader.read_all();

            for record in records {
                if record.sequence <= min_wal_seq {
                    continue;
                }
                match record.payload {
                    WalPayload::SubmitOrder(sbe) => {
                        let id = decode_fixed_str(&sbe.order_id)?;
                        let user_id = decode_fixed_str(&sbe.user_id)?;
                        let side = decode_side(sbe.side)?;
                        let order_type = decode_order_type(sbe.order_type)?;
                        let price = decode_decimal(sbe.price_mantissa, sbe.price_scale)?;
                        let quantity = decode_decimal(sbe.qty_mantissa, sbe.qty_scale)?.unwrap_or(UDecimal::ZERO);

                        if let Ok(trades) = orderbook.add_order(id, user_id, side, order_type, price, quantity) {
                            total_trades_count += trades.len();
                            for trade in trades {
                                total_volume_traded += trade.quantity;
                                trade_history.push(trade);
                            }
                        }
                    }
                    WalPayload::CancelOrder(sbe) => {
                        let order_id = decode_fixed_str(&sbe.order_id)?;
                        let user_id = decode_fixed_str(&sbe.user_id)?;
                        let _ = orderbook.cancel_order(&order_id, &user_id);
                    }
                    WalPayload::SnapshotMarker(_) => {}
                }
            }
        }

        // 3. Open / resume append writer
        let wal_writer = WalWriter::open_or_create(wal_path, sync_policy)?;

        Ok(Matcher {
            symbol,
            orderbook,
            trade_history,
            total_volume_traded,
            total_trades_count,
            wal_writer: Some(wal_writer),
        })
    }

    /// Captures the current in-memory state of this shard as a [`BookSnapshot`].
    /// Holds the shard lock only in memory for a few microseconds without performing disk I/O.
    #[must_use]
    pub fn capture_snapshot(&self) -> BookSnapshot {
        let sequence = self
            .wal_writer
            .as_ref()
            .map_or(0, |w| w.next_sequence().saturating_sub(1));

        let timestamp_ns = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        )
        .unwrap_or(0);

        BookSnapshot {
            symbol: self.symbol.clone(),
            sequence,
            timestamp_ns,
            last_traded_price: self.orderbook.last_traded_price(),
            total_volume_traded: self.total_volume_traded,
            total_trades_count: self.total_trades_count as u64,
            open_orders: self.orderbook.open_orders(),
        }
    }

    /// Emits a `SnapshotMarker` to the WAL stream recording the checkpoint sequence.
    pub fn emit_snapshot_marker(&mut self, snapshot_seq: u64, open_orders_count: u64) -> Result<Option<u64>, String> {
        if let Some(writer) = &mut self.wal_writer {
            let timestamp_ns = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
            )
            .unwrap_or(0);

            let seq = writer.append_snapshot_marker(snapshot_seq, open_orders_count, timestamp_ns)?;
            Ok(Some(seq))
        } else {
            Ok(None)
        }
    }

    #[must_use]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Submits an order on the hot path.
    ///
    /// When persistence is enabled, the order is appended to the Write-Ahead Log **before**
    /// the in-memory matching algorithm executes.
    pub fn submit_order(
        &mut self,
        id: String,
        user_id: String,
        side: Side,
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
    ) -> Result<MatchResult, String> {
        let timestamp_ns = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        )
        .unwrap_or(0);

        // 1. Write-Ahead Log append
        if let Some(writer) = &mut self.wal_writer {
            writer.append_submit_order(&id, &user_id, side, order_type, price, quantity, timestamp_ns)?;
        }

        // 2. In-Memory matching execution
        let trades = self
            .orderbook
            .add_order(id.clone(), user_id, side, order_type, price, quantity)?;

        self.total_trades_count += trades.len();
        for trade in &trades {
            self.total_volume_traded += trade.quantity;
            self.trade_history.push(trade.clone());
        }

        Ok(MatchResult { order_id: id, trades })
    }

    /// Cancels an order on the hot path.
    ///
    /// When persistence is enabled, the cancellation is written to the Write-Ahead Log **before**
    /// modifying the order book state.
    pub fn cancel_order(&mut self, order_id: &str, user_id: &str) -> Result<Order, String> {
        let timestamp_ns = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        )
        .unwrap_or(0);

        // 1. Write-Ahead Log append
        if let Some(writer) = &mut self.wal_writer {
            writer.append_cancel_order(order_id, user_id, timestamp_ns)?;
        }

        // 2. In-Memory cancellation
        self.orderbook.cancel_order(order_id, user_id)
    }

    pub fn get_order(&self, order_id: &str) -> Option<&Order> {
        self.orderbook.get_order(order_id)
    }

    pub fn get_depth(&self, limit: usize) -> (Vec<BookLevel>, Vec<BookLevel>) {
        self.orderbook.get_depth(limit)
    }

    #[must_use]
    pub fn trade_history(&self) -> &[Trade] {
        &self.trade_history
    }

    #[must_use]
    pub fn last_traded_price(&self) -> Option<UDecimal> {
        self.orderbook.last_traded_price()
    }

    #[must_use]
    pub fn wal_writer(&self) -> Option<&WalWriter> {
        self.wal_writer.as_ref()
    }

    pub fn wal_writer_mut(&mut self) -> Option<&mut WalWriter> {
        self.wal_writer.as_mut()
    }

    /// Flushes the internal WAL writer buffer if configured.
    pub fn flush_wal(&mut self) -> Result<(), String> {
        if let Some(writer) = &mut self.wal_writer {
            writer.flush()?;
        }
        Ok(())
    }

    #[must_use]
    pub fn stats(&self) -> MatcherStats {
        MatcherStats {
            symbol: self.symbol.clone(),
            last_traded_price: self.orderbook.last_traded_price(),
            total_trades_count: self.total_trades_count,
            total_volume_traded: self.total_volume_traded,
            active_buy_volume: self.orderbook.buy_volume,
            active_sell_volume: self.orderbook.sell_volume,
            best_bid: self.orderbook.best_bid(),
            best_ask: self.orderbook.best_ask(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    pub fn test_single_symbol_matcher() {
        let mut matcher = Matcher::new("BTC-USDT".to_string(), Some(UDecimal::new(dec!(65000.0)).unwrap()));
        assert_eq!(matcher.symbol(), "BTC-USDT");

        // Submit limit sell
        let sell_res = matcher
            .submit_order(
                "ord-sell-1".into(),
                "user-maker-1".into(),
                Side::Sell,
                OrderType::Limit,
                Some(UDecimal::new(dec!(65000.0)).unwrap()),
                UDecimal::new(dec!(2.0)).unwrap(),
            )
            .expect("place limit sell");
        assert_eq!(sell_res.trades.len(), 0);
        assert_eq!(sell_res.order_id, "ord-sell-1");

        // Submit matching limit buy
        let buy_res = matcher
            .submit_order(
                "ord-buy-1".into(),
                "user-taker-1".into(),
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(65000.0)).unwrap()),
                UDecimal::new(dec!(1.5)).unwrap(),
            )
            .expect("place limit buy");
        assert_eq!(buy_res.trades.len(), 1);
        assert_eq!(buy_res.order_id, "ord-buy-1");
        assert_eq!(buy_res.trades[0].quantity, UDecimal::new(dec!(1.5)).unwrap());
        assert_eq!(buy_res.trades[0].maker_order_id, "ord-sell-1");
        assert_eq!(buy_res.trades[0].taker_order_id, "ord-buy-1");
        assert_eq!(buy_res.trades[0].symbol, "BTC-USDT");

        let stats = matcher.stats();
        assert_eq!(stats.total_trades_count, 1);
        assert_eq!(stats.total_volume_traded, UDecimal::new(dec!(1.5)).unwrap());
        assert_eq!(stats.active_sell_volume, UDecimal::new(dec!(0.5)).unwrap());
        assert_eq!(stats.active_buy_volume, UDecimal::ZERO);
    }

    #[test]
    pub fn test_matcher_wal_write_and_recovery() {
        let mut temp_wal = std::env::temp_dir();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        temp_wal.push(format!("matcher_hot_path_{unique}.wal"));

        // 1. Create matcher with WAL enabled and submit orders
        {
            let mut matcher = Matcher::with_wal(
                "ETH-USDT".to_string(),
                Some(UDecimal::new(dec!(3500.0)).unwrap()),
                &temp_wal,
                SyncPolicy::None,
            )
            .expect("create matcher with WAL");

            matcher
                .submit_order(
                    "eth-sell-1".into(),
                    "trader-alice".into(),
                    Side::Sell,
                    OrderType::Limit,
                    Some(UDecimal::new(dec!(3500.0)).unwrap()),
                    UDecimal::new(dec!(10.0)).unwrap(),
                )
                .expect("submit limit sell");

            matcher
                .submit_order(
                    "eth-buy-1".into(),
                    "trader-bob".into(),
                    Side::Buy,
                    OrderType::Limit,
                    Some(UDecimal::new(dec!(3500.0)).unwrap()),
                    UDecimal::new(dec!(4.0)).unwrap(),
                )
                .expect("submit matching buy");

            // Cancel resting order
            matcher
                .cancel_order("eth-sell-1", "trader-alice")
                .expect("cancel resting sell");

            matcher.flush_wal().expect("flush WAL");
        }

        // 2. Recover into a brand new Matcher instance from WAL
        {
            let recovered = Matcher::with_wal(
                "ETH-USDT".to_string(),
                Some(UDecimal::new(dec!(3500.0)).unwrap()),
                &temp_wal,
                SyncPolicy::None,
            )
            .expect("recover matcher from WAL");

            assert_eq!(recovered.symbol(), "ETH-USDT");
            let stats = recovered.stats();
            assert_eq!(stats.total_trades_count, 1);
            assert_eq!(stats.total_volume_traded, UDecimal::new(dec!(4.0)).unwrap());
            // After cancellation of remaining 6.0 ETH, active sell volume must be 0
            assert_eq!(stats.active_sell_volume, UDecimal::ZERO);
            assert_eq!(stats.active_buy_volume, UDecimal::ZERO);
        }

        let _ = std::fs::remove_file(&temp_wal);
    }
}
