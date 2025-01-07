use chrono::Utc;

use super::price::Price;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum OrderType {
    Buy,
    Sell,
    LimitSell,
    LimitBuy,
}

#[derive(Debug, Clone)]
pub struct Order {
    pub id: String,
    pub quantity: f64,
    pub order_type: OrderType,
    pub price: Option<Price>,
    pub timestamp: i64,
}

impl Order {
    pub fn new(id: String, quantity: f64, order_type: OrderType, price: Option<f64>) -> Order {
        let price = match price {
            Some(p) => Some(Price::new(p)),
            None => None,
        };

        let dt = Utc::now();

        Order {
            id,
            quantity,
            order_type,
            price,
            timestamp: dt.timestamp(),
        }
    }
}
