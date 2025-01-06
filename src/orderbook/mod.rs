use std::collections::BTreeMap;

pub const SCALAR: f64 = 10000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Price {
    integral: u64,
    fractional: u64,
}

impl Price {
    fn new(price: f64) -> Price {
        Price {
            integral: price as u64,
            fractional: ((price % 1.0) * SCALAR) as u64,
        }
    }

    pub fn to_f64(&self) -> f64 {
        (self.integral) as f64 + (self.fractional as f64 / SCALAR)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum OrderType {
    Buy,
    Sell,
    LimitSell,
    LimitBuy,
}

#[derive(Debug, Clone, Copy)]
pub struct Order {
    quantity: f64,
    order_type: OrderType,
    price: Option<Price>,
}

impl Order {
    pub fn new(quantity: f64, order_type: OrderType, price: Option<f64>) -> Order {
        let price = match price {
            Some(p) => Some(Price::new(p)),
            None => None,
        };
        Order {
            quantity,
            order_type,
            price,
        }
    }
}

#[derive(Debug, Default)]
pub struct OrderBook {
    order_count: u128,
    buy_orders: BTreeMap<Price, Vec<Order>>,
    sell_orders: BTreeMap<Price, Vec<Order>>,
}

impl OrderBook {
    pub fn new() -> OrderBook {
        OrderBook::default()
    }

    pub fn add_order(&mut self, order_type: OrderType, price: Option<f64>, quantity: f64) -> u128 {
        let count = self.order_count;
        let order = Order::new(quantity, order_type, price);

        match order.order_type {
            OrderType::LimitBuy => {
                let p = Price::new(price.expect("LimitBuy needs a price"));
                let orders = self.buy_orders.entry(p.clone()).or_insert(vec![]);
                orders.push(order);
            }
            OrderType::LimitSell => {
                let p = Price::new(price.expect("LimitSell needs a price"));
                let orders = self.sell_orders.entry(p.clone()).or_insert(vec![]);
                orders.push(order);
            }
            OrderType::Buy => self.match_order(order),
            OrderType::Sell => self.match_order(order),
        }
        count
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

                    while let Some(sell_order) = orders.first_mut() {
                        let traded_quantity = order.quantity.min(sell_order.quantity);

                        sell_order.quantity -= traded_quantity;
                        order.quantity -= traded_quantity;

                        trades.push((sell_order.clone(), order.clone(), traded_quantity));

                        if sell_order.quantity == 0.0 {
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

                    while let Some(buy_order) = orders.first_mut() {
                        let traded_quantity = order.quantity.min(buy_order.quantity);

                        buy_order.quantity -= traded_quantity;
                        order.quantity -= traded_quantity;

                        trades.push((buy_order.clone(), order.clone(), traded_quantity));

                        if buy_order.quantity == 0.0 {
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
            println!(
                "Trade executed: BUY ({:?}) <--> SELL ({:?}), Qty: {}, Price: {:?}",
                buy, sell, qty, sell.price
            );
        }
    }
}
