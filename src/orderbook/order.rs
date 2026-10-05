use chrono::Utc;

use super::udecimal::UDecimal;

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
    quantity: UDecimal,
    initial_quantity: UDecimal,
    order_type: OrderType,
    price: Option<UDecimal>,
    status: OrderStatus,
    timestamp: i64,
}

impl Order {
    pub fn new(
        id: String,
        symbol: String,
        quantity: UDecimal,
        order_type: OrderType,
        price: Option<UDecimal>,
    ) -> Result<Order, String> {
        if quantity.is_zero() {
            return Err("Order quantity must be positive".to_string());
        }

        let decimal_price = match order_type {
            OrderType::LimitBuy | OrderType::LimitSell => {
                let p = price.ok_or_else(|| "Limit order requires a price".to_string())?;
                if p.is_zero() {
                    return Err("Order price must be positive".to_string());
                }
                Some(p)
            }
            OrderType::Buy | OrderType::Sell => None,
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
        price: Option<UDecimal>,
        quantity: Option<UDecimal>,
    ) -> Result<Self, String> {
        if matches!(self.status, OrderStatus::Cancelled | OrderStatus::Executed) {
            return Err("Cannot update a cancelled or executed order".to_owned());
        }

        let new_type = order_type.unwrap_or(self.order_type);
        let new_price = match new_type {
            OrderType::LimitBuy | OrderType::LimitSell => {
                let p = price.or(self.price).ok_or("Limit order requires a price")?;
                if p.is_zero() {
                    return Err("Price must be positive".to_string());
                }
                Some(p)
            }
            OrderType::Buy | OrderType::Sell => None,
        };

        if let Some(qty) = quantity {
            if qty.is_zero() {
                return Err("Updated quantity must be positive".to_string());
            }
            if self.status == OrderStatus::Open {
                self.initial_quantity = qty;
            }
            self.quantity = qty;
        }

        self.order_type = new_type;
        self.price = new_price;
        self.timestamp = Utc::now().timestamp_millis();
        Ok(self.clone())
    }

    pub fn fill_order(&mut self, amount: UDecimal) {
        if amount.is_zero() {
            return;
        }

        if amount >= self.quantity {
            self.quantity = UDecimal::ZERO;
            self.status = OrderStatus::Executed;
        } else {
            self.quantity -= amount;
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
        self.quantity.is_zero() || self.status == OrderStatus::Executed
    }

    pub fn quantity(&self) -> UDecimal {
        self.quantity
    }

    pub fn initial_quantity(&self) -> UDecimal {
        self.initial_quantity
    }

    pub fn filled_quantity(&self) -> UDecimal {
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

    pub fn price(&self) -> &Option<UDecimal> {
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
    use rust_decimal_macros::dec;

    #[test]
    pub fn test_order_creation() {
        let buy = Order::new(
            "o1".into(),
            "ETH-USDT".into(),
            UDecimal::new(dec!(10.0)).unwrap(),
            OrderType::Buy,
            None,
        );
        assert!(buy.is_ok());
        assert_eq!(buy.unwrap().price(), &None);

        let buy_with_extraneous_price = Order::new(
            "o1_m".into(),
            "ETH-USDT".into(),
            UDecimal::new(dec!(10.0)).unwrap(),
            OrderType::Buy,
            Some(UDecimal::new(dec!(3000.0)).unwrap()),
        );
        assert!(buy_with_extraneous_price.is_ok());
        assert_eq!(buy_with_extraneous_price.unwrap().price(), &None);

        let limit_buy = Order::new(
            "o2".into(),
            "ETH-USDT".into(),
            UDecimal::new(dec!(10.0)).unwrap(),
            OrderType::LimitBuy,
            Some(UDecimal::new(dec!(2500.0)).unwrap()),
        );
        assert!(limit_buy.is_ok());

        let limit_no_price = Order::new(
            "o3".into(),
            "ETH-USDT".into(),
            UDecimal::new(dec!(10.0)).unwrap(),
            OrderType::LimitBuy,
            None,
        );
        assert!(limit_no_price.is_err());

        let zero_price = Order::new(
            "o4".into(),
            "ETH-USDT".into(),
            UDecimal::new(dec!(10.0)).unwrap(),
            OrderType::LimitBuy,
            Some(UDecimal::ZERO),
        );
        assert!(zero_price.is_err());

        let zero_qty = Order::new(
            "o5".into(),
            "ETH-USDT".into(),
            UDecimal::ZERO,
            OrderType::Buy,
            None,
        );
        assert!(zero_qty.is_err());
    }

    #[test]
    pub fn test_order_fill_and_status() {
        let mut order = Order::new(
            "o1".into(),
            "BTC-USDT".into(),
            UDecimal::new(dec!(5.0)).unwrap(),
            OrderType::LimitBuy,
            Some(UDecimal::new(dec!(60000.0)).unwrap()),
        )
        .unwrap();

        order.fill_order(UDecimal::new(dec!(2.0)).unwrap());
        assert_eq!(*order.status(), OrderStatus::PartiallyExecuted);
        assert_eq!(order.quantity(), UDecimal::new(dec!(3.0)).unwrap());
        assert_eq!(order.filled_quantity(), UDecimal::new(dec!(2.0)).unwrap());

        order.fill_order(UDecimal::new(dec!(3.0)).unwrap());
        assert_eq!(*order.status(), OrderStatus::Executed);
        assert!(order.is_filled());
        assert_eq!(order.filled_quantity(), UDecimal::new(dec!(5.0)).unwrap());
    }

    #[test]
    pub fn test_order_update() {
        let mut order = Order::new(
            "o1".into(),
            "BTC-USDT".into(),
            UDecimal::new(dec!(5.0)).unwrap(),
            OrderType::LimitBuy,
            Some(UDecimal::new(dec!(60000.0)).unwrap()),
        )
        .unwrap();

        assert!(order
            .update(
                None,
                Some(UDecimal::new(dec!(61000.0)).unwrap()),
                Some(UDecimal::new(dec!(7.0)).unwrap())
            )
            .is_ok());
        assert_eq!(order.price(), &Some(UDecimal::new(dec!(61000.0)).unwrap()));
        assert_eq!(order.quantity(), UDecimal::new(dec!(7.0)).unwrap());
        assert_eq!(order.initial_quantity(), UDecimal::new(dec!(7.0)).unwrap());

        // Zero price should fail
        assert!(order.update(None, Some(UDecimal::ZERO), None).is_err());
        assert_eq!(order.price(), &Some(UDecimal::new(dec!(61000.0)).unwrap()));
    }
}
