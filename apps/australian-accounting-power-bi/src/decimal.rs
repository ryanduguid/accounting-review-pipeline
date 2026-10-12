//! The Python `decimal` arithmetic the fixture generator relies on, under the default
//! context: 28 significant digits, ROUND_HALF_EVEN.
//!
//! Every operation returns the correctly rounded value of the exact operation, with the
//! sign CPython gives it, zeros included, so a negative amount that rounds away still
//! formats as `-0.00`. The exponent CPython would keep is not modelled: the generator
//! formats every amount at fixed places, and equal values format identically there.
//! Special values never occur. tests/differential/decimal_differential.py compares the
//! results with CPython.

use std::cmp::Ordering;

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::Zero;

/// Significant digits in Python's default decimal context.
pub const PRECISION: u32 = 28;

/// A finite decimal value: `(-1)^negative * magnitude * 10^exponent`.
#[derive(Clone, Debug)]
pub struct Decimal {
    negative: bool,
    magnitude: BigInt,
    exponent: i64,
}

fn pow10(n: u64) -> BigInt {
    num_traits::pow(BigInt::from(10), usize::try_from(n).expect("exponent fits"))
}

fn digits(magnitude: &BigInt) -> u64 {
    magnitude.magnitude().to_str_radix(10).len() as u64
}

/// `magnitude / 10^drop`, rounded half to even. `sticky` marks a nonzero remainder
/// below the dropped digits, as an inexact quotient leaves.
fn round_half_even(magnitude: &BigInt, drop: u64, sticky: bool) -> BigInt {
    if drop == 0 {
        assert!(!sticky, "an inexact value needs digits to round");
        return magnitude.clone();
    }
    let (quotient, remainder) = magnitude.div_rem(&pow10(drop));
    let half = pow10(drop - 1) * 5;
    let up = match remainder.cmp(&half) {
        Ordering::Greater => true,
        Ordering::Equal => sticky || quotient.is_odd(),
        Ordering::Less => false,
    };
    if up { quotient + 1 } else { quotient }
}

