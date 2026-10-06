use std::collections::{BTreeMap, HashMap, VecDeque};

use super::{
    id_generator::IdGenerator,
    level::BookLevel,
    order::{Order, OrderType, Side},
    trade::Trade,
    udecimal::UDecimal,
};

#[derive(Debug)]
pub struct OrderBook {
    pair_id: String,
    id_generator: IdGenerator,
    buy_orders: BTreeMap<UDecimal, VecDeque<Order>>,
    sell_orders: BTreeMap<UDecimal, VecDeque<Order>>,
    pub sell_volume: UDecimal,
    pub buy_volume: UDecimal,
    order_index: HashMap<String, Order>,
    last_traded_price: Option<UDecimal>,
}

impl OrderBook {
    pub fn new(pair_id: String, listing_price: Option<UDecimal>) -> OrderBook {
        OrderBook {
            pair_id: pair_id.clone(),
            id_generator: IdGenerator::new(pair_id),
            buy_orders: BTreeMap::new(),
            sell_orders: BTreeMap::new(),
            order_index: HashMap::new(),
            last_traded_price: listing_price.filter(|p| p.is_positive()),
            sell_volume: UDecimal::ZERO,
            buy_volume: UDecimal::ZERO,
        }
    }

    pub fn pair_id(&self) -> &str {
        &self.pair_id
    }

    pub fn last_traded_price(&self) -> Option<UDecimal> {
        self.last_traded_price
    }

    pub fn add_order(
        &mut self,
        id: String,
        user_id: String,
        side: Side,
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
    ) -> Result<Vec<Trade>, String> {
        if self.order_index.contains_key(&id) {
            return Err(format!("Order with id '{}' already exists in book", id));
        }

        let mut order = Order::new(
            id.clone(),
            user_id,
            self.pair_id.clone(),
            side,
            order_type,
            quantity,
            price,
        )?;

        let trades = match order.order_type() {
            OrderType::Market => self.match_market_order(&mut order),
            OrderType::Limit => self.match_limit_order(&mut order),
        };

        if !order.is_filled() && matches!(order.order_type(), OrderType::Limit) {
            self.order_index.insert(order.id().to_string(), order);
        }

        Ok(trades)
    }

    /// Common matching helper: matches an aggressive order against maker orders in a FIFO queue at a specific price level.
    fn match_against_level(
        &mut self,
        opposite_side: Side,
        level_price: UDecimal,
        order: &mut Order,
        trades: &mut Vec<Trade>,
    ) {
        let (orders_map, total_volume) = match opposite_side {
            Side::Buy => (&mut self.buy_orders, &mut self.buy_volume),
            Side::Sell => (&mut self.sell_orders, &mut self.sell_volume),
        };

        let Some(book_orders) = orders_map.get_mut(&level_price) else {
            return;
        };

        while let Some(mut book_order) = book_orders.pop_front() {
            let trade_qty = order.quantity().min(book_order.quantity());
            let trade_price = level_price;

            book_order.fill_order(trade_qty);
            order.fill_order(trade_qty);
            *total_volume -= trade_qty;
            self.last_traded_price = Some(trade_price);

            let trade = Trade::new(
                self.id_generator.generate_id(),
                self.pair_id.clone(),
                book_order.id().to_string(),
                order.id().to_string(),
                trade_price,
                trade_qty,
            );
            trades.push(trade);

            if book_order.is_filled() {
                self.order_index.remove(book_order.id());
            } else {
                self.order_index
                    .insert(book_order.id().to_string(), book_order.clone());
                book_orders.push_front(book_order);
                break;
            }

            if order.is_filled() {
                break;
            }
        }

        if orders_map.get(&level_price).map_or(false, |q| q.is_empty()) {
            orders_map.remove(&level_price);
        }
    }

    /// Price-Time Priority matching for Market orders (matches across best available price levels until filled or liquidity is exhausted).
    fn match_market_order(&mut self, order: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();
        let opposite_side = match order.side() {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        };

        while !order.is_filled() {
            let best_price = match order.side() {
                Side::Buy => self.sell_orders.keys().next().copied(),
                Side::Sell => self.buy_orders.keys().next_back().copied(),
            };

            match best_price {
                Some(level_price) => {
                    self.match_against_level(opposite_side, level_price, order, &mut trades);
                }
                None => break,
            }
        }

        trades
    }

