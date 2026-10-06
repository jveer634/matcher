use crate::gen::order::v1::{
    Order as ProtoOrder, OrderStatus as ProtoOrderStatus, OrderType as ProtoOrderType,
    Side as ProtoSide,
};
use crate::gen::orderbook::v1::{BookLevel as ProtoBookLevel, Stats as ProtoStats};
use crate::gen::trade::v1::Trade as ProtoTrade;
use crate::matcher::MatcherStats;
use crate::orderbook::{BookLevel, Order, OrderStatus, OrderType, Side, Trade, UDecimal};

impl TryFrom<ProtoSide> for Side {
    type Error = String;

    fn try_from(proto_side: ProtoSide) -> Result<Self, Self::Error> {
        match proto_side {
            ProtoSide::Buy => Ok(Side::Buy),
            ProtoSide::Sell => Ok(Side::Sell),
            ProtoSide::Unspecified => Err("Invalid Side: SIDE_UNSPECIFIED".to_string()),
        }
    }
}

impl TryFrom<i32> for Side {
    type Error = String;

    fn try_from(val: i32) -> Result<Self, Self::Error> {
        let proto =
            ProtoSide::try_from(val).map_err(|_| format!("Unknown side integer value: {}", val))?;
        Side::try_from(proto)
    }
}

impl From<Side> for ProtoSide {
    fn from(side: Side) -> Self {
        match side {
            Side::Buy => ProtoSide::Buy,
            Side::Sell => ProtoSide::Sell,
        }
    }
}

impl From<Side> for i32 {
    fn from(side: Side) -> Self {
        ProtoSide::from(side) as i32
    }
}

impl TryFrom<ProtoOrderType> for OrderType {
    type Error = String;

    fn try_from(proto_type: ProtoOrderType) -> Result<Self, Self::Error> {
        match proto_type {
            ProtoOrderType::Limit => Ok(OrderType::Limit),
            ProtoOrderType::Market => Ok(OrderType::Market),
            ProtoOrderType::Unspecified => {
                Err("Invalid OrderType: ORDER_TYPE_UNSPECIFIED".to_string())
            }
        }
    }
}

impl TryFrom<i32> for OrderType {
    type Error = String;

    fn try_from(val: i32) -> Result<Self, Self::Error> {
        let proto = ProtoOrderType::try_from(val)
            .map_err(|_| format!("Unknown order type integer value: {}", val))?;
        OrderType::try_from(proto)
    }
}

impl From<OrderType> for ProtoOrderType {
    fn from(order_type: OrderType) -> Self {
        match order_type {
            OrderType::Limit => ProtoOrderType::Limit,
            OrderType::Market => ProtoOrderType::Market,
        }
    }
}

impl From<OrderType> for i32 {
    fn from(order_type: OrderType) -> Self {
        ProtoOrderType::from(order_type) as i32
    }
}

impl TryFrom<ProtoOrderStatus> for OrderStatus {
    type Error = String;

    fn try_from(proto_status: ProtoOrderStatus) -> Result<Self, Self::Error> {
        match proto_status {
            ProtoOrderStatus::Open => Ok(OrderStatus::Open),
            ProtoOrderStatus::Executed => Ok(OrderStatus::Executed),
            ProtoOrderStatus::Cancelled => Ok(OrderStatus::Cancelled),
            ProtoOrderStatus::PartiallyExecuted => Ok(OrderStatus::PartiallyExecuted),
            ProtoOrderStatus::Unspecified => {
                Err("Invalid OrderStatus: ORDER_STATUS_UNSPECIFIED".to_string())
            }
        }
    }
}

impl TryFrom<i32> for OrderStatus {
    type Error = String;

    fn try_from(val: i32) -> Result<Self, Self::Error> {
        let proto = ProtoOrderStatus::try_from(val)
            .map_err(|_| format!("Unknown order status integer value: {}", val))?;
        OrderStatus::try_from(proto)
    }
}

impl From<OrderStatus> for ProtoOrderStatus {
    fn from(status: OrderStatus) -> Self {
        match status {
            OrderStatus::Open => ProtoOrderStatus::Open,
            OrderStatus::Executed => ProtoOrderStatus::Executed,
            OrderStatus::Cancelled => ProtoOrderStatus::Cancelled,
            OrderStatus::PartiallyExecuted => ProtoOrderStatus::PartiallyExecuted,
        }
    }
}

impl From<OrderStatus> for i32 {
    fn from(status: OrderStatus) -> Self {
        ProtoOrderStatus::from(status) as i32
    }
}

impl TryFrom<ProtoOrder> for Order {
    type Error = String;

    fn try_from(p: ProtoOrder) -> Result<Self, Self::Error> {
        let side = Side::try_from(p.side)?;
        let order_type = OrderType::try_from(p.order_type)?;
        let quantity = UDecimal::try_from(p.quantity)?;
        let price = match p.price {
            Some(price_val) => Some(UDecimal::try_from(price_val)?),
            None => None,
        };

        Order::new(p.id, p.user_id, p.symbol, side, order_type, quantity, price)
    }
}

impl From<&Order> for ProtoOrder {
    fn from(order: &Order) -> Self {
        ProtoOrder {
            id: order.id().to_string(),
            user_id: order.user_id().to_string(),
            symbol: order.symbol().to_string(),
            side: ProtoSide::from(order.side()) as i32,
            order_type: ProtoOrderType::from(order.order_type()) as i32,
            price: order.price().map(|p| p.to_f64()),
            quantity: order.quantity().to_f64(),
            initial_quantity: order.initial_quantity().to_f64(),
            filled_quantity: order.filled_quantity().to_f64(),
            timestamp: order.timestamp(),
            status: ProtoOrderStatus::from(order.status()) as i32,
        }
    }
}

