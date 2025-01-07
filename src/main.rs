mod exchange;
mod orderbook;

use exchange::Matcher;
use orderbook::order::OrderType;

fn main() {
    let mut matcher = Matcher::new();
    let pair = matcher.add_pair(String::from("ETH"), String::from("INC"), 200.0);

    let pair2 = matcher.add_pair(String::from("ETH"), String::from("USDT"), 12.0);
    let order1 = matcher
        .add_order(pair.clone(), OrderType::LimitSell, Some(12.1), 12.3)
        .unwrap();

    let order2 = matcher
        .add_order(pair2.clone(), OrderType::LimitBuy, Some(12.0), 12.1)
        .unwrap();

    dbg!("Before Deletion", &matcher, &order2);

    matcher
        .update_order(order1, OrderType::Buy, None, 11.9)
        .unwrap();

    matcher.cancel_order(order2).unwrap();

    dbg!("After deletion", &matcher);
}
