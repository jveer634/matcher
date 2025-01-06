mod exchange;
mod orderbook;

use exchange::Matcher;
use orderbook::OrderType;

fn main() {
    let mut matcher = Matcher::new();
    let pair = matcher.add_pair(String::from("ETH"), String::from("INC"));

    matcher
        .add_order(pair.clone(), OrderType::LimitSell, Some(12.1), 12.3)
        .unwrap();

    matcher
        .add_order("ETHINC".to_owned(), OrderType::Buy, None, 12.1)
        .unwrap();

    dbg!(matcher);
}
