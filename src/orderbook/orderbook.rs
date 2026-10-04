use std::collections::{BTreeMap, HashMap, VecDeque};

use rust_decimal::{prelude::FromPrimitive, Decimal};

use super::{
    id_generator::IdGenerator,
    order::{Order, OrderType},
    trade::Trade,
};

#[derive(Debug, Clone, PartialEq)]
pub struct BookLevel {
    pub price: Decimal,
    pub quantity: f64,
    pub order_count: usize,
}

#[derive(Debug)]
pub struct OrderBook {
    pair_id: String,
    id_generator: IdGenerator,
    buy_orders: BTreeMap<Decimal, VecDeque<Order>>,
    sell_orders: BTreeMap<Decimal, VecDeque<Order>>,
    pub sell_volume: f64,
    pub buy_volume: f64,
    order_index: HashMap<String, Order>,
    last_traded_price: Option<Decimal>,
}

impl OrderBook {
    pub fn new(pair_id: String, listing_price: Option<f64>) -> OrderBook {
        OrderBook {
            pair_id: pair_id.clone(),
            id_generator: IdGenerator::new(pair_id),
            buy_orders: BTreeMap::new(),
            sell_orders: BTreeMap::new(),
            order_index: HashMap::new(),
            last_traded_price: listing_price.and_then(Decimal::from_f64),
            sell_volume: 0.0,
            buy_volume: 0.0,
        }
    }

    pub fn pair_id(&self) -> &str {
        &self.pair_id
    }

    pub fn last_traded_price(&self) -> Option<Decimal> {
        self.last_traded_price
    }

    pub fn add_order(
        &mut self,
        order_type: OrderType,
        price: Option<f64>,
        quantity: f64,
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

        if !order.is_filled() && matches!(*order.order_type(), OrderType::LimitBuy | OrderType::LimitSell) {
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
                    self.order_index.insert(book_order.id().clone(), book_order.clone());
                    book_orders.push_front(book_order);
                    break;
                }

                if order.is_filled() {
                    break;
                }
            }

            if self.sell_orders.get(&lowest_ask_price).map_or(false, |q| q.is_empty()) {
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
                    self.order_index.insert(book_order.id().clone(), book_order.clone());
                    book_orders.push_front(book_order);
                    break;
                }

