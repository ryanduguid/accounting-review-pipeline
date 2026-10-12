//! Money as the Python script held it: `Decimal` values with at most 2
//! places, added and subtracted exactly. Python's sums keep the smallest
//! exponent of their operands, so `str()` writes "80.00" for 41.25 + 38.75 and
//! "1000" for 600 + 400; this type keeps that exponent.

use std::cmp::Ordering;
use std::ops::{Add, Sub};

#[derive(Clone, Copy, Debug)]
pub struct Amount {
    /// The value times 10**-exp.
    coeff: i128,
    /// 0, -1 or -2.
    exp: i8,
}

impl Amount {
    pub const ZERO: Amount = Amount { coeff: 0, exp: 0 };

    /// `Decimal(text)` for text that `money()` already matched; digits may be any Unicode decimal digit.
    pub fn from_digits(int_digits: &[u32], frac_digits: &[u32]) -> Amount {
        let coeff = int_digits
            .iter()
            .chain(frac_digits)
            .fold(0i128, |acc, d| acc * 10 + *d as i128);
        Amount {
            coeff,
            exp: -(frac_digits.len() as i8),
        }
    }

    fn align(self, other: Amount) -> (i128, i128, i8) {
        let exp = self.exp.min(other.exp);
        let scale = |a: Amount| a.coeff * 10i128.pow((a.exp - exp) as u32);
        (scale(self), scale(other), exp)
    }

    pub fn is_zero(self) -> bool {
        self.coeff == 0
    }

    pub fn is_negative(self) -> bool {
        self.coeff < 0
    }
}

impl PartialEq for Amount {
    fn eq(&self, other: &Amount) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Amount {}

impl PartialOrd for Amount {
    fn partial_cmp(&self, other: &Amount) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Amount {
    fn cmp(&self, other: &Amount) -> Ordering {
        let (a, b, _) = self.align(*other);
        a.cmp(&b)
    }
}

impl Add for Amount {
    type Output = Amount;

    fn add(self, other: Amount) -> Amount {
        let (a, b, exp) = self.align(other);
        Amount { coeff: a + b, exp }
    }
}

impl Sub for Amount {
    type Output = Amount;

    fn sub(self, other: Amount) -> Amount {
        let (a, b, exp) = self.align(other);
        Amount { coeff: a - b, exp }
    }
}

impl std::fmt::Display for Amount {
    /// `str(Decimal)`: plain notation for these exponents.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sign = if self.coeff < 0 { "-" } else { "" };
        let digits = self.coeff.unsigned_abs().to_string();
        let places = (-self.exp) as usize;
        if places == 0 {
            return write!(f, "{sign}{digits}");
        }
        let padded = format!("{digits:0>width$}", width = places + 1);
        let (int, frac) = padded.split_at(padded.len() - places);
        write!(f, "{sign}{int}.{frac}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(text: &str) -> Amount {
        let (int, frac) = text.split_once('.').unwrap_or((text, ""));
        let digits = |s: &str| s.chars().map(|c| c.to_digit(10).unwrap()).collect::<Vec<_>>();
        Amount::from_digits(&digits(int), &digits(frac))
    }

    #[test]
    fn sums_keep_the_smallest_exponent_like_decimal() {
        assert_eq!(a("41.25").add(a("38.75")).to_string(), "80.00");
        assert_eq!(a("600").add(a("400")).to_string(), "1000");
        assert_eq!(Amount::ZERO.add(a("12.5")).to_string(), "12.5");
        assert_eq!(a("2319.99").sub(a("2320")).to_string(), "-0.01");
        assert_eq!(a("1.00").sub(a("1.00")).to_string(), "0.00");
        assert_eq!(a("007").to_string(), "7");
        assert_eq!(a("0.05").to_string(), "0.05");
        assert!(a("3599.99") < a("3600"));
    }
}
