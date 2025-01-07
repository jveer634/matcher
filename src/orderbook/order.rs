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
    pub fn new(
        id: String,
        quantity: f64,
        order_type: OrderType,
        price: Option<f64>,
    ) -> Result<Order, String> {
        let price = match price {
            Some(p) => Some(Price::new(p)),
            None => None,
        };

        let dt = Utc::now();

        if matches!(order_type, OrderType::LimitBuy | OrderType::LimitSell) && price.is_none() {
            return Err("Limit Order needs a price".to_owned());
        }

        Ok(Order {
            id,
            quantity,
            order_type,
            price,
            timestamp: dt.timestamp(),
        })
    }

    pub fn update(
        &mut self,
        order_type: OrderType,
        price: Option<f64>,
        quantity: f64,
    ) -> Result<(), String> {
        if matches!(order_type, OrderType::LimitBuy | OrderType::LimitSell) && price.is_none() {
            return Err("Limit Order needs a price".to_owned());
        }

        let dt = Utc::now();
        self.price = if matches!(order_type, OrderType::Buy | OrderType::Sell) {
            None
        } else {
            Some(Price::new(price.unwrap()))
        };

        self.quantity = quantity;
        self.order_type = order_type;
        self.timestamp = dt.timestamp();

        Ok(())
    }
}
