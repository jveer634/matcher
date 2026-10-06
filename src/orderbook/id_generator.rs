use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Lock-free unique ID generator for matching engine events.
///
/// # Customizing the ID Schema
/// While order IDs are provided directly by external clients/callers in incoming order requests,
/// the matching engine generates internal unique Trade IDs upon trade executions.
///
/// To adapt this generator to your infrastructure schema (e.g., Twitter Snowflake, UUIDv7,
/// ULID, nanosecond timestamps, or exchange-specific hex formats), you can modify
/// the [`IdGenerator::generate_id`] or [`IdGenerator::generate_trade_id`] functions below.
#[derive(Debug)]
pub struct IdGenerator {
    pair_id: String,
    counter: AtomicU64,
}

impl IdGenerator {
    /// Creates a new [`IdGenerator`] instance scoped to a specific trading pair symbol.
    pub fn new(pair_id: String) -> Self {
        IdGenerator {
            pair_id,
            counter: AtomicU64::new(0),
        }
    }

    /// Returns the symbol/pair identifier associated with this generator.
    pub fn pair_id(&self) -> &str {
        &self.pair_id
    }

    /// Generates a unique, monotonically increasing identifier with a custom entity prefix.
    ///
    /// # Default Format
    /// `{PAIR_ID}-TRD-{TIMESTAMP_HEX}-{COUNTER_HEX}`
    /// e.g. `BTC-USDT-TRD-199b0c2a8e1-000001`
    ///
    /// # Customization Point
    /// Replace the formatting logic inside this function if your exchange gateway uses
    /// a custom binary encoding, UUIDv7, Snowflake, or database sequence ID schema.
    pub fn generate_id(&self) -> String {
        let count = self.counter.fetch_add(1, Ordering::SeqCst);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        format!("{}-TRD-{:x}-{:06x}", self.pair_id, timestamp, count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_generating_trade_ids() {
        let pair_id = String::from("ETH-USDT");
        let generator = IdGenerator::new(pair_id.clone());

        let trade_id1 = generator.generate_id();
        let trade_id2 = generator.generate_id();

        assert!(trade_id1.starts_with("ETH-USDT-TRD-"));
        assert!(trade_id2.starts_with("ETH-USDT-TRD-"));
        assert_ne!(trade_id1, trade_id2);
    }
}
