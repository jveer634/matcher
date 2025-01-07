use core::fmt;
use std::collections::HashMap;

use crate::orderbook::{order::OrderType, orderbook::OrderBook};

#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub struct TradingPair {
    id: String,
    base: String,
    quote: String,
    is_active: bool,
}

impl TradingPair {
    pub fn new(base: String, quote: String) -> TradingPair {
        let id = format!("{base}{quote}");
        TradingPair {
            id,
            base,
            quote,
            is_active: true,
        }
    }
}

impl fmt::Display for TradingPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PoolId: {}, Base: {}, Quote: {}",
            self.id, self.base, self.quote
        )
    }
}

#[derive(Debug)]
pub struct Matcher {
    pub pairs: HashMap<String, OrderBook>,
}

impl Matcher {
    pub fn new() -> Matcher {
        Matcher {
            pairs: HashMap::new(),
        }
    }

    pub fn add_pair(&mut self, base: String, quote: String) -> String {
        let pair = TradingPair::new(base, quote);
        match self.pairs.get(&pair.id) {
            Some(order_book) => {
                println!("OrderBook for pair {pair} already exists: {:?}", order_book);
            }
            None => {
                let order_book = OrderBook::new(pair.id.clone());
                self.pairs.insert(pair.id.clone(), order_book);
                println!("Added new pair: {:?}", pair);
            }
        }

        pair.id
    }

    pub fn add_order(
        &mut self,
        pair_id: String,
        order_type: OrderType,
        price: Option<f64>,
        quantity: f64,
    ) -> Result<String, String> {
        match self.pairs.get_mut(&pair_id) {
            Some(book) => {
                return book.add_order(order_type, price, quantity);
            }
            None => {
                return Err("Invalid PoolId".to_owned());
            }
        }
    }

    pub fn cancel_order(&mut self, order_id: String) -> Result<(), String> {
        let pair_id = order_id.split('-').next().unwrap().to_string();

        match self.pairs.get_mut(&pair_id) {
            Some(book) => book.cancel_order(order_id),
            None => Err("Invalid Order Id".to_string()),
        }
    }
}