                if order.is_filled() {
                    break;
                }
            }

            if self.buy_orders.get(&highest_bid_price).map_or(false, |q| q.is_empty()) {
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
                    self.order_index.insert(book_order.id().clone(), book_order.clone());
                    book_orders.push_front(book_order);
                    break;
                }

                if order.is_filled() {
                    break;
                }
            }

            if self.sell_orders.get(&lowest_ask_price).map_or(false, |q| q.is_empty()) {
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
                    self.order_index.insert(book_order.id().clone(), book_order.clone());
                    book_orders.push_front(book_order);
                    break;
                }

                if order.is_filled() {
                    break;
                }
            }

            if self.buy_orders.get(&highest_bid_price).map_or(false, |q| q.is_empty()) {
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

        let price = order.price().ok_or("Cannot cancel market order with no price")?;

        let (orders, volume) = if *order.order_type() == OrderType::LimitBuy {
            (&mut self.buy_orders, &mut self.buy_volume)
        } else {
            (&mut self.sell_orders, &mut self.sell_volume)
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
        quantity: Option<f64>,
        order_type: Option<OrderType>,
        price: Option<f64>,
    ) -> Result<(Order, Vec<Trade>), String> {
        let old_order = self.cancel_order(order_id)?;

        let new_type = order_type.unwrap_or(*old_order.order_type());
        let new_qty = quantity.unwrap_or(old_order.quantity());
        let new_price = price.or_else(|| {
            old_order
                .price()
                .map(|p| p.to_string().parse::<f64>().unwrap_or(0.0))
        });

        let mut updated_order = Order::new(
            old_order.id().clone(),
            self.pair_id.clone(),
            new_qty,
            new_type,
            new_price,
        )?;

        let trades = match *updated_order.order_type() {
            OrderType::Buy => self.match_market_buy(&mut updated_order),
            OrderType::Sell => self.match_market_sell(&mut updated_order),
            OrderType::LimitBuy => self.match_limit_buy(&mut updated_order),
            OrderType::LimitSell => self.match_limit_sell(&mut updated_order),
        };

        if !updated_order.is_filled()
            && matches!(*updated_order.order_type(), OrderType::LimitBuy | OrderType::LimitSell)
        {
            self.order_index.insert(updated_order.id().clone(), updated_order.clone());
        }

        Ok((updated_order, trades))
    }

    pub fn get_order(&self, order_id: &str) -> Option<&Order> {
        self.order_index.get(order_id)
    }

    pub fn best_bid(&self) -> Option<(Decimal, f64)> {
        self.buy_orders.iter().next_back().map(|(price, queue)| {
            let qty: f64 = queue.iter().map(|o| o.quantity()).sum();
            (*price, qty)
        })
    }

    pub fn best_ask(&self) -> Option<(Decimal, f64)> {
        self.sell_orders.iter().next().map(|(price, queue)| {
            let qty: f64 = queue.iter().map(|o| o.quantity()).sum();
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

    #[test]
    pub fn test_add_and_cancel_limit_order() {
        let mut book = OrderBook::new("ETH-USDT".to_string(), Some(1000.0));

        let (order_id, trades) = book
            .add_order(OrderType::LimitBuy, Some(1050.0), 10.0)
            .expect("add order");
        assert_eq!(trades.len(), 0);
        assert_eq!(book.buy_volume, 10.0);
        assert!(book.get_order(&order_id).is_some());

        let cancelled = book.cancel_order(&order_id).expect("cancel");
        assert_eq!(*cancelled.status(), OrderStatus::Cancelled);
        assert_eq!(book.buy_volume, 0.0);
        assert!(book.get_order(&order_id).is_none());
    }

    #[test]
    pub fn test_limit_matching_price_priority() {
        let mut book = OrderBook::new("BTC-USDT".to_string(), None);

        // Place asks: 1 BTC @ 50000, 1 BTC @ 51000
        book.add_order(OrderType::LimitSell, Some(50000.0), 1.0).unwrap();
        book.add_order(OrderType::LimitSell, Some(51000.0), 1.0).unwrap();

        // Place limit buy: 1.5 BTC @ 50500 -> Should fill 1.0 BTC @ 50000, 0.5 rests @ 50500
        let (buy_id, trades) = book
            .add_order(OrderType::LimitBuy, Some(50500.0), 1.5)
            .unwrap();

        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].quantity, 1.0);
        assert_eq!(trades[0].price, Decimal::from_f64(50000.0).unwrap());
        assert_eq!(book.buy_volume, 0.5);
        assert_eq!(book.sell_volume, 1.0);

        let resting = book.get_order(&buy_id).unwrap();
        assert_eq!(resting.quantity(), 0.5);
    }

    #[test]
    pub fn test_market_order_matching() {
        let mut book = OrderBook::new("SOL-USDT".to_string(), None);

        book.add_order(OrderType::LimitSell, Some(100.0), 5.0).unwrap();
        book.add_order(OrderType::LimitSell, Some(101.0), 5.0).unwrap();

        let (_, trades) = book.add_order(OrderType::Buy, None, 7.0).unwrap();
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].quantity, 5.0);
        assert_eq!(trades[0].price, Decimal::from_f64(100.0).unwrap());
        assert_eq!(trades[1].quantity, 2.0);
        assert_eq!(trades[1].price, Decimal::from_f64(101.0).unwrap());

        assert_eq!(book.sell_volume, 3.0);
    }

    #[test]
    pub fn test_depth_snapshot() {
        let mut book = OrderBook::new("AVAX-USDT".to_string(), None);
        book.add_order(OrderType::LimitBuy, Some(30.0), 10.0).unwrap();
        book.add_order(OrderType::LimitBuy, Some(29.0), 20.0).unwrap();
        book.add_order(OrderType::LimitSell, Some(31.0), 15.0).unwrap();

        let (bids, asks) = book.get_depth(5);
        assert_eq!(bids.len(), 2);
        assert_eq!(bids[0].price, Decimal::from_f64(30.0).unwrap());
        assert_eq!(bids[0].quantity, 10.0);
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].price, Decimal::from_f64(31.0).unwrap());
    }
}
