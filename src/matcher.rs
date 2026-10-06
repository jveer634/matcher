use crate::orderbook::{BookLevel, Order, OrderBook, OrderType, Side, Trade, UDecimal};

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
}

impl Matcher {
    pub fn new(symbol: String, listing_price: Option<UDecimal>) -> Self {
        Matcher {
            symbol: symbol.clone(),
            orderbook: OrderBook::new(symbol, listing_price),
            trade_history: Vec::new(),
            total_volume_traded: UDecimal::ZERO,
        }
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn submit_order(
        &mut self,
        side: Side,
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
    ) -> Result<MatchResult, String> {
        let (order_id, trades) = self.orderbook.add_order(side, order_type, price, quantity)?;

        for trade in &trades {
            self.total_volume_traded += trade.quantity;
            self.trade_history.push(trade.clone());
        }

        Ok(MatchResult { order_id, trades })
    }

    pub fn cancel_order(&mut self, order_id: &str) -> Result<Order, String> {
        self.orderbook.cancel_order(order_id)
    }

    pub fn get_order(&self, order_id: &str) -> Option<&Order> {
        self.orderbook.get_order(order_id)
    }

    pub fn get_depth(&self, limit: usize) -> (Vec<BookLevel>, Vec<BookLevel>) {
        self.orderbook.get_depth(limit)
    }

    pub fn trade_history(&self) -> &[Trade] {
        &self.trade_history
    }

    pub fn last_traded_price(&self) -> Option<UDecimal> {
        self.orderbook.last_traded_price()
    }

    pub fn stats(&self) -> MatcherStats {
        MatcherStats {
            symbol: self.symbol.clone(),
            last_traded_price: self.orderbook.last_traded_price(),
            total_trades_count: self.trade_history.len(),
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
        let mut matcher = Matcher::new(
            "BTC-USDT".to_string(),
            Some(UDecimal::new(dec!(65000.0)).unwrap()),
        );
        assert_eq!(matcher.symbol(), "BTC-USDT");

        // Submit limit sell
        let sell_res = matcher
            .submit_order(
                Side::Sell,
                OrderType::Limit,
                Some(UDecimal::new(dec!(65000.0)).unwrap()),
                UDecimal::new(dec!(2.0)).unwrap(),
            )
            .expect("place limit sell");
        assert_eq!(sell_res.trades.len(), 0);

        // Submit matching limit buy
        let buy_res = matcher
            .submit_order(
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(65000.0)).unwrap()),
                UDecimal::new(dec!(1.5)).unwrap(),
            )
            .expect("place limit buy");
        assert_eq!(buy_res.trades.len(), 1);
        assert_eq!(
            buy_res.trades[0].quantity,
            UDecimal::new(dec!(1.5)).unwrap()
        );
        assert_eq!(buy_res.trades[0].symbol, "BTC-USDT");

        let stats = matcher.stats();
        assert_eq!(stats.total_trades_count, 1);
        assert_eq!(
            stats.total_volume_traded,
            UDecimal::new(dec!(1.5)).unwrap()
        );
        assert_eq!(
            stats.active_sell_volume,
            UDecimal::new(dec!(0.5)).unwrap()
        );
        assert_eq!(stats.active_buy_volume, UDecimal::ZERO);
    }
}
