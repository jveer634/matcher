use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct IdGenerator {
    pair_id: String,
    counter: AtomicU64,
}

impl IdGenerator {
    pub fn new(pair_id: String) -> Self {
        IdGenerator {
            pair_id,
            counter: AtomicU64::new(0),
        }
    }

    pub fn generate_order_id(&self) -> String {
        // Increment counter
        let count = self.counter.fetch_add(1, Ordering::SeqCst);

        // Get current timestamp in milliseconds
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis();

        // Format ID as "POOLID-TIMESTAMP-COUNTER"
        format!("{}-{:x}-{:06x}", self.pair_id, timestamp, count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn pass_generating_id() {
        let pair_id = String::from("ETHINC");
        let generator = IdGenerator::new(pair_id.clone());

        let binding = generator.generate_order_id();
        let vec: Vec<&str> = binding.split('-').collect();

        assert_eq!(*vec[0].to_string(), pair_id);
        assert_eq!(vec[2], "000000");

        let binding = generator.generate_order_id();
        let vec: Vec<&str> = binding.split('-').collect();

        assert_eq!(*vec[0].to_string(), pair_id);
        assert_eq!(vec[2], "000001");
    }
}
