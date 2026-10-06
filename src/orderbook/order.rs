use chrono::Utc;

pub use super::types::{OrderStatus, OrderType, Side};
use super::udecimal::UDecimal;

/// Core Order structure representing a participant's order in the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    id: String,
    user_id: String,
    symbol: String,
    side: Side,
    order_type: OrderType,
    quantity: UDecimal,
    initial_quantity: UDecimal,
    price: Option<UDecimal>,
    status: OrderStatus,
    timestamp: i64,
}

impl Order {
    pub fn new(
        id: String,
        user_id: String,
        symbol: String,
        side: Side,
        order_type: OrderType,
        quantity: UDecimal,
        price: Option<UDecimal>,
    ) -> Result<Order, String> {
        if id.trim().is_empty() {
            return Err("Order id cannot be empty".to_string());
        }

        if user_id.trim().is_empty() {
            return Err("User id cannot be empty".to_string());
        }

        if quantity.is_zero() {
            return Err("Order quantity must be positive".to_string());
        }

        let decimal_price = match order_type {
            OrderType::Limit => {
                let p = price.ok_or_else(|| "Limit order requires a price".to_string())?;
                if p.is_zero() {
                    return Err("Order price must be positive".to_string());
                }
                Some(p)
            }
            OrderType::Market => None,
        };

        let dt = Utc::now();

        Ok(Order {
            id,
            user_id,
            symbol,
            side,
            order_type,
            quantity,
            initial_quantity: quantity,
            price: decimal_price,
            timestamp: dt.timestamp_millis(),
            status: OrderStatus::Open,
        })
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

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn side(&self) -> Side {
        self.side
    }

    pub fn order_type(&self) -> OrderType {
        self.order_type
    }

    pub fn price(&self) -> Option<UDecimal> {
        self.price
    }

    pub fn status(&self) -> OrderStatus {
        self.status
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
            "u1".into(),
            "ETH-USDT".into(),
            Side::Buy,
            OrderType::Market,
            UDecimal::new(dec!(10.0)).unwrap(),
            None,
        );
        assert!(buy.is_ok());
        let o = buy.unwrap();
        assert_eq!(o.id(), "o1");
        assert_eq!(o.user_id(), "u1");
        assert_eq!(o.price(), None);

        let buy_with_extraneous_price = Order::new(
            "o1_m".into(),
            "u1".into(),
            "ETH-USDT".into(),
            Side::Buy,
            OrderType::Market,
            UDecimal::new(dec!(10.0)).unwrap(),
            Some(UDecimal::new(dec!(3000.0)).unwrap()),
        );
        assert!(buy_with_extraneous_price.is_ok());
        assert_eq!(buy_with_extraneous_price.unwrap().price(), None);

        let limit_buy = Order::new(
            "o2".into(),
            "u2".into(),
            "ETH-USDT".into(),
            Side::Buy,
            OrderType::Limit,
            UDecimal::new(dec!(10.0)).unwrap(),
            Some(UDecimal::new(dec!(2500.0)).unwrap()),
        );
        assert!(limit_buy.is_ok());

        let limit_no_price = Order::new(
            "o3".into(),
            "u3".into(),
            "ETH-USDT".into(),
            Side::Buy,
            OrderType::Limit,
            UDecimal::new(dec!(10.0)).unwrap(),
            None,
        );
        assert!(limit_no_price.is_err());

        let zero_price = Order::new(
            "o4".into(),
            "u4".into(),
            "ETH-USDT".into(),
            Side::Buy,
            OrderType::Limit,
            UDecimal::new(dec!(10.0)).unwrap(),
            Some(UDecimal::ZERO),
        );
        assert!(zero_price.is_err());

        let zero_qty = Order::new(
            "o5".into(),
            "u5".into(),
            "ETH-USDT".into(),
            Side::Buy,
            OrderType::Market,
            UDecimal::ZERO,
            None,
        );
        assert!(zero_qty.is_err());

        let empty_id = Order::new(
            "".into(),
            "u1".into(),
            "ETH-USDT".into(),
            Side::Buy,
            OrderType::Market,
            UDecimal::new(dec!(1.0)).unwrap(),
            None,
        );
        assert!(empty_id.is_err());

        let empty_user = Order::new(
            "o1".into(),
            "".into(),
            "ETH-USDT".into(),
            Side::Buy,
            OrderType::Market,
            UDecimal::new(dec!(1.0)).unwrap(),
            None,
        );
        assert!(empty_user.is_err());
    }

    #[test]
    pub fn test_order_fill_and_status() {
        let mut order = Order::new(
            "o1".into(),
            "u1".into(),
            "BTC-USDT".into(),
            Side::Buy,
            OrderType::Limit,
            UDecimal::new(dec!(5.0)).unwrap(),
            Some(UDecimal::new(dec!(60000.0)).unwrap()),
        )
        .unwrap();

        order.fill_order(UDecimal::new(dec!(2.0)).unwrap());
        assert_eq!(order.status(), OrderStatus::PartiallyExecuted);
        assert_eq!(order.quantity(), UDecimal::new(dec!(3.0)).unwrap());
        assert_eq!(order.filled_quantity(), UDecimal::new(dec!(2.0)).unwrap());

        order.fill_order(UDecimal::new(dec!(3.0)).unwrap());
        assert_eq!(order.status(), OrderStatus::Executed);
        assert!(order.is_filled());
        assert_eq!(order.filled_quantity(), UDecimal::new(dec!(5.0)).unwrap());
    }

    #[test]
    pub fn test_order_cancellation() {
        let mut order = Order::new(
            "o1".into(),
            "u1".into(),
            "BTC-USDT".into(),
            Side::Buy,
            OrderType::Limit,
            UDecimal::new(dec!(5.0)).unwrap(),
            Some(UDecimal::new(dec!(60000.0)).unwrap()),
        )
        .unwrap();

        assert_eq!(order.status(), OrderStatus::Open);
        assert!(order.cancel().is_ok());
        assert_eq!(order.status(), OrderStatus::Cancelled);
        assert!(order.cancel().is_err());
    }
}
