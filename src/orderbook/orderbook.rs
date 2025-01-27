use std::collections::{BTreeMap, HashMap, VecDeque};

use super::{
    id_generator::IdGenerator,
    order::{Order, OrderType},
};

use rust_decimal::{prelude::FromPrimitive, Decimal};

#[derive(Debug)]
pub struct Trade {
    order: Order,
    book_order: Order,
    quantity: f64,
}

#[derive(Debug)]
pub struct OrderBook {
    id_generator: IdGenerator,
    buy_orders: BTreeMap<Decimal, VecDeque<Order>>,
    sell_orders: BTreeMap<Decimal, VecDeque<Order>>,
    pub sell_volume: f64,
    pub buy_volume: f64,
    order_index: HashMap<String, Order>,
    last_traded_price: Decimal,
}

impl OrderBook {
    pub fn new(pair_id: String, listing_price: f64) -> OrderBook {
        OrderBook {
            id_generator: IdGenerator::new(pair_id),
            buy_orders: BTreeMap::new(),
            sell_orders: BTreeMap::new(),
            order_index: HashMap::new(),
            last_traded_price: Decimal::from_f64(listing_price).unwrap(),
            sell_volume: 0.0,
            buy_volume: 0.0,
        }
    }

    pub fn add_order(
        &mut self,
        order_type: OrderType,
        price: Option<f64>,
        quantity: f64,
    ) -> Result<String, String> {
        let id = self.id_generator.generate_order_id();
        let order = Order::new(id.clone(), quantity, order_type, price)?;

        self.order_index.insert(order.id().clone(), order.clone());

        if matches!(*order.order_type(), OrderType::Buy | OrderType::Sell) {
            self.match_market_order(order);
        } else {
            self.match_limit_order(order);
        }
        return Ok(id);
    }

    fn match_market_order(&mut self, mut order: Order) {
        let mut trades: Vec<Trade> = Vec::new();

        let (orders, volume) = if *order.order_type() == OrderType::Buy {
            (&mut self.sell_orders, &mut self.sell_volume)
        } else {
            (&mut self.buy_orders, &mut self.buy_volume)
        };

        for (_, orders) in orders.iter_mut() {
            while let Some(book_order) = orders.front_mut() {
                let traded_quantity = order.quantity().min(book_order.quantity());

                book_order.fill_order(traded_quantity);
                order.fill_order(traded_quantity);

                *volume -= traded_quantity;

                trades.push(Trade {
                    book_order: book_order.clone(),
                    order: order.clone(),
                    quantity: traded_quantity,
                });

                if book_order.is_filled() {
                    self.order_index.remove(book_order.id());
                    orders.remove(0);
                }

                if order.is_filled() {
                    break;
                }
            }

            println!("Trades happened: {:?} ", trades);
        }
    }

    fn match_limit_order(&mut self, mut order: Order) {
        let (orders, volume) = if *order.order_type() == OrderType::LimitBuy {
            (
                self.sell_orders.get_mut(&order.price().unwrap()),
                &mut self.sell_volume,
            )
        } else {
            (
                self.buy_orders.get_mut(&order.price().unwrap()),
                &mut self.buy_volume,
            )
        };

        println!("Book Orders {orders:#?}, order: {order:#?}");

        match orders {
            None => {
                if *order.order_type() == OrderType::LimitBuy {
                    self.buy_volume += order.quantity();
                    self.buy_orders
                        .entry(order.price().unwrap())
                        .or_insert(VecDeque::new())
                        .push_back(order);
                } else {
                    self.sell_volume += order.quantity();
                    self.sell_orders
                        .entry(order.price().unwrap())
                        .or_insert(VecDeque::new())
                        .push_back(order);
                }
            }
            Some(orders) => {
                let mut trades: Vec<Trade> = Vec::new();
                while let Some(book_order) = orders.front_mut() {
                    let traded_quantity = order.quantity().min(book_order.quantity());

                    book_order.fill_order(traded_quantity);
                    order.fill_order(traded_quantity);

                    *volume -= traded_quantity;

                    trades.push(Trade {
                        book_order: book_order.clone(),
                        order: order.clone(),
                        quantity: traded_quantity,
                    });

                    if book_order.is_filled() {
                        self.order_index.remove(book_order.id());
                        orders.remove(0);
                    }

                    if order.is_filled() {
                        break;
                    }
                }

                if !order.is_filled() {
                    if *order.order_type() == OrderType::LimitBuy {
                        self.buy_volume += order.quantity();
                        self.buy_orders
                            .entry(order.price().unwrap())
                            .or_insert(VecDeque::new())
                            .push_back(order);
                    } else {
                        self.sell_volume += order.quantity();
                        self.sell_orders
                            .entry(order.price().unwrap())
                            .or_insert(VecDeque::new())
                            .push_back(order);
                    }
                }
            }
        }
    }

    pub fn cancel_order(&mut self, order_id: String) -> Result<(), String> {
        let (_, order) = self
            .order_index
            .remove_entry(&order_id)
            .ok_or("Invalid Order Id".to_string())?;

        let orders = if *order.order_type() == OrderType::LimitBuy {
            self.buy_volume -= order.quantity();
            self.buy_orders.get_mut(&order.price().unwrap())
        } else {
            self.sell_volume -= order.quantity();
            self.sell_orders.get_mut(&order.price().unwrap())
        };

        let orders = orders.ok_or("Order already executed".to_string())?;
        orders.retain(|o| *o.id() != order_id);

        Ok(())
    }

