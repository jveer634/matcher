use std::collections::{BTreeMap, HashMap, VecDeque};

use super::{
    id_generator::IdGenerator,
    order::{self, Order, OrderType},
    price::Price,
};

#[derive(Debug)]
pub struct OrderBook {
    id_generator: IdGenerator,
    buy_orders: BTreeMap<Price, VecDeque<Order>>,
    sell_orders: BTreeMap<Price, VecDeque<Order>>,
    pub sell_volume: f64,
    pub buy_volume: f64,
    order_index: HashMap<String, Order>,
    last_traded_price: Price,
}

impl OrderBook {
    pub fn new(pair_id: String, listing_price: f64) -> OrderBook {
        OrderBook {
            id_generator: IdGenerator::new(pair_id),
            buy_orders: BTreeMap::new(),
            sell_orders: BTreeMap::new(),
            order_index: HashMap::new(),
            last_traded_price: Price::new(listing_price),
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
        let order = Order::new(id.clone(), quantity, order_type, price);

        match order {
            Ok(order) => {
                self.order_index.insert(order.id.clone(), order.clone());
                match order.order_type {
                    OrderType::LimitBuy => {
                        let orders = self
                            .buy_orders
                            .entry(order.price.unwrap().clone())
                            .or_insert(VecDeque::new());
                        self.buy_volume += order.quantity;
                        orders.push_back(order.clone());
                        self.match_order(order);

                        dbg!("Order reived limit buy");
                    }
                    OrderType::LimitSell => {
                        let orders = self
                            .sell_orders
                            .entry(order.price.unwrap().clone())
                            .or_insert(VecDeque::new());
                        self.sell_volume += order.quantity;

                        dbg!(self.sell_volume);
                        orders.push_back(order.clone());
                        self.match_order(order);
                        dbg!("Order added limit sell");
                    }
                    OrderType::Buy => self.match_order(order),
                    OrderType::Sell => self.match_order(order),
                }

                return Ok(id);
            }

            Err(err) => return Err(err),
        }
    }

    fn match_order(&mut self, mut order: Order) {
        let mut trades = Vec::new();
        match order.order_type {
            OrderType::Buy | OrderType::LimitBuy => {
                let sell_orders = &mut self.sell_orders;
                for (price, orders) in sell_orders.iter_mut() {
                    if order.order_type == OrderType::LimitBuy && order.price.unwrap() > *price {
                        continue;
                    }

                    while let Some(sell_order) = orders.front_mut() {
                        let traded_quantity = order.quantity.min(sell_order.quantity);

                        sell_order.quantity -= traded_quantity;
                        order.quantity -= traded_quantity;

                        self.buy_volume -= traded_quantity;
                        self.sell_volume -= traded_quantity;

                        trades.push((sell_order.clone(), order.clone(), traded_quantity));

                        if sell_order.quantity == 0.0 {
                            self.order_index.remove(&sell_order.id);
                            orders.remove(0);
                        }

                        if order.quantity == 0.0 {
                            break;
                        }
                    }
                }
                // if order.quantity > 0.0 {
                //     let orders = sell_orders
                //         .entry(self.last_traded_price.clone())
                //         .or_insert(VecDeque::new());
                //     self.sell_volume += order.quantity;
                //     orders.push_back(order);
                // }
            }
            OrderType::Sell | OrderType::LimitSell => {
                let buy_orders = &mut self.buy_orders;
                for (price, orders) in buy_orders.iter_mut() {
                    if order.order_type == OrderType::LimitSell && order.price.unwrap() < *price {
                        continue;
                    }

                    while let Some(buy_order) = orders.front_mut() {
                        let traded_quantity = order.quantity.min(buy_order.quantity);

                        buy_order.quantity -= traded_quantity;
                        order.quantity -= traded_quantity;

                        self.sell_volume -= traded_quantity;
                        self.buy_volume -= traded_quantity;

                        trades.push((buy_order.clone(), order.clone(), traded_quantity));

                        if buy_order.quantity == 0.0 {
                            self.order_index.remove(&buy_order.id);
                            orders.remove(0);
                        }
                        if order.quantity == 0.0 {
                            break;
                        }
                    }
                }

                // if order.quantity > 0.0 {
                //     let orders = buy_orders
                //         .entry(self.last_traded_price.clone())
                //         .or_insert(VecDeque::new());
                //     self.sell_volume += order.quantity;
                //     orders.push_back(order);
                // }
            }
        }

        for (sell, buy, qty) in trades {
            self.last_traded_price = sell.price.unwrap();
            println!(
                "Trade executed: BUY ({:?}) <--> SELL ({:?}), Qty: {}, Price: {:?}",
                buy, sell, qty, sell.price
            );
        }
    }

    pub fn cancel_order(&mut self, order_id: String) -> Result<(), String> {
        match self.order_index.remove_entry(&order_id) {
            Some((_key, order)) => {
                let orders = if order.order_type == OrderType::LimitBuy {
                    self.buy_volume -= order.quantity;
                    self.buy_orders.get_mut(&order.price.unwrap())
                } else {
                    self.sell_volume -= order.quantity;
                    self.sell_orders.get_mut(&order.price.unwrap())
                };

                match orders {
                    Some(orders) => {
                        // delete order from the vector here
                        orders.retain(|o| o.id != order_id);
                        return Ok(());
                    }
                    None => return Err("Order already executed".to_string()),
                }
            }
            None => Err("Invalid OrderId".to_owned()),
        }
    }

    pub fn update_order(
        &mut self,
        order_id: String,
        quantity: f64,
        order_type: OrderType,
        price: Option<f64>,
    ) -> Result<(), String> {
        match self.order_index.remove_entry(&order_id) {
            Some((_, order)) => {
                let orders = if order.order_type == OrderType::LimitBuy
                    || order.order_type == OrderType::Buy
                {
                    self.buy_orders.get_mut(&order.price.unwrap())
                } else {
                    self.sell_orders.get_mut(&order.price.unwrap())
                };

                match orders {
                    Some(orders) => {
                        if let Some(pos) = orders.iter().position(|order| order.id == order_id) {
                            let mut order = orders.remove(pos).unwrap(); // Remove and get the order
                            if order.order_type == OrderType::LimitBuy {
                                self.buy_volume -= order.quantity;
                            }
                            if order.order_type == OrderType::LimitSell {
                                self.sell_volume -= order.quantity;
                            }

                            return match order.update(order_type, price, quantity) {
                                Ok(_) => {
                                    self.order_index.insert(order.id.clone(), order.clone());
                                    if matches!(
                                        order_type,
                                        OrderType::LimitBuy | OrderType::LimitSell
                                    ) {
                                        orders.push_back(order.clone());
                                    }

                                    if order_type == OrderType::LimitBuy {
                                        self.buy_volume += order.quantity;
                                    }
                                    if order_type == OrderType::LimitSell {
                                        self.sell_volume += order.quantity;
                                    }
                                    self.match_order(order);
                                    Ok(())
                                }
                                Err(err) => Err(err),
                            };
                        }
                        Ok(())
                    }
                    None => return Err("Order already executed".to_string()),
                }
            }
            None => Err("Invalid OrderId".to_owned()),
        }
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

        assert_eq!(order.order_type, OrderType::LimitBuy);

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
        assert_eq!(order.quantity, buy_quantity);

        let buy_quantity = buy_quantity - 100.0;

        book.update_order(order_id.clone(), buy_quantity, OrderType::Buy, None)
            .expect("Update order failed");

        let order = book.get_order(order_id.clone()).unwrap();

        // once market order is converted to limit order, it is removed from orderbook
        assert_eq!(book.buy_volume, 0.0);
        assert_eq!(order.quantity, buy_quantity);
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

        let buy = book
            .add_order(OrderType::LimitBuy, Some(listing_price), quantity)
            .expect("can't add limit buy with price");

        let sell = book.add_order(OrderType::LimitSell, Some(listing_price), quantity);

        assert_eq!(book.buy_volume, 0.0);
        assert_eq!(book.sell_volume, 0.0);
    }
}
