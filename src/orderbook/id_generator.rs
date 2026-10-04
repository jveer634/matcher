use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct IdGenerator {
    pair_id: String,
    order_counter: AtomicU64,
    trade_counter: AtomicU64,
}

impl IdGenerator {
    pub fn new(pair_id: String) -> Self {
        IdGenerator {
            pair_id,
            order_counter: AtomicU64::new(0),
            trade_counter: AtomicU64::new(0),
        }
    }

    #[allow(dead_code)]
    pub fn pair_id(&self) -> &str {
        &self.pair_id
    }

    pub fn generate_order_id(&self) -> String {
        let count = self.order_counter.fetch_add(1, Ordering::SeqCst);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis();

        format!("{}-ORD-{:x}-{:06x}", self.pair_id, timestamp, count)
    }

    pub fn generate_trade_id(&self) -> String {
        let count = self.trade_counter.fetch_add(1, Ordering::SeqCst);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis();

        format!("{}-TRD-{:x}-{:06x}", self.pair_id, timestamp, count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_generating_ids() {
        let pair_id = String::from("ETHINC");
        let generator = IdGenerator::new(pair_id.clone());

        let order_id = generator.generate_order_id();
        assert!(order_id.starts_with("ETHINC-ORD-"));

        let trade_id = generator.generate_trade_id();
        assert!(trade_id.starts_with("ETHINC-TRD-"));
    }
}
