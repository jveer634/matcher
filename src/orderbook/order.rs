use chrono::Utc;
use rust_decimal::{prelude::FromPrimitive, Decimal};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum OrderType {
    Buy,
    Sell,
    LimitSell,
    LimitBuy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Open,
    Executed,
    Cancelled,
    PartiallyExecuted,
}

#[derive(Debug, Clone)]
pub struct Order {
    id: String,
    quantity: f64,
    order_type: OrderType,
    price: Option<Decimal>,
    status: OrderStatus,
    timestamp: i64,
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
            status: OrderStatus::Open,
        })
    }

    pub fn update(
        &mut self,
        order_type: Option<OrderType>,
        price: Option<f64>,
        quantity: Option<f64>,
    ) -> Result<Self, String> {
        if matches!(self.status, OrderStatus::Cancelled) {
            return Err("Order already cancelled".to_owned());
        }

        if order_type.is_some() {
            //  when price of a limit order is updated
            let order_type = order_type.unwrap();
            if matches!(order_type, OrderType::LimitBuy | OrderType::LimitSell) && price.is_none() {
                return Err("Limit Order needs a price".to_owned());
            }

            self.price = if matches!(order_type, OrderType::Buy | OrderType::Sell) {
                None
            } else {
                Decimal::from_f64(price.unwrap())
            };
            self.order_type = order_type;
        }

        if quantity.is_some() {
            self.quantity = quantity.unwrap();
        }
        let dt = Utc::now();
        self.timestamp = dt.timestamp();

        Ok(self.clone())
    }

    pub fn fill_order(&mut self, amount: f64) {
        self.quantity -= amount;
        if self.quantity == 0.0 {
            self.status = OrderStatus::Executed
        } else {
            self.status = OrderStatus::PartiallyExecuted
        }
    }

    pub fn cancel(&mut self) -> Result<(), String> {
        if self.status != OrderStatus::Open {
            return Err("Order can't be cancalled".to_string());
        }

        self.status = OrderStatus::Cancelled;
        Ok(())
    }

    pub fn is_filled(&self) -> bool {
        self.quantity == 0.0
    }

    pub fn quantity(&self) -> f64 {
        self.quantity
    }

    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn order_type(&self) -> &OrderType {
        &self.order_type
    }

    pub fn price(&self) -> &Option<Decimal> {
        &self.price
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
