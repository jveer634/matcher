use core::fmt;
use std::{collections::HashMap, fmt::format};

use crate::orderbook::{order::OrderType, orderbook::OrderBook};

#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub struct TradingPair {
    id: String,
    base: String,
    quote: String,
    is_active: bool,
    listing_price: String,
}

impl TradingPair {
    pub fn new(base: String, quote: String, listing_price: f64) -> TradingPair {
        let id = format!("{base}{quote}");
        TradingPair {
            id,
            base,
            quote,
            is_active: true,
            listing_price: listing_price.to_string(),
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
    pub books: HashMap<String, OrderBook>,
    pub pairs: HashMap<String, TradingPair>,
}

impl Matcher {
    pub fn new() -> Matcher {
        Matcher {
            books: HashMap::new(),
            pairs: HashMap::new(),
        }
    }

    pub fn add_pair(
        &mut self,
        base: String,
        quote: String,
        listing_price: f64,
    ) -> Result<String, String> {
        let pair = TradingPair::new(base, quote, listing_price);
        let id = pair.id.clone();
        match self.books.get(&id) {
            Some(_) => Err(format!("Pair already exits ")),
            None => {
                let order_book = OrderBook::new(id.clone(), listing_price);
                self.books.insert(id.clone(), order_book);

                self.pairs.insert(id.clone(), pair);

                println!("Added new pair: {:?}", id.clone());
                Ok(id)
            }
        }
    }

    pub fn get_pair(&self, pair_id: String) -> Result<&TradingPair, String> {
        match self.pairs.get(&pair_id) {
            Some(pair) => return Ok(&pair),
            None => Err("Invalid pair id".to_string()),
        }
    }

    pub fn update_pool(&mut self, pair_id: String, enable: bool) -> Result<(), String> {
        match self.pairs.get_mut(&pair_id) {
            Some(pair) => {
                pair.is_active = enable;
                Ok(())
            }
            None => Err("Invalid pair id".to_string()),
        }
    }

    pub fn add_order(
        &mut self,
        pair_id: String,
        order_type: OrderType,
        price: Option<f64>,
        quantity: f64,
    ) -> Result<String, String> {
        match self.books.get_mut(&pair_id) {
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

        match self.books.get_mut(&pair_id) {
            Some(book) => book.cancel_order(order_id),
            None => Err("Invalid Order Id".to_string()),
        }
    }

    pub fn update_order(
        &mut self,
        order_id: String,
        order_type: OrderType,
        price: Option<f64>,
        quantity: f64,
    ) -> Result<(), String> {
        let pair_id = order_id.split('-').next().unwrap().to_string();

        match self.books.get_mut(&pair_id) {
            Some(book) => book.update_order(order_id, quantity, order_type, price),
            None => Err("Invalid Order Id".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn pass_add_pair() {
        let mut matcher = Matcher::new();

        let base = String::from("ETH");
        let quote = String::from("INC");

        let list_price = 1122345.0;

        let pair_id = matcher
            .add_pair(base.clone(), quote.clone(), list_price)
            .expect("Adding pair failed");

        let pair = matcher.get_pair(pair_id).expect("Can't find pair");

        assert_eq!(pair.base, base);
        assert_eq!(pair.quote, quote);
        assert_eq!(pair.listing_price, list_price.to_string());
    }
}