    /// Price-Time Priority matching for Limit orders (matches against best eligible prices, resting any remaining unfilled quantity on the book).
    fn match_limit_order(&mut self, order: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();
        let limit_price = order.price().expect("Limit order must have a price");
        let opposite_side = match order.side() {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        };

        while !order.is_filled() {
            let eligible_best_price = match order.side() {
                Side::Buy => match self.sell_orders.keys().next().copied() {
                    Some(p) if p <= limit_price => Some(p),
                    _ => None,
                },
                Side::Sell => match self.buy_orders.keys().next_back().copied() {
                    Some(p) if p >= limit_price => Some(p),
                    _ => None,
                },
            };

            match eligible_best_price {
                Some(level_price) => {
                    self.match_against_level(opposite_side, level_price, order, &mut trades);
                }
                None => break,
            }
        }

        if !order.is_filled() {
            match order.side() {
                Side::Buy => {
                    self.buy_volume += order.quantity();
                    self.buy_orders
                        .entry(limit_price)
                        .or_default()
                        .push_back(order.clone());
                }
                Side::Sell => {
                    self.sell_volume += order.quantity();
                    self.sell_orders
                        .entry(limit_price)
                        .or_default()
                        .push_back(order.clone());
                }
            }
        }

        trades
    }

    pub fn cancel_order(&mut self, order_id: &str) -> Result<Order, String> {
        let mut order = self
            .order_index
            .remove(order_id)
            .ok_or_else(|| "Order not found in book".to_string())?;

        let price = order.price().ok_or("Cannot cancel order with no price")?;

        let (orders, volume) = match order.side() {
            Side::Buy => (&mut self.buy_orders, &mut self.buy_volume),
            Side::Sell => (&mut self.sell_orders, &mut self.sell_volume),
        };

        if let Some(queue) = orders.get_mut(&price) {
            queue.retain(|o| o.id() != order_id);
            *volume -= order.quantity();
            if queue.is_empty() {
                orders.remove(&price);
            }
        }

        order.cancel()?;
        Ok(order)
    }

    pub fn get_order(&self, order_id: &str) -> Option<&Order> {
        self.order_index.get(order_id)
    }

    pub fn best_bid(&self) -> Option<(UDecimal, UDecimal)> {
        self.buy_orders.iter().next_back().map(|(price, queue)| {
            let qty: UDecimal = queue.iter().map(|o| o.quantity()).sum();
            (*price, qty)
        })
    }

    pub fn best_ask(&self) -> Option<(UDecimal, UDecimal)> {
        self.sell_orders.iter().next().map(|(price, queue)| {
            let qty: UDecimal = queue.iter().map(|o| o.quantity()).sum();
            (*price, qty)
        })
    }

    pub fn get_depth(&self, limit: usize) -> (Vec<BookLevel>, Vec<BookLevel>) {
        let bids: Vec<BookLevel> = self
            .buy_orders
            .iter()
            .rev()
            .take(limit)
            .map(|(price, queue)| BookLevel {
                price: *price,
                quantity: queue.iter().map(|o| o.quantity()).sum(),
                order_count: queue.len(),
            })
            .collect();

        let asks: Vec<BookLevel> = self
            .sell_orders
            .iter()
            .take(limit)
            .map(|(price, queue)| BookLevel {
                price: *price,
                quantity: queue.iter().map(|o| o.quantity()).sum(),
                order_count: queue.len(),
            })
            .collect();

        (bids, asks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::order::OrderStatus;
    use rust_decimal_macros::dec;

    #[test]
    pub fn test_add_and_cancel_limit_order() {
        let mut book = OrderBook::new(
            "ETH-USDT".to_string(),
            Some(UDecimal::new(dec!(1000.0)).unwrap()),
        );

        let trades = book
            .add_order(
                "ord-1".into(),
                "user-1".into(),
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(1050.0)).unwrap()),
                UDecimal::new(dec!(10.0)).unwrap(),
            )
            .expect("add order");
        assert_eq!(trades.len(), 0);
        assert_eq!(book.buy_volume, UDecimal::new(dec!(10.0)).unwrap());
        assert!(book.get_order("ord-1").is_some());
        assert_eq!(book.get_order("ord-1").unwrap().user_id(), "user-1");

        let cancelled = book.cancel_order("ord-1").expect("cancel");
        assert_eq!(cancelled.status(), OrderStatus::Cancelled);
        assert_eq!(book.buy_volume, UDecimal::ZERO);
        assert!(book.get_order("ord-1").is_none());
    }

