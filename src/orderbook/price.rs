pub const SCALAR: f64 = 10000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Price {
    integral: u64,
    fractional: u64,
}

impl Price {
    pub fn new(price: f64) -> Price {
        Price {
            integral: price as u64,
            fractional: ((price % 1.0) * SCALAR) as u64,
        }
    }

    pub fn to_f64(&self) -> f64 {
        (self.integral) as f64 + (self.fractional as f64 / SCALAR)
    }
}