    // only limit order can be converted to market order or limit order parameters can be updated
    pub fn update_order(
        &mut self,
        order_id: String,
        quantity: Option<f64>,
        order_type: Option<OrderType>,
        price: Option<f64>,
    ) -> Result<(), String> {
        let (_, order) = self
            .order_index
            .remove_entry(&order_id)
            .ok_or("Invalid Order Id".to_string())?;

        let orders = if matches!(*order.order_type(), OrderType::LimitBuy | OrderType::Buy) {
            self.buy_orders.get_mut(&order.price().unwrap())
        } else {
            self.sell_orders.get_mut(&order.price().unwrap())
        };

        let orders = orders.ok_or("Order already executed".to_string())?;

        if let Some(pos) = orders.iter().position(|order| *order.id() == order_id) {
            let mut order = orders.remove(pos).unwrap(); // Remove and get the order
            if *order.order_type() == OrderType::LimitBuy {
                self.buy_volume -= order.quantity();
            }
            if *order.order_type() == OrderType::LimitSell {
                self.sell_volume -= order.quantity();
            }

            let updated_order = order.update(order_type, price, quantity)?;
            self.order_index.insert(order.id().clone(), order.clone());

            if matches!(
                updated_order.order_type(),
                OrderType::LimitBuy | OrderType::LimitSell
            ) {
                orders.push_back(order.clone());
            }

            if matches!(
                *updated_order.order_type(),
                OrderType::LimitBuy | OrderType::LimitSell
            ) {
                if *updated_order.order_type() == OrderType::LimitBuy {
                    self.buy_volume += order.quantity();
                } else {
                    self.sell_volume += order.quantity();
                }

                self.match_limit_order(order);
            } else {
                self.match_market_order(order);
            }
        }
        Ok(())
    }

   
    pub fn get_order(&self, order_id: String) -> Option<&Order> {
        self.order_index.get(&order_id)
    }


}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn pass_add_order() {
        let listing_price = 1023.0;
        let pair_id = String::from("ETHINC");
        let sell_quantity = 1232.5;
        let buy_quantity = 23243.5;

        let mut book = OrderBook::new(pair_id, listing_price);

        let order_id = book
            .add_order(
                OrderType::LimitBuy,
                Some(listing_price + 123.2),
                buy_quantity,
            )
            .expect("can't add limit buy with price");

        assert_eq!(book.buy_volume, buy_quantity);

        let order = book.get_order(order_id).unwrap();

        assert_eq!(*order.order_type(), OrderType::LimitBuy);

        book.add_order(OrderType::LimitSell, Some(listing_price), sell_quantity)
            .expect("Unable to add a limit sell");
        assert_eq!(book.sell_volume, sell_quantity);
    }

    #[test]
    pub fn pass_update_order() {
        let listing_price = 1023.0;
        let pair_id = String::from("ETHINC");
        let buy_quantity = 23243.5;

        let mut book = OrderBook::new(pair_id, listing_price);

        let order_id = book
            .add_order(
                OrderType::LimitBuy,
                Some(listing_price + 123.2),
                buy_quantity,
            )
            .expect("can't add limit buy with price");

        let order = book.get_order(order_id.clone()).unwrap();
        assert_eq!(book.buy_volume, buy_quantity);
        assert_eq!(order.quantity(), buy_quantity);

        let buy_quantity = buy_quantity - 100.0;

        book.update_order(
            order_id.clone(),
            Some(buy_quantity),
            Some(OrderType::Buy),
            None,
        )
        .expect("Update order failed");

        let order = book.get_order(order_id.clone()).unwrap();

        // once market order is converted to limit order, it is removed from orderbook
        assert_eq!(book.buy_volume, 0.0);
        assert_eq!(order.quantity(), buy_quantity);
    }

    #[test]
    pub fn pass_cancel_order() {
        let listing_price = 1023.0;
        let pair_id = String::from("ETHINC");
        let buy_quantity = 23243.5;

        let mut book = OrderBook::new(pair_id, listing_price);

        let order_id = book
            .add_order(
                OrderType::LimitBuy,
                Some(listing_price + 123.2),
                buy_quantity,
            )
            .expect("can't add limit buy with price");

        book.cancel_order(order_id.clone())
            .expect("Failled cancelling order");

        // once market order is converted to limit order, it is removed from orderbook
        assert_eq!(book.buy_volume, 0.0);
        assert!(book.get_order(order_id.clone()).is_none());
    }

    #[test]
    pub fn pass_match_limit_orders() {
        let listing_price = 1023.0;
        let pair_id = String::from("ETHINC");
        let quantity = 23243.5;

        let mut book = OrderBook::new(pair_id, listing_price);

        book.add_order(OrderType::LimitBuy, Some(listing_price), quantity)
            .expect("can't add limit buy with price");

        book.add_order(OrderType::LimitSell, Some(listing_price), quantity)
            .expect("Can't add limit sell order");

        dbg!(&book);

        assert_eq!(book.buy_volume, 0.0);
        assert_eq!(book.sell_volume, 0.0);
    }
}
