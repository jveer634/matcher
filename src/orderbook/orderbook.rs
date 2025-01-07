use std::collections::{BTreeMap, HashMap, VecDeque};

use super::{
    id_generator::IdGenerator,
    order::{Order, OrderType},
    price::Price,
};

#[derive(Debug)]
pub struct OrderBook {
    id_generator: IdGenerator,
    buy_orders: BTreeMap<Price, VecDeque<Order>>,
    sell_orders: BTreeMap<Price, VecDeque<Order>>,
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
                        orders.push_back(order);
                    }
                    OrderType::LimitSell => {
                        let orders = self
                            .sell_orders
                            .entry(order.price.unwrap().clone())
                            .or_insert(VecDeque::new());
                        orders.push_back(order);
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
                    if order.order_type == OrderType::LimitSell && *price > order.price.unwrap() {
                        break;
                    }

                    while let Some(sell_order) = orders.front_mut() {
                        let traded_quantity = order.quantity.min(sell_order.quantity);

                        sell_order.quantity -= traded_quantity;
                        order.quantity -= traded_quantity;

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
            }
            OrderType::Sell | OrderType::LimitSell => {
                let sell_orders = &mut self.buy_orders;
                for (price, orders) in sell_orders.iter_mut() {
                    if order.order_type == OrderType::LimitBuy && *price > order.price.unwrap() {
                        break;
                    }

                    while let Some(buy_order) = orders.front_mut() {
                        let traded_quantity = order.quantity.min(buy_order.quantity);

                        buy_order.quantity -= traded_quantity;
                        order.quantity -= traded_quantity;

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
            }
        }
        if order.quantity > 0.0 {
            println!("Handle Low liquity");
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
                let orders = if order.order_type == OrderType::LimitBuy
                    || order.order_type == OrderType::Buy
                {
                    self.buy_orders.get_mut(&order.price.unwrap())
                } else {
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
                            return match order.update(order_type, price, quantity) {
                                Ok(_) => {
                                    if matches!(
                                        order_type,
                                        OrderType::LimitBuy | OrderType::LimitSell
                                    ) && price.is_none()
                                    {
                                        orders.push_back(order.clone());
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
}
