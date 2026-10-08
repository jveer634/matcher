#![allow(clippy::all)]
#![allow(unused_qualifications)]

pub mod order {
    pub mod v1 {
        include!("order.v1.rs");
    }
}

pub mod orderbook {
    pub mod v1 {
        include!("orderbook.v1.rs");
    }
}

pub mod trade {
    pub mod v1 {
        include!("trade.v1.rs");
    }
}

pub mod engine {
    pub mod v1 {
        include!("engine.v1.rs");
    }
}
