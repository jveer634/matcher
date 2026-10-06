use super::udecimal::UDecimal;

/// Aggregated order book price level for depth queries and L2 book snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookLevel {
    pub price: UDecimal,
    pub quantity: UDecimal,
    pub order_count: usize,
}

impl BookLevel {
    pub fn new(price: UDecimal, quantity: UDecimal, order_count: usize) -> Self {
        Self {
            price,
            quantity,
            order_count,
        }
    }
}
