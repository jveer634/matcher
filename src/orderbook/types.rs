use std::fmt;
use std::str::FromStr;

/// Order side: Buy (bid) or Sell (ask).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for Side {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_uppercase().as_str() {
            "BUY" => Ok(Side::Buy),
            "SELL" => Ok(Side::Sell),
            _ => Err(format!("Unknown side: '{}'", s)),
        }
    }
}

/// Order execution type: Market or Limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderType {
    Market,
    Limit,
}

impl OrderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
        }
    }
}

impl fmt::Display for OrderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for OrderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_uppercase().as_str() {
            "MARKET" => Ok(OrderType::Market),
            "LIMIT" => Ok(OrderType::Limit),
            _ => Err(format!("Unknown order type: '{}'", s)),
        }
    }
}

/// Order execution status lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderStatus {
    Open,
    Executed,
    Cancelled,
    PartiallyExecuted,
}

impl OrderStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderStatus::Open => "OPEN",
            OrderStatus::Executed => "EXECUTED",
            OrderStatus::Cancelled => "CANCELLED",
            OrderStatus::PartiallyExecuted => "PARTIALLY_EXECUTED",
        }
    }
}

impl fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for OrderStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_uppercase().as_str() {
            "OPEN" => Ok(OrderStatus::Open),
            "EXECUTED" => Ok(OrderStatus::Executed),
            "CANCELLED" => Ok(OrderStatus::Cancelled),
            "PARTIALLY_EXECUTED" | "PARTIALLYEXECUTED" => Ok(OrderStatus::PartiallyExecuted),
            _ => Err(format!("Unknown order status: '{}'", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_side_display_and_parse() {
        assert_eq!(Side::Buy.to_string(), "BUY");
        assert_eq!(Side::Sell.to_string(), "SELL");
        assert_eq!("buy".parse::<Side>().unwrap(), Side::Buy);
        assert_eq!("SELL".parse::<Side>().unwrap(), Side::Sell);
        assert!("invalid".parse::<Side>().is_err());
    }

    #[test]
    pub fn test_order_type_display_and_parse() {
        assert_eq!(OrderType::Limit.to_string(), "LIMIT");
        assert_eq!(OrderType::Market.to_string(), "MARKET");
        assert_eq!("limit".parse::<OrderType>().unwrap(), OrderType::Limit);
        assert_eq!("MARKET".parse::<OrderType>().unwrap(), OrderType::Market);
        assert!("invalid".parse::<OrderType>().is_err());
    }

    #[test]
    pub fn test_order_status_display_and_parse() {
        assert_eq!(OrderStatus::Open.to_string(), "OPEN");
        assert_eq!(OrderStatus::Executed.to_string(), "EXECUTED");
        assert_eq!(OrderStatus::Cancelled.to_string(), "CANCELLED");
        assert_eq!(OrderStatus::PartiallyExecuted.to_string(), "PARTIALLY_EXECUTED");

        assert_eq!("open".parse::<OrderStatus>().unwrap(), OrderStatus::Open);
        assert_eq!("EXECUTED".parse::<OrderStatus>().unwrap(), OrderStatus::Executed);
        assert_eq!("cancelled".parse::<OrderStatus>().unwrap(), OrderStatus::Cancelled);
        assert_eq!("partially_executed".parse::<OrderStatus>().unwrap(), OrderStatus::PartiallyExecuted);
        assert_eq!("PARTIALLYEXECUTED".parse::<OrderStatus>().unwrap(), OrderStatus::PartiallyExecuted);
        assert!("unknown".parse::<OrderStatus>().is_err());
    }
}