    #[test]
    pub fn test_duplicate_order_id_rejected() {
        let mut book = OrderBook::new("BTC-USDT".to_string(), None);
        book.add_order(
            "ord-dup".into(),
            "u1".into(),
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(50000.0)).unwrap()),
            UDecimal::new(dec!(1.0)).unwrap(),
        )
        .unwrap();

        let dup = book.add_order(
            "ord-dup".into(),
            "u2".into(),
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(50000.0)).unwrap()),
            UDecimal::new(dec!(1.0)).unwrap(),
        );
        assert!(dup.is_err());
    }

    #[test]
    pub fn test_limit_matching_price_priority() {
        let mut book = OrderBook::new("BTC-USDT".to_string(), None);

        // Place asks: 1 BTC @ 50000, 1 BTC @ 51000
        book.add_order(
            "ask-1".into(),
            "u1".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(50000.0)).unwrap()),
            UDecimal::new(dec!(1.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            "ask-2".into(),
            "u2".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(51000.0)).unwrap()),
            UDecimal::new(dec!(1.0)).unwrap(),
        )
        .unwrap();

        // Place limit buy: 1.5 BTC @ 50500 -> Should fill 1.0 BTC @ 50000, 0.5 rests @ 50500
        let trades = book
            .add_order(
                "buy-1".into(),
                "u3".into(),
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(50500.0)).unwrap()),
                UDecimal::new(dec!(1.5)).unwrap(),
            )
            .unwrap();

        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].quantity, UDecimal::new(dec!(1.0)).unwrap());
        assert_eq!(trades[0].price, UDecimal::new(dec!(50000.0)).unwrap());
        assert_eq!(trades[0].notional(), UDecimal::new(dec!(50000.0)).unwrap());
        assert_eq!(book.buy_volume, UDecimal::new(dec!(0.5)).unwrap());
        assert_eq!(book.sell_volume, UDecimal::new(dec!(1.0)).unwrap());

        let resting = book.get_order("buy-1").unwrap();
        assert_eq!(resting.quantity(), UDecimal::new(dec!(0.5)).unwrap());
    }

    #[test]
    pub fn test_market_order_matching() {
        let mut book = OrderBook::new("SOL-USDT".to_string(), None);

        book.add_order(
            "ask-1".into(),
            "u1".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(100.0)).unwrap()),
            UDecimal::new(dec!(5.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            "ask-2".into(),
            "u2".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(101.0)).unwrap()),
            UDecimal::new(dec!(5.0)).unwrap(),
        )
        .unwrap();

        let trades = book
            .add_order(
                "market-buy".into(),
                "u3".into(),
                Side::Buy,
                OrderType::Market,
                None,
                UDecimal::new(dec!(7.0)).unwrap(),
            )
            .unwrap();
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].quantity, UDecimal::new(dec!(5.0)).unwrap());
        assert_eq!(trades[0].price, UDecimal::new(dec!(100.0)).unwrap());
        assert_eq!(trades[0].notional(), UDecimal::new(dec!(500.0)).unwrap());
        assert_eq!(trades[1].quantity, UDecimal::new(dec!(2.0)).unwrap());
        assert_eq!(trades[1].price, UDecimal::new(dec!(101.0)).unwrap());
        assert_eq!(trades[1].notional(), UDecimal::new(dec!(202.0)).unwrap());

        assert_eq!(book.sell_volume, UDecimal::new(dec!(3.0)).unwrap());
    }

    #[test]
    pub fn test_depth_snapshot() {
        let mut book = OrderBook::new("AVAX-USDT".to_string(), None);
        book.add_order(
            "b1".into(),
            "u1".into(),
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(30.0)).unwrap()),
            UDecimal::new(dec!(10.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            "b2".into(),
            "u2".into(),
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(29.0)).unwrap()),
            UDecimal::new(dec!(20.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            "a1".into(),
            "u3".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(31.0)).unwrap()),
            UDecimal::new(dec!(15.0)).unwrap(),
        )
        .unwrap();

        let (bids, asks) = book.get_depth(5);
        assert_eq!(bids.len(), 2);
        assert_eq!(bids[0].price, UDecimal::new(dec!(30.0)).unwrap());
        assert_eq!(bids[0].quantity, UDecimal::new(dec!(10.0)).unwrap());
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].price, UDecimal::new(dec!(31.0)).unwrap());
        assert_eq!(asks[0].quantity, UDecimal::new(dec!(15.0)).unwrap());
    }

    #[test]
    pub fn test_multi_level_price_priority_limit_buy() {
        let mut book = OrderBook::new("BTC-USDT".to_string(), None);

        // Place resting asks across multiple levels
        book.add_order(
            "a1".into(),
            "u1".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(100.0)).unwrap()),
            UDecimal::new(dec!(2.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            "a2".into(),
            "u2".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(101.0)).unwrap()),
            UDecimal::new(dec!(3.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            "a3".into(),
            "u3".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(102.0)).unwrap()),
            UDecimal::new(dec!(5.0)).unwrap(),
        )
        .unwrap();

        // Limit buy with price 102 for 7 units matches best prices first: 100 -> 101 -> 102
        let trades = book
            .add_order(
                "b1".into(),
                "u4".into(),
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(102.0)).unwrap()),
                UDecimal::new(dec!(7.0)).unwrap(),
            )
            .unwrap();

        assert_eq!(trades.len(), 3);
        assert_eq!(trades[0].price, UDecimal::new(dec!(100.0)).unwrap());
        assert_eq!(trades[0].quantity, UDecimal::new(dec!(2.0)).unwrap());
        assert_eq!(trades[1].price, UDecimal::new(dec!(101.0)).unwrap());
        assert_eq!(trades[1].quantity, UDecimal::new(dec!(3.0)).unwrap());
        assert_eq!(trades[2].price, UDecimal::new(dec!(102.0)).unwrap());
        assert_eq!(trades[2].quantity, UDecimal::new(dec!(2.0)).unwrap());

        // Remaining ask volume on book at 102 should be 3.0
        assert_eq!(book.sell_volume, UDecimal::new(dec!(3.0)).unwrap());
        let (bids, asks) = book.get_depth(5);
        assert_eq!(bids.len(), 0);
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].price, UDecimal::new(dec!(102.0)).unwrap());
        assert_eq!(asks[0].quantity, UDecimal::new(dec!(3.0)).unwrap());
    }

    #[test]
    pub fn test_multi_level_price_priority_limit_sell() {
        let mut book = OrderBook::new("BTC-USDT".to_string(), None);

        // Place resting bids across multiple levels
        book.add_order(
            "b1".into(),
            "u1".into(),
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(200.0)).unwrap()),
            UDecimal::new(dec!(2.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            "b2".into(),
            "u2".into(),
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(199.0)).unwrap()),
            UDecimal::new(dec!(3.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            "b3".into(),
            "u3".into(),
            Side::Buy,
            OrderType::Limit,
            Some(UDecimal::new(dec!(198.0)).unwrap()),
            UDecimal::new(dec!(5.0)).unwrap(),
        )
        .unwrap();

        // Limit sell with price 198 for 7 units matches best prices first: 200 -> 199 -> 198
        let trades = book
            .add_order(
                "s1".into(),
                "u4".into(),
                Side::Sell,
                OrderType::Limit,
                Some(UDecimal::new(dec!(198.0)).unwrap()),
                UDecimal::new(dec!(7.0)).unwrap(),
            )
            .unwrap();

        assert_eq!(trades.len(), 3);
        assert_eq!(trades[0].price, UDecimal::new(dec!(200.0)).unwrap());
        assert_eq!(trades[0].quantity, UDecimal::new(dec!(2.0)).unwrap());
        assert_eq!(trades[1].price, UDecimal::new(dec!(199.0)).unwrap());
        assert_eq!(trades[1].quantity, UDecimal::new(dec!(3.0)).unwrap());
        assert_eq!(trades[2].price, UDecimal::new(dec!(198.0)).unwrap());
        assert_eq!(trades[2].quantity, UDecimal::new(dec!(2.0)).unwrap());

        // Remaining bid volume on book at 198 should be 3.0
        assert_eq!(book.buy_volume, UDecimal::new(dec!(3.0)).unwrap());
        let (bids, asks) = book.get_depth(5);
        assert_eq!(asks.len(), 0);
        assert_eq!(bids.len(), 1);
        assert_eq!(bids[0].price, UDecimal::new(dec!(198.0)).unwrap());
        assert_eq!(bids[0].quantity, UDecimal::new(dec!(3.0)).unwrap());
    }

    #[test]
    pub fn test_time_priority_fifo_same_price() {
        let mut book = OrderBook::new("ETH-USDT".to_string(), None);

        // Place two orders at the exact same price
        book.add_order(
            "s1".into(),
            "u1".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(100.0)).unwrap()),
            UDecimal::new(dec!(2.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            "s2".into(),
            "u2".into(),
            Side::Sell,
            OrderType::Limit,
            Some(UDecimal::new(dec!(100.0)).unwrap()),
            UDecimal::new(dec!(3.0)).unwrap(),
        )
        .unwrap();

        // Limit buy for 3.0 units should fill the first order completely (2.0) and second order partially (1.0)
        let trades = book
            .add_order(
                "b1".into(),
                "u3".into(),
                Side::Buy,
                OrderType::Limit,
                Some(UDecimal::new(dec!(100.0)).unwrap()),
                UDecimal::new(dec!(3.0)).unwrap(),
            )
            .unwrap();

        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].maker_order_id, "s1");
        assert_eq!(trades[0].quantity, UDecimal::new(dec!(2.0)).unwrap());
        assert_eq!(trades[1].maker_order_id, "s2");
        assert_eq!(trades[1].quantity, UDecimal::new(dec!(1.0)).unwrap());

        // First order must be gone, second order has 2.0 remaining
        assert!(book.get_order("s1").is_none());
        let second = book.get_order("s2").unwrap();
        assert_eq!(second.quantity(), UDecimal::new(dec!(2.0)).unwrap());
    }
}