impl Decimal {
    /// `Decimal(text)` for finite values. Like CPython's constructor it is exact, strips
    /// surrounding whitespace, drops every underscore and reads an optional sign, digits
    /// with an optional point, and an optional exponent. Infinities, NaNs, non-ASCII
    /// digits and exponents beyond 10,000 are outside this module and panic.
    pub fn parse(text: &str) -> Decimal {
        let refuse = || -> ! { panic!("InvalidOperation: not a supported decimal literal: {text:?}") };
        let python_space = |c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c);
        let cleaned: String = text.trim_matches(python_space).chars().filter(|&c| c != '_').collect();
        let (negative, unsigned) = match cleaned.as_bytes().first() {
            Some(b'-') => (true, &cleaned[1..]),
            Some(b'+') => (false, &cleaned[1..]),
            _ => (false, cleaned.as_str()),
        };
        let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
            Some(at) => (
                &unsigned[..at],
                parse_exponent(&unsigned[at + 1..]).unwrap_or_else(|| refuse()),
            ),
            None => (unsigned, 0),
        };
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let digits = format!("{whole}{fraction}");
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            refuse();
        }
        let exponent = exponent - i64::try_from(fraction.len()).expect("short literal");
        Decimal {
            negative,
            magnitude: digits.parse().expect("digits"),
            exponent,
        }
    }

    pub fn from_int(value: i64) -> Decimal {
        Decimal {
            negative: value < 0,
            magnitude: BigInt::from(value.unsigned_abs()),
            exponent: 0,
        }
    }

    pub fn zero() -> Decimal {
        Decimal::from_int(0)
    }

    pub fn is_zero(&self) -> bool {
        self.magnitude.is_zero()
    }

    fn signed(&self) -> BigInt {
        if self.negative {
            -&self.magnitude
        } else {
            self.magnitude.clone()
        }
    }

    /// The exact value rounded to the context precision, keeping its sign.
    fn rounded(negative: bool, magnitude: BigInt, exponent: i64, sticky: bool) -> Decimal {
        let count = digits(&magnitude);
        if count <= u64::from(PRECISION) && !sticky {
            return Decimal {
                negative,
                magnitude,
                exponent,
            };
        }
        let mut drop = count.saturating_sub(u64::from(PRECISION));
        let mut magnitude = round_half_even(&magnitude, drop, sticky);
        if digits(&magnitude) > u64::from(PRECISION) {
            // Rounding carried into a new digit: 10^28 becomes 10^27 one place higher.
            magnitude /= 10;
            drop += 1;
        }
        Decimal {
            negative,
            magnitude,
            exponent: exponent + i64::try_from(drop).expect("small shift"),
        }
    }

    /// Both signed coefficients scaled to the smaller exponent.
    fn aligned(&self, other: &Decimal) -> (BigInt, BigInt, i64) {
        let exponent = self.exponent.min(other.exponent);
        let scale = |d: &Decimal| d.signed() * pow10((d.exponent - exponent) as u64);
        (scale(self), scale(other), exponent)
    }

    /// `self + other`. An exact zero sum is negative only when both operands are, as
    /// CPython gives it outside ROUND_FLOOR.
    pub fn add(&self, other: &Decimal) -> Decimal {
        let (a, b, exponent) = self.aligned(other);
        let sum = a + b;
        let negative = if sum.is_zero() {
            self.negative && other.negative
        } else {
            sum < BigInt::zero()
        };
        Decimal::rounded(negative, sum.magnitude().clone().into(), exponent, false)
    }

    /// `self - other`: the exact difference, rounded once, as `__add__` of `copy_negate()`.
    pub fn sub(&self, other: &Decimal) -> Decimal {
        self.add(&Decimal {
            negative: !other.negative,
            ..other.clone()
        })
    }

    pub fn mul(&self, other: &Decimal) -> Decimal {
        let magnitude = &self.magnitude * &other.magnitude;
        Decimal::rounded(
            self.negative != other.negative,
            magnitude,
            self.exponent + other.exponent,
            false,
        )
    }

    /// True division, correctly rounded: the quotient is taken to at least one digit
    /// beyond the precision and any remainder is kept as a sticky bit.
    pub fn div(&self, other: &Decimal) -> Decimal {
        assert!(!other.is_zero(), "division by zero");
        let negative = self.negative != other.negative;
        if self.is_zero() {
            return Decimal {
                negative,
                magnitude: BigInt::zero(),
                exponent: 0,
            };
        }
        let shift = i64::from(PRECISION) + 1 + digits(&other.magnitude) as i64 - digits(&self.magnitude) as i64;
        let (numerator, denominator) = if shift >= 0 {
            (&self.magnitude * pow10(shift as u64), other.magnitude.clone())
        } else {
            (self.magnitude.clone(), &other.magnitude * pow10(shift.unsigned_abs()))
        };
        let (quotient, remainder) = numerator.div_rem(&denominator);
        let exponent = self.exponent - other.exponent - shift;
        Decimal::rounded(negative, quotient, exponent, !remainder.is_zero())
    }

    /// Unary minus, rounded to the context like CPython's `__neg__`; a zero becomes positive.
    pub fn neg(&self) -> Decimal {
        let negative = !self.negative && !self.is_zero();
        Decimal::rounded(negative, self.magnitude.clone(), self.exponent, false)
    }

    /// `value` at exactly `places` decimals, half to even, keeping the sign.
    fn rescale(&self, places: u32) -> Decimal {
        let target = -i64::from(places);
        let magnitude = if self.exponent >= target {
            &self.magnitude * pow10((self.exponent - target) as u64)
        } else {
            round_half_even(&self.magnitude, (target - self.exponent) as u64, false)
        };
        Decimal {
            negative: self.negative,
            magnitude,
            exponent: target,
        }
    }

    /// `round(value, places)`: quantize to `places` decimals, half to even, keeping the
    /// sign even when the result is zero. Like quantize it refuses a result longer than
    /// the context precision.
    pub fn round(&self, places: u32) -> Decimal {
        let result = self.rescale(places);
        assert!(
            digits(&result.magnitude) <= u64::from(PRECISION),
            "InvalidOperation: quantize result exceeds the context precision"
        );
        result
    }

    /// `format(value, f".{places}f")`, sign included for a negative zero. Formatting has
    /// no precision limit.
    pub fn to_fixed(&self, places: u32) -> String {
        let text = self.rescale(places).magnitude.to_str_radix(10);
        let width = places as usize + 1;
        let padded = format!("{text:0>width$}");
        let (whole, fraction) = padded.split_at(padded.len() - places as usize);
        let sign = if self.negative { "-" } else { "" };
        if places == 0 {
            format!("{sign}{whole}")
        } else {
            format!("{sign}{whole}.{fraction}")
        }
    }
}

