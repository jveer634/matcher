use chrono::Utc;

use super::udecimal::UDecimal;

#[derive(Debug, Clone, PartialEq)]
pub struct Trade {
    pub id: String,
    pub symbol: String,
    pub maker_order_id: String,
    pub taker_order_id: String,
    pub price: UDecimal,
    pub quantity: UDecimal,
    pub timestamp: i64,
}

impl Trade {
    pub fn new(
        id: String,
        symbol: String,
        maker_order_id: String,
        taker_order_id: String,
        price: UDecimal,
        quantity: UDecimal,
    ) -> Self {
        Trade {
            id,
            symbol,
            maker_order_id,
            taker_order_id,
            price,
            quantity,
            timestamp: Utc::now().timestamp_millis(),
        }
    }

    /// Total traded financial value (quote currency value = price * quantity)
    pub fn notional(&self) -> UDecimal {
        self.price * self.quantity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    pub fn test_trade_notional() {
        let trade = Trade::new(
            "t1".into(),
            "BTC-USDT".into(),
            "m1".into(),
            "t1".into(),
            UDecimal::new(dec!(65000.50)).unwrap(),
            UDecimal::new(dec!(2.5)).unwrap(),
        );
        assert_eq!(trade.notional(), UDecimal::new(dec!(162501.25)).unwrap());
    }
}
