use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Deref, Div, Mul, Sub, SubAssign};
use std::str::FromStr;

use rust_decimal::Decimal;

/// An unsigned, non-negative financial `Decimal` wrapper (value >= 0).
/// Guarantees that prices, quantities, and volumes can never be negative (< 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct UDecimal(Decimal);

impl UDecimal {
    pub const ZERO: UDecimal = UDecimal(Decimal::ZERO);
    pub const ONE: UDecimal = UDecimal(Decimal::ONE);

    /// Creates a new `UDecimal` from a `Decimal`, returning an error if negative (< 0).
    pub fn new(val: Decimal) -> Result<Self, String> {
        if val < Decimal::ZERO {
            Err(format!("Expected non-negative value (>= 0), got {}", val))
        } else {
            Ok(UDecimal(val))
        }
    }

    /// Creates a new `UDecimal` that must be strictly positive (> 0).
    pub fn new_strictly_positive(val: Decimal) -> Result<Self, String> {
        if val <= Decimal::ZERO {
            Err(format!("Expected strictly positive value (> 0), got {}", val))
        } else {
            Ok(UDecimal(val))
        }
    }

    /// Returns the underlying `Decimal` value.
    #[inline]
    pub fn get(&self) -> Decimal {
        self.0
    }

    /// Consumes self and returns the underlying `Decimal`.
    #[inline]
    pub fn into_inner(self) -> Decimal {
        self.0
    }

    /// Checks if the value is zero (== 0).
    #[inline]
    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    /// Checks if the value is strictly positive (> 0).
    #[inline]
    pub fn is_positive(&self) -> bool {
        self.0 > Decimal::ZERO
    }

    /// Saturating subtraction: `(self - other).max(0)`
    #[inline]
    pub fn saturating_sub(self, other: UDecimal) -> Self {
        if self.0 <= other.0 {
            UDecimal::ZERO
        } else {
            UDecimal(self.0 - other.0)
        }
    }

    /// Minimum of two `UDecimal` values.
    #[inline]
    pub fn min(self, other: UDecimal) -> Self {
        UDecimal(self.0.min(other.0))
    }

    /// Maximum of two `UDecimal` values.
    #[inline]
    pub fn max(self, other: UDecimal) -> Self {
        UDecimal(self.0.max(other.0))
    }
}

impl Deref for UDecimal {
    type Target = Decimal;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for UDecimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for UDecimal {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let dec = Decimal::from_str(s).map_err(|e| e.to_string())?;
        UDecimal::new(dec)
    }
}

impl From<UDecimal> for Decimal {
    #[inline]
    fn from(u: UDecimal) -> Self {
        u.0
    }
}

impl TryFrom<Decimal> for UDecimal {
    type Error = String;

    fn try_from(val: Decimal) -> Result<Self, Self::Error> {
        UDecimal::new(val)
    }
}

impl TryFrom<f64> for UDecimal {
    type Error = String;

    fn try_from(val: f64) -> Result<Self, Self::Error> {
        use rust_decimal::prelude::FromPrimitive;
        let dec = Decimal::from_f64(val).ok_or_else(|| "Invalid float for Decimal".to_string())?;
        UDecimal::new(dec)
    }
}

impl Add for UDecimal {
    type Output = UDecimal;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        UDecimal(self.0 + rhs.0)
    }
}

impl AddAssign for UDecimal {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for UDecimal {
    type Output = UDecimal;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        self.saturating_sub(rhs)
    }
}

impl SubAssign for UDecimal {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        *self = self.saturating_sub(rhs);
    }
}

impl Mul for UDecimal {
    type Output = UDecimal;

    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        UDecimal(self.0 * rhs.0)
    }
}

impl Div for UDecimal {
    type Output = UDecimal;

    #[inline]
    fn div(self, rhs: Self) -> Self::Output {
        UDecimal(self.0 / rhs.0)
    }
}

impl Sum for UDecimal {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(UDecimal::ZERO, |acc, x| acc + x)
    }
}

impl<'a> Sum<&'a UDecimal> for UDecimal {
    fn sum<I: Iterator<Item = &'a UDecimal>>(iter: I) -> Self {
        iter.fold(UDecimal::ZERO, |acc, x| acc + *x)
    }
}

impl PartialEq<Decimal> for UDecimal {
    #[inline]
    fn eq(&self, other: &Decimal) -> bool {
        self.0 == *other
    }
}

impl PartialOrd<Decimal> for UDecimal {
    #[inline]
    fn partial_cmp(&self, other: &Decimal) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(other)
    }
}

#[macro_export]
macro_rules! udec {
    ($val:expr) => {{
        use rust_decimal_macros::dec;
        $crate::UDecimal::new(dec!($val)).expect("Static udec! failed validation")
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_udecimal_creation() {
        assert!(UDecimal::new(dec!(10.5)).is_ok());
        assert!(UDecimal::new(dec!(0.0)).is_ok());
        assert!(UDecimal::new(dec!(-1.0)).is_err());

        assert!(UDecimal::new_strictly_positive(dec!(1.0)).is_ok());
        assert!(UDecimal::new_strictly_positive(dec!(0.0)).is_err());
        assert!(UDecimal::new_strictly_positive(dec!(-0.01)).is_err());
    }

    #[test]
    fn test_arithmetic_operations() {
        let a = UDecimal::new(dec!(10.0)).unwrap();
        let b = UDecimal::new(dec!(3.0)).unwrap();

        assert_eq!(a + b, UDecimal::new(dec!(13.0)).unwrap());
        assert_eq!(a - b, UDecimal::new(dec!(7.0)).unwrap());
        assert_eq!(b - a, UDecimal::ZERO); // saturating subtraction
        assert_eq!(a * b, UDecimal::new(dec!(30.0)).unwrap());
        assert_eq!(a.min(b), b);
        assert_eq!(a.max(b), a);
    }
}
