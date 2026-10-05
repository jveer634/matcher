use std::collections::{BTreeMap, HashMap, VecDeque};

use super::{
    id_generator::IdGenerator,
    order::{Order, OrderType},
    trade::Trade,
    udecimal::UDecimal,
};

#[derive(Debug, Clone, PartialEq)]
pub struct BookLevel {
    pub price: UDecimal,
    pub quantity: UDecimal,
    pub order_count: usize,
}

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
        order_type: OrderType,
        price: Option<UDecimal>,
        quantity: UDecimal,
    ) -> Result<(String, Vec<Trade>), String> {
        let id = self.id_generator.generate_order_id();
        let mut order = Order::new(
            id.clone(),
            self.pair_id.clone(),
            quantity,
            order_type,
            price,
        )?;

        let trades = match *order.order_type() {
            OrderType::Buy => self.match_market_buy(&mut order),
            OrderType::Sell => self.match_market_sell(&mut order),
            OrderType::LimitBuy => self.match_limit_buy(&mut order),
            OrderType::LimitSell => self.match_limit_sell(&mut order),
        };

        if !order.is_filled()
            && matches!(
                *order.order_type(),
                OrderType::LimitBuy | OrderType::LimitSell
            )
        {
            self.order_index.insert(order.id().clone(), order);
        }

        Ok((id, trades))
    }

    fn match_market_buy(&mut self, order: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();

        while !order.is_filled() {
            let lowest_ask_price = match self.sell_orders.keys().next().cloned() {
                Some(p) => p,
                None => break,
            };

            let book_orders = self.sell_orders.get_mut(&lowest_ask_price).unwrap();
            while let Some(mut book_order) = book_orders.pop_front() {
                let trade_qty = order.quantity().min(book_order.quantity());
                let trade_price = lowest_ask_price;

                book_order.fill_order(trade_qty);
                order.fill_order(trade_qty);
                self.sell_volume -= trade_qty;
                self.last_traded_price = Some(trade_price);

                let trade = Trade::new(
                    self.id_generator.generate_trade_id(),
                    self.pair_id.clone(),
                    book_order.id().clone(),
                    order.id().clone(),
                    trade_price,
                    trade_qty,
                );
                trades.push(trade);

                if book_order.is_filled() {
                    self.order_index.remove(book_order.id());
                } else {
                    self.order_index
                        .insert(book_order.id().clone(), book_order.clone());
                    book_orders.push_front(book_order);
                    break;
                }

                if order.is_filled() {
                    break;
                }
            }

            if self
                .sell_orders
                .get(&lowest_ask_price)
                .map_or(false, |q| q.is_empty())
            {
                self.sell_orders.remove(&lowest_ask_price);
            }
        }

        trades
    }

    fn match_market_sell(&mut self, order: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();

        while !order.is_filled() {
            let highest_bid_price = match self.buy_orders.keys().next_back().cloned() {
                Some(p) => p,
                None => break,
            };

            let book_orders = self.buy_orders.get_mut(&highest_bid_price).unwrap();
            while let Some(mut book_order) = book_orders.pop_front() {
                let trade_qty = order.quantity().min(book_order.quantity());
                let trade_price = highest_bid_price;

                book_order.fill_order(trade_qty);
                order.fill_order(trade_qty);
                self.buy_volume -= trade_qty;
                self.last_traded_price = Some(trade_price);

                let trade = Trade::new(
                    self.id_generator.generate_trade_id(),
                    self.pair_id.clone(),
                    book_order.id().clone(),
                    order.id().clone(),
                    trade_price,
                    trade_qty,
                );
                trades.push(trade);

                if book_order.is_filled() {
                    self.order_index.remove(book_order.id());
                } else {
                    self.order_index
                        .insert(book_order.id().clone(), book_order.clone());
                    book_orders.push_front(book_order);
                    break;
                }

                if order.is_filled() {
                    break;
                }
            }

            if self
                .buy_orders
                .get(&highest_bid_price)
                .map_or(false, |q| q.is_empty())
            {
                self.buy_orders.remove(&highest_bid_price);
            }
        }

        trades
    }

    fn match_limit_buy(&mut self, order: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();
        let limit_price = order.price().unwrap();

        while !order.is_filled() {
            let lowest_ask_price = match self.sell_orders.keys().next().cloned() {
                Some(p) if p <= limit_price => p,
                _ => break,
            };

            let book_orders = self.sell_orders.get_mut(&lowest_ask_price).unwrap();
            while let Some(mut book_order) = book_orders.pop_front() {
                let trade_qty = order.quantity().min(book_order.quantity());
                let trade_price = lowest_ask_price;

                book_order.fill_order(trade_qty);
                order.fill_order(trade_qty);
                self.sell_volume -= trade_qty;
                self.last_traded_price = Some(trade_price);

                let trade = Trade::new(
                    self.id_generator.generate_trade_id(),
                    self.pair_id.clone(),
                    book_order.id().clone(),
                    order.id().clone(),
                    trade_price,
                    trade_qty,
                );
                trades.push(trade);

                if book_order.is_filled() {
                    self.order_index.remove(book_order.id());
                } else {
                    self.order_index
                        .insert(book_order.id().clone(), book_order.clone());
                    book_orders.push_front(book_order);
                    break;
                }

                if order.is_filled() {
                    break;
                }
            }

            if self
                .sell_orders
                .get(&lowest_ask_price)
                .map_or(false, |q| q.is_empty())
            {
                self.sell_orders.remove(&lowest_ask_price);
            }
        }

        if !order.is_filled() {
            self.buy_volume += order.quantity();
            self.buy_orders
                .entry(limit_price)
                .or_insert_with(VecDeque::new)
                .push_back(order.clone());
        }

        trades
    }

    fn match_limit_sell(&mut self, order: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();
        let limit_price = order.price().unwrap();

        while !order.is_filled() {
            let highest_bid_price = match self.buy_orders.keys().next_back().cloned() {
                Some(p) if p >= limit_price => p,
                _ => break,
            };

            let book_orders = self.buy_orders.get_mut(&highest_bid_price).unwrap();
            while let Some(mut book_order) = book_orders.pop_front() {
                let trade_qty = order.quantity().min(book_order.quantity());
                let trade_price = highest_bid_price;

                book_order.fill_order(trade_qty);
                order.fill_order(trade_qty);
                self.buy_volume -= trade_qty;
                self.last_traded_price = Some(trade_price);

                let trade = Trade::new(
                    self.id_generator.generate_trade_id(),
                    self.pair_id.clone(),
                    book_order.id().clone(),
                    order.id().clone(),
                    trade_price,
                    trade_qty,
                );
                trades.push(trade);

                if book_order.is_filled() {
                    self.order_index.remove(book_order.id());
                } else {
                    self.order_index
                        .insert(book_order.id().clone(), book_order.clone());
                    book_orders.push_front(book_order);
                    break;
                }

                if order.is_filled() {
                    break;
                }
            }

            if self
                .buy_orders
                .get(&highest_bid_price)
                .map_or(false, |q| q.is_empty())
            {
                self.buy_orders.remove(&highest_bid_price);
            }
        }

        if !order.is_filled() {
            self.sell_volume += order.quantity();
            self.sell_orders
                .entry(limit_price)
                .or_insert_with(VecDeque::new)
                .push_back(order.clone());
        }

        trades
    }

    pub fn cancel_order(&mut self, order_id: &str) -> Result<Order, String> {
        let mut order = self
            .order_index
            .remove(order_id)
            .ok_or_else(|| "Order not found in book".to_string())?;

        let price = order.price().ok_or("Cannot cancel order with no price")?;

        let (orders, volume) = match *order.order_type() {
            OrderType::LimitBuy | OrderType::Buy => (&mut self.buy_orders, &mut self.buy_volume),
            OrderType::LimitSell | OrderType::Sell => {
                (&mut self.sell_orders, &mut self.sell_volume)
            }
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

    pub fn update_order(
        &mut self,
        order_id: &str,
        quantity: Option<UDecimal>,
        order_type: Option<OrderType>,
        price: Option<UDecimal>,
    ) -> Result<(Order, Vec<Trade>), String> {
        // Validate against current order state before making any modifications
        let current_order = self
            .get_order(order_id)
            .ok_or_else(|| "Order not found in book".to_string())?;

        let new_type = order_type.unwrap_or(*current_order.order_type());
        let new_qty = quantity.unwrap_or(current_order.quantity());
        let new_price = match new_type {
            OrderType::LimitBuy | OrderType::LimitSell => price.or(*current_order.price()),
            OrderType::Buy | OrderType::Sell => None,
        };

        // Validate and create updated order before removing the existing one
        let mut updated_order = Order::new(
            order_id.to_string(),
            self.pair_id.clone(),
            new_qty,
            new_type,
            new_price,
        )?;

        // Safely cancel and remove the old order
        self.cancel_order(order_id)?;

        let trades = match *updated_order.order_type() {
            OrderType::Buy => self.match_market_buy(&mut updated_order),
            OrderType::Sell => self.match_market_sell(&mut updated_order),
            OrderType::LimitBuy => self.match_limit_buy(&mut updated_order),
            OrderType::LimitSell => self.match_limit_sell(&mut updated_order),
        };

        if !updated_order.is_filled()
            && matches!(
                *updated_order.order_type(),
                OrderType::LimitBuy | OrderType::LimitSell
            )
        {
            self.order_index
                .insert(updated_order.id().clone(), updated_order.clone());
        }

        Ok((updated_order, trades))
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

        let (order_id, trades) = book
            .add_order(
                OrderType::LimitBuy,
                Some(UDecimal::new(dec!(1050.0)).unwrap()),
                UDecimal::new(dec!(10.0)).unwrap(),
            )
            .expect("add order");
        assert_eq!(trades.len(), 0);
        assert_eq!(book.buy_volume, UDecimal::new(dec!(10.0)).unwrap());
        assert!(book.get_order(&order_id).is_some());

        let cancelled = book.cancel_order(&order_id).expect("cancel");
        assert_eq!(*cancelled.status(), OrderStatus::Cancelled);
        assert_eq!(book.buy_volume, UDecimal::ZERO);
        assert!(book.get_order(&order_id).is_none());
    }

    #[test]
    pub fn test_update_order_atomic_validation() {
        let mut book = OrderBook::new("ETH-USDT".to_string(), None);

        let (order_id, _) = book
            .add_order(
                OrderType::LimitBuy,
                Some(UDecimal::new(dec!(1050.0)).unwrap()),
                UDecimal::new(dec!(10.0)).unwrap(),
            )
            .expect("add order");

        // Attempting to update with invalid zero quantity must fail and keep old order intact
        let res = book.update_order(&order_id, Some(UDecimal::ZERO), None, None);
        assert!(res.is_err());
        assert!(book.get_order(&order_id).is_some());
        assert_eq!(book.buy_volume, UDecimal::new(dec!(10.0)).unwrap());

        // Valid update modifies the order
        let (updated, _) = book
            .update_order(
                &order_id,
                Some(UDecimal::new(dec!(15.0)).unwrap()),
                None,
                Some(UDecimal::new(dec!(1060.0)).unwrap()),
            )
            .expect("valid update");
        assert_eq!(updated.quantity(), UDecimal::new(dec!(15.0)).unwrap());
        assert_eq!(
            updated.price(),
            &Some(UDecimal::new(dec!(1060.0)).unwrap())
        );
        assert_eq!(book.buy_volume, UDecimal::new(dec!(15.0)).unwrap());
    }

    #[test]
    pub fn test_limit_matching_price_priority() {
        let mut book = OrderBook::new("BTC-USDT".to_string(), None);

        // Place asks: 1 BTC @ 50000, 1 BTC @ 51000
        book.add_order(
            OrderType::LimitSell,
            Some(UDecimal::new(dec!(50000.0)).unwrap()),
            UDecimal::new(dec!(1.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            OrderType::LimitSell,
            Some(UDecimal::new(dec!(51000.0)).unwrap()),
            UDecimal::new(dec!(1.0)).unwrap(),
        )
        .unwrap();

        // Place limit buy: 1.5 BTC @ 50500 -> Should fill 1.0 BTC @ 50000, 0.5 rests @ 50500
        let (buy_id, trades) = book
            .add_order(
                OrderType::LimitBuy,
                Some(UDecimal::new(dec!(50500.0)).unwrap()),
                UDecimal::new(dec!(1.5)).unwrap(),
            )
            .unwrap();

        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].quantity, UDecimal::new(dec!(1.0)).unwrap());
        assert_eq!(trades[0].price, UDecimal::new(dec!(50000.0)).unwrap());
        assert_eq!(
            trades[0].notional(),
            UDecimal::new(dec!(50000.0)).unwrap()
        );
        assert_eq!(book.buy_volume, UDecimal::new(dec!(0.5)).unwrap());
        assert_eq!(book.sell_volume, UDecimal::new(dec!(1.0)).unwrap());

        let resting = book.get_order(&buy_id).unwrap();
        assert_eq!(resting.quantity(), UDecimal::new(dec!(0.5)).unwrap());
    }

    #[test]
    pub fn test_market_order_matching() {
        let mut book = OrderBook::new("SOL-USDT".to_string(), None);

        book.add_order(
            OrderType::LimitSell,
            Some(UDecimal::new(dec!(100.0)).unwrap()),
            UDecimal::new(dec!(5.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            OrderType::LimitSell,
            Some(UDecimal::new(dec!(101.0)).unwrap()),
            UDecimal::new(dec!(5.0)).unwrap(),
        )
        .unwrap();

        let (_, trades) = book
            .add_order(OrderType::Buy, None, UDecimal::new(dec!(7.0)).unwrap())
            .unwrap();
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].quantity, UDecimal::new(dec!(5.0)).unwrap());
        assert_eq!(trades[0].price, UDecimal::new(dec!(100.0)).unwrap());
        assert_eq!(
            trades[0].notional(),
            UDecimal::new(dec!(500.0)).unwrap()
        );
        assert_eq!(trades[1].quantity, UDecimal::new(dec!(2.0)).unwrap());
        assert_eq!(trades[1].price, UDecimal::new(dec!(101.0)).unwrap());
        assert_eq!(
            trades[1].notional(),
            UDecimal::new(dec!(202.0)).unwrap()
        );

        assert_eq!(book.sell_volume, UDecimal::new(dec!(3.0)).unwrap());
    }

    #[test]
    pub fn test_depth_snapshot() {
        let mut book = OrderBook::new("AVAX-USDT".to_string(), None);
        book.add_order(
            OrderType::LimitBuy,
            Some(UDecimal::new(dec!(30.0)).unwrap()),
            UDecimal::new(dec!(10.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            OrderType::LimitBuy,
            Some(UDecimal::new(dec!(29.0)).unwrap()),
            UDecimal::new(dec!(20.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            OrderType::LimitSell,
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
            OrderType::LimitSell,
            Some(UDecimal::new(dec!(100.0)).unwrap()),
            UDecimal::new(dec!(2.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            OrderType::LimitSell,
            Some(UDecimal::new(dec!(101.0)).unwrap()),
            UDecimal::new(dec!(3.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            OrderType::LimitSell,
            Some(UDecimal::new(dec!(102.0)).unwrap()),
            UDecimal::new(dec!(5.0)).unwrap(),
        )
        .unwrap();

        // Limit buy with price 102 for 7 units matches best prices first: 100 -> 101 -> 102
        let (_, trades) = book
            .add_order(
                OrderType::LimitBuy,
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
            OrderType::LimitBuy,
            Some(UDecimal::new(dec!(200.0)).unwrap()),
            UDecimal::new(dec!(2.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            OrderType::LimitBuy,
            Some(UDecimal::new(dec!(199.0)).unwrap()),
            UDecimal::new(dec!(3.0)).unwrap(),
        )
        .unwrap();
        book.add_order(
            OrderType::LimitBuy,
            Some(UDecimal::new(dec!(198.0)).unwrap()),
            UDecimal::new(dec!(5.0)).unwrap(),
        )
        .unwrap();

        // Limit sell with price 198 for 7 units matches best prices first: 200 -> 199 -> 198
        let (_, trades) = book
            .add_order(
                OrderType::LimitSell,
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
        let (first_order_id, _) = book
            .add_order(
                OrderType::LimitSell,
                Some(UDecimal::new(dec!(100.0)).unwrap()),
                UDecimal::new(dec!(2.0)).unwrap(),
            )
            .unwrap();
        let (second_order_id, _) = book
            .add_order(
                OrderType::LimitSell,
                Some(UDecimal::new(dec!(100.0)).unwrap()),
                UDecimal::new(dec!(3.0)).unwrap(),
            )
            .unwrap();

        // Limit buy for 3.0 units should fill the first order completely (2.0) and second order partially (1.0)
        let (_, trades) = book
            .add_order(
                OrderType::LimitBuy,
                Some(UDecimal::new(dec!(100.0)).unwrap()),
                UDecimal::new(dec!(3.0)).unwrap(),
            )
            .unwrap();

        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].maker_order_id, first_order_id);
        assert_eq!(trades[0].quantity, UDecimal::new(dec!(2.0)).unwrap());
        assert_eq!(trades[1].maker_order_id, second_order_id);
        assert_eq!(trades[1].quantity, UDecimal::new(dec!(1.0)).unwrap());

        // First order must be gone, second order has 2.0 remaining
        assert!(book.get_order(&first_order_id).is_none());
        let second = book.get_order(&second_order_id).unwrap();
        assert_eq!(second.quantity(), UDecimal::new(dec!(2.0)).unwrap());
    }
}
