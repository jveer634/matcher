mod exchange;
mod orderbook;

use exchange::Matcher;
use orderbook::orderbook::OrderType;

fn main() {
    let mut matcher = Matcher::new();
    let pair = matcher.add_pair(String::from("ETH"), String::from("INC"));

    let pair2 = matcher.add_pair(String::from("ETH"), String::from("USDT"));
    matcher
        .add_order(pair.clone(), OrderType::LimitSell, Some(12.1), 12.3)
        .unwrap();

    matcher
        .add_order(pair2.clone(), OrderType::LimitBuy, Some(12.5), 12.1)
        .unwrap();

    dbg!(matcher);
}