/// A decimal exponent: optional sign, then digits; `None` if malformed or beyond 10,000.
fn parse_exponent(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let significant = digits.trim_start_matches('0');
    if significant.len() > 5 {
        return None;
    }
    let value: i64 = if significant.is_empty() {
        0
    } else {
        significant.parse().ok()?
    };
    (value <= 10_000).then_some(if text.starts_with('-') { -value } else { value })
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Decimal) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Decimal) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Numeric order; a negative zero equals zero.
impl Ord for Decimal {
    fn cmp(&self, other: &Decimal) -> Ordering {
        let (a, b, _) = self.aligned(other);
        a.cmp(&b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        Decimal::parse(text)
    }

    #[test]
    fn rounds_half_to_even_at_two_places() {
        for (value, expected) in [
            ("2.345", "2.34"),
            ("2.355", "2.36"),
            ("-2.345", "-2.34"),
            ("0.005", "0.00"),
            ("1.0", "1.00"),
        ] {
            assert_eq!(d(value).to_fixed(2), expected, "{value}");
        }
        assert_eq!(d("350000.0").to_fixed(2), "350000.00");
    }

    #[test]
    fn signed_zeros_follow_cpython() {
        assert_eq!(d("-0.001").round(2).to_fixed(2), "-0.00");
        assert_eq!(d("-0.001").to_fixed(2), "-0.00");
        assert_eq!(d("-5").add(&d("5")).to_fixed(1), "0.0");
        assert_eq!(d("-0.0").add(&d("-0")).to_fixed(1), "-0.0");
        assert_eq!(d("-0.0").add(&d("0")).to_fixed(1), "0.0");
        assert_eq!(d("-3").mul(&d("0")).to_fixed(1), "-0.0");
        assert_eq!(d("0").div(&d("-7")).to_fixed(1), "-0.0");
        assert_eq!(d("-0.0").neg().to_fixed(1), "0.0");
        assert_eq!(d("0.0").neg().to_fixed(1), "0.0");
        assert_eq!(d("1").sub(&d("1")).to_fixed(1), "0.0");
        assert!(d("-0.0") == Decimal::zero());
    }

    #[test]
    fn division_rounds_to_twenty_eight_digits() {
        assert_eq!(
            d("0.1143").div(&Decimal::from_int(365)).to_fixed(30),
            "0.000313150684931506849315068493"
        );
        assert_eq!(d("2").div(&d("3")).to_fixed(28), "0.6666666666666666666666666667");
        assert_eq!(
            d("145000.0").div(&Decimal::from_int(26)).round(2).to_fixed(2),
            "5576.92"
        );
        assert_eq!(d("-1").div(&d("8")), d("-0.125"));
    }

    #[test]
    fn addition_and_multiplication_round_after_the_exact_result() {
        let big = d("9999999999999999999999999999");
        assert_eq!(big.add(&d("1")).to_fixed(0), "10000000000000000000000000000");
        assert_eq!(big.add(&d("0.5")).to_fixed(0), "10000000000000000000000000000");
        let near_one = d("1.000000000000000000000000001");
        assert_eq!(near_one.mul(&near_one).to_fixed(27), "1.000000000000000000000000002");
        assert_eq!(d("1.015").mul(&d("160000.0")), d("162400"));
        assert!(d("0.0") == Decimal::zero() && d("1.10") == d("1.1") && d("1.1") < d("1.11"));
    }
}
