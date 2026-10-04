use chrono::Utc;
use rust_decimal::{prelude::FromPrimitive, Decimal};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum OrderType {
    Buy,
    Sell,
    LimitBuy,
    LimitSell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Open,
    Executed,
    Cancelled,
    PartiallyExecuted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    id: String,
    symbol: String,
    quantity: f64,
    initial_quantity: f64,
    order_type: OrderType,
    price: Option<Decimal>,
    status: OrderStatus,
    timestamp: i64,
}

impl Order {
    pub fn new(
        id: String,
        symbol: String,
        quantity: f64,
        order_type: OrderType,
        price: Option<f64>,
    ) -> Result<Order, String> {
        if quantity <= 0.0 {
            return Err("Order quantity must be positive".to_string());
        }

        let decimal_price = match price {
            Some(p) => {
                if p <= 0.0 {
                    return Err("Order price must be positive".to_string());
                }
                Some(Decimal::from_f64(p).ok_or("Invalid price decimal")?)
            }
            None => {
                if matches!(order_type, OrderType::LimitBuy | OrderType::LimitSell) {
                    return Err("Limit order requires a price".to_owned());
                }
                None
            }
        };

        let dt = Utc::now();

        Ok(Order {
            id,
            symbol,
            quantity,
            initial_quantity: quantity,
            order_type,
            price: decimal_price,
            timestamp: dt.timestamp_millis(),
            status: OrderStatus::Open,
        })
    }

    pub fn update(
        &mut self,
        order_type: Option<OrderType>,
        price: Option<f64>,
        quantity: Option<f64>,
    ) -> Result<Self, String> {
        if matches!(self.status, OrderStatus::Cancelled | OrderStatus::Executed) {
            return Err("Cannot update a cancelled or executed order".to_owned());
        }

        if let Some(new_type) = order_type {
            if matches!(new_type, OrderType::LimitBuy | OrderType::LimitSell) {
                let p = price
                    .or_else(|| self.price.map(|dp| dp.to_string().parse::<f64>().unwrap_or(0.0)))
                    .ok_or("Limit order requires a price")?;
                if p <= 0.0 {
                    return Err("Price must be positive".to_string());
                }
                self.price = Some(Decimal::from_f64(p).ok_or("Invalid price decimal")?);
            } else {
                self.price = None;
            }
            self.order_type = new_type;
        } else if let Some(p) = price {
            if matches!(self.order_type, OrderType::LimitBuy | OrderType::LimitSell) {
                if p <= 0.0 {
                    return Err("Price must be positive".to_string());
                }
                self.price = Some(Decimal::from_f64(p).ok_or("Invalid price decimal")?);
            }
        }

        if let Some(qty) = quantity {
            if qty <= 0.0 {
                return Err("Updated quantity must be positive".to_string());
            }
            self.quantity = qty;
        }

        self.timestamp = Utc::now().timestamp_millis();
        Ok(self.clone())
    }

    pub fn fill_order(&mut self, amount: f64) {
        self.quantity = (self.quantity - amount).max(0.0);
        if self.quantity <= 1e-9 {
            self.quantity = 0.0;
            self.status = OrderStatus::Executed;
        } else {
            self.status = OrderStatus::PartiallyExecuted;
        }
    }

    pub fn cancel(&mut self) -> Result<(), String> {
        if matches!(self.status, OrderStatus::Cancelled | OrderStatus::Executed) {
            return Err("Order cannot be cancelled".to_string());
        }

        self.status = OrderStatus::Cancelled;
        Ok(())
    }

    pub fn is_filled(&self) -> bool {
        self.quantity == 0.0 || self.status == OrderStatus::Executed
    }

    pub fn quantity(&self) -> f64 {
        self.quantity
    }

    pub fn initial_quantity(&self) -> f64 {
        self.initial_quantity
    }

    pub fn filled_quantity(&self) -> f64 {
        self.initial_quantity - self.quantity
    }

    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn symbol(&self) -> &String {
        &self.symbol
    }

    pub fn order_type(&self) -> &OrderType {
        &self.order_type
    }

    pub fn price(&self) -> &Option<Decimal> {
        &self.price
    }

    pub fn status(&self) -> &OrderStatus {
        &self.status
    }

    pub fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_order_creation() {
        let buy = Order::new(
            "o1".into(),
            "ETH-USDT".into(),
            10.0,
            OrderType::Buy,
            None,
        );
        assert!(buy.is_ok());

        let limit_buy = Order::new(
            "o2".into(),
            "ETH-USDT".into(),
            10.0,
            OrderType::LimitBuy,
            Some(2500.0),
        );
        assert!(limit_buy.is_ok());

        let limit_no_price = Order::new(
            "o3".into(),
            "ETH-USDT".into(),
            10.0,
            OrderType::LimitBuy,
            None,
        );
        assert!(limit_no_price.is_err());
    }

    #[test]
    pub fn test_order_fill_and_status() {
        let mut order = Order::new(
            "o1".into(),
            "BTC-USDT".into(),
            5.0,
            OrderType::LimitBuy,
            Some(60000.0),
        )
        .unwrap();

        order.fill_order(2.0);
        assert_eq!(*order.status(), OrderStatus::PartiallyExecuted);
        assert_eq!(order.quantity(), 3.0);

        order.fill_order(3.0);
        assert_eq!(*order.status(), OrderStatus::Executed);
        assert!(order.is_filled());
    }
}
