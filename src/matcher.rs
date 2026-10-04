use rust_decimal::Decimal;

use crate::orderbook::{
    order::{Order, OrderType},
    orderbook::{BookLevel, OrderBook},
    trade::Trade,
};

#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    pub order_id: String,
    pub trades: Vec<Trade>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatcherStats {
    pub symbol: String,
    pub last_traded_price: Option<Decimal>,
    pub total_trades_count: usize,
    pub total_volume_traded: f64,
    pub active_buy_volume: f64,
    pub active_sell_volume: f64,
    pub best_bid: Option<(Decimal, f64)>,
    pub best_ask: Option<(Decimal, f64)>,
}

/// A dedicated matching engine shard for a single symbol / trading pair.
/// Each instance handles orders, state, and trades strictly for one symbol.
#[derive(Debug)]
pub struct Matcher {
    symbol: String,
    orderbook: OrderBook,
    trade_history: Vec<Trade>,
    total_volume_traded: f64,
}

impl Matcher {
    pub fn new(symbol: String, listing_price: Option<f64>) -> Self {
        Matcher {
            symbol: symbol.clone(),
            orderbook: OrderBook::new(symbol, listing_price),
            trade_history: Vec::new(),
            total_volume_traded: 0.0,
        }
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn submit_order(
        &mut self,
        order_type: OrderType,
        price: Option<f64>,
        quantity: f64,
    ) -> Result<MatchResult, String> {
        let (order_id, trades) = self.orderbook.add_order(order_type, price, quantity)?;

        for trade in &trades {
            self.total_volume_traded += trade.quantity;
            self.trade_history.push(trade.clone());
        }

        Ok(MatchResult { order_id, trades })
    }

    pub fn cancel_order(&mut self, order_id: &str) -> Result<Order, String> {
        self.orderbook.cancel_order(order_id)
    }

    pub fn update_order(
        &mut self,
        order_id: &str,
        quantity: Option<f64>,
        order_type: Option<OrderType>,
        price: Option<f64>,
    ) -> Result<MatchResult, String> {
        let (order, trades) = self
            .orderbook
            .update_order(order_id, quantity, order_type, price)?;

        for trade in &trades {
            self.total_volume_traded += trade.quantity;
            self.trade_history.push(trade.clone());
        }

        Ok(MatchResult {
            order_id: order.id().clone(),
            trades,
        })
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

    pub fn last_traded_price(&self) -> Option<Decimal> {
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

    #[test]
    pub fn test_single_symbol_matcher() {
        let mut matcher = Matcher::new("BTC-USDT".to_string(), Some(65000.0));
        assert_eq!(matcher.symbol(), "BTC-USDT");

        // Submit limit sell
        let sell_res = matcher
            .submit_order(OrderType::LimitSell, Some(65000.0), 2.0)
            .expect("place limit sell");
        assert_eq!(sell_res.trades.len(), 0);

        // Submit matching limit buy
        let buy_res = matcher
            .submit_order(OrderType::LimitBuy, Some(65000.0), 1.5)
            .expect("place limit buy");
        assert_eq!(buy_res.trades.len(), 1);
        assert_eq!(buy_res.trades[0].quantity, 1.5);
        assert_eq!(buy_res.trades[0].symbol, "BTC-USDT");

        let stats = matcher.stats();
        assert_eq!(stats.total_trades_count, 1);
        assert_eq!(stats.total_volume_traded, 1.5);
        assert_eq!(stats.active_sell_volume, 0.5);
        assert_eq!(stats.active_buy_volume, 0.0);
    }
}