impl From<Order> for ProtoOrder {
    fn from(order: Order) -> Self {
        ProtoOrder::from(&order)
    }
}

impl TryFrom<ProtoTrade> for Trade {
    type Error = String;

    fn try_from(p: ProtoTrade) -> Result<Self, Self::Error> {
        let price = UDecimal::try_from(p.price)?;
        let quantity = UDecimal::try_from(p.quantity)?;

        Ok(Trade {
            id: p.id,
            symbol: p.symbol,
            maker_order_id: String::new(),
            taker_order_id: String::new(),
            price,
            quantity,
            timestamp: p.timestamp,
        })
    }
}

impl From<&Trade> for ProtoTrade {
    fn from(trade: &Trade) -> Self {
        ProtoTrade {
            id: trade.id.clone(),
            symbol: trade.symbol.clone(),
            price: trade.price.to_f64(),
            quantity: trade.quantity.to_f64(),
            timestamp: trade.timestamp,
        }
    }
}

impl From<Trade> for ProtoTrade {
    fn from(trade: Trade) -> Self {
        ProtoTrade::from(&trade)
    }
}

impl TryFrom<ProtoBookLevel> for BookLevel {
    type Error = String;

    fn try_from(p: ProtoBookLevel) -> Result<Self, Self::Error> {
        let price = UDecimal::try_from(p.price)?;
        let quantity = UDecimal::try_from(p.quantity)?;
        Ok(BookLevel::new(price, quantity, p.order_count as usize))
    }
}

impl From<&BookLevel> for ProtoBookLevel {
    fn from(level: &BookLevel) -> Self {
        ProtoBookLevel {
            price: level.price.to_f64(),
            quantity: level.quantity.to_f64(),
            order_count: level.order_count as u64,
        }
    }
}

impl From<BookLevel> for ProtoBookLevel {
    fn from(level: BookLevel) -> Self {
        ProtoBookLevel::from(&level)
    }
}

impl From<&MatcherStats> for ProtoStats {
    fn from(stats: &MatcherStats) -> Self {
        ProtoStats {
            symbol: stats.symbol.clone(),
            last_traded_price: stats.last_traded_price.map(|p| p.to_f64()),
            total_trades_count: stats.total_trades_count as u64,
            total_volume_traded: stats.total_volume_traded.to_f64(),
            active_buy_volume: stats.active_buy_volume.to_f64(),
            active_sell_volume: stats.active_sell_volume.to_f64(),
            best_bid: stats.best_bid.map(|(p, q)| ProtoBookLevel {
                price: p.to_f64(),
                quantity: q.to_f64(),
                order_count: 0,
            }),
            best_ask: stats.best_ask.map(|(p, q)| ProtoBookLevel {
                price: p.to_f64(),
                quantity: q.to_f64(),
                order_count: 0,
            }),
        }
    }
}

impl From<MatcherStats> for ProtoStats {
    fn from(stats: MatcherStats) -> Self {
        ProtoStats::from(&stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_side_conversions() {
        assert_eq!(Side::try_from(ProtoSide::Buy), Ok(Side::Buy));
        assert_eq!(Side::try_from(ProtoSide::Sell), Ok(Side::Sell));
        assert!(Side::try_from(ProtoSide::Unspecified).is_err());

        assert_eq!(ProtoSide::from(Side::Buy), ProtoSide::Buy);
        assert_eq!(ProtoSide::from(Side::Sell), ProtoSide::Sell);
    }

    #[test]
    fn test_order_type_conversions() {
        assert_eq!(
            OrderType::try_from(ProtoOrderType::Limit),
            Ok(OrderType::Limit)
        );
        assert_eq!(
            OrderType::try_from(ProtoOrderType::Market),
            Ok(OrderType::Market)
        );
        assert!(OrderType::try_from(ProtoOrderType::Unspecified).is_err());
    }

    #[test]
    fn test_order_status_conversions() {
        assert_eq!(
            OrderStatus::try_from(ProtoOrderStatus::Open),
            Ok(OrderStatus::Open)
        );
        assert_eq!(
            OrderStatus::try_from(ProtoOrderStatus::Executed),
            Ok(OrderStatus::Executed)
        );
        assert_eq!(
            OrderStatus::try_from(ProtoOrderStatus::Cancelled),
            Ok(OrderStatus::Cancelled)
        );
        assert_eq!(
            OrderStatus::try_from(ProtoOrderStatus::PartiallyExecuted),
            Ok(OrderStatus::PartiallyExecuted)
        );
        assert!(OrderStatus::try_from(ProtoOrderStatus::Unspecified).is_err());
    }

    #[test]
    fn test_order_roundtrip_conversion() {
        let domain_order = Order::new(
            "o-1".to_string(),
            "u-1".to_string(),
            "BTC-USDT".to_string(),
            Side::Buy,
            OrderType::Limit,
            UDecimal::new(dec!(2.5)).unwrap(),
            Some(UDecimal::new(dec!(65000.0)).unwrap()),
        )
        .unwrap();

        let proto_order = ProtoOrder::from(&domain_order);
        assert_eq!(proto_order.id, "o-1");
        assert_eq!(proto_order.price, Some(65000.0));
        assert_eq!(proto_order.quantity, 2.5);

        let roundtrip = Order::try_from(proto_order).unwrap();
        assert_eq!(roundtrip.id(), domain_order.id());
        assert_eq!(roundtrip.side(), domain_order.side());
        assert_eq!(roundtrip.quantity(), domain_order.quantity());
    }
}
