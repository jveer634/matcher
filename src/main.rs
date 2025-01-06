mod orderbook;

use orderbook::{OrderBook, OrderType};

fn main() {
    let mut book = OrderBook::new();
    book.add_order(OrderType::LimitSell, Some(12.1), 12.3);
    book.add_order(OrderType::Buy, None, 12.3);

    book.add_order(OrderType::Sell, Some(13.9), 12.3);
    book.add_order(OrderType::Buy, Some(12.5), 1.0);
    dbg!(&book);
}
