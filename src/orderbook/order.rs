use chrono::Utc;
use rust_decimal::{prelude::FromPrimitive, Decimal};

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
    pub price: Option<Decimal>,
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
            Some(p) => Some(Decimal::from_f64(p).unwrap()),
            None => {
                if matches!(order_type, OrderType::LimitBuy | OrderType::LimitSell)
                    && price.is_none()
                {
                    return Err("Limit Order needs a price".to_owned());
                };

                None
            }
        };
        let dt = Utc::now();

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
            Decimal::from_f64(price.unwrap())
        };

        self.quantity = quantity;
        self.order_type = order_type;
        self.timestamp = dt.timestamp();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn pass_creating_orders() {
        let _buy_order =
            Order::new("buy_order".to_owned(), 12.0, super::OrderType::Buy, None).unwrap();
        let _sell_order =
            Order::new("sell_order".to_owned(), 12.0, super::OrderType::Sell, None).unwrap();

        let _limit_buy_order = Order::new(
            "limit_buy_order".to_owned(),
            12.0,
            super::OrderType::LimitBuy,
            Some(12.3),
        )
        .unwrap();
        let _sell_order = Order::new(
            "buy_order".to_owned(),
            12.0,
            super::OrderType::LimitBuy,
            Some(12.54),
        )
        .unwrap();
    }

    #[test]
    #[should_panic]
    pub fn fail_creating_orders() {
        let _limit_buy_order = Order::new(
            "limit_buy_order".to_owned(),
            12.0,
            super::OrderType::LimitBuy,
            None,
        )
        .unwrap();
        let _sell_order = Order::new(
            "buy_order".to_owned(),
            12.0,
            super::OrderType::LimitBuy,
            None,
        )
        .unwrap();
    }
}
