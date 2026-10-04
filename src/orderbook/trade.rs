use chrono::Utc;
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq)]
pub struct Trade {
    pub id: String,
    pub symbol: String,
    pub maker_order_id: String,
    pub taker_order_id: String,
    pub price: Decimal,
    pub quantity: f64,
    pub timestamp: i64,
}

impl Trade {
    pub fn new(
        id: String,
        symbol: String,
        maker_order_id: String,
        taker_order_id: String,
        price: Decimal,
        quantity: f64,
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
}
