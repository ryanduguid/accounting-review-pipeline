//! Signed four-place amounts; scale and negative zero follow Decimal.
use crate::transactions::text::{digit, space};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};

#[derive(Clone, Debug)]
pub struct Amount {
    coefficient: BigInt,
    places: u32,
    negative: bool,
}

impl Amount {
    pub fn zero() -> Self {
        Self {
            coefficient: BigInt::zero(),
            places: 0,
            negative: false,
        }
    }

    fn aligned(&self, places: u32) -> BigInt {
        &self.coefficient * BigInt::from(10).pow(places - self.places)
    }

    pub fn add(&self, other: &Self) -> Self {
        let places = self.places.max(other.places);
        let coefficient = self.aligned(places) + other.aligned(places);
        let negative =
            coefficient.is_negative() || (coefficient.is_zero() && self.negative && other.negative);
        Self {
            coefficient,
            places,
            negative,
        }
    }

    pub fn sub(&self, other: &Self) -> Self {
        let opposite = Self {
            coefficient: -&other.coefficient,
            places: other.places,
            negative: !other.negative,
        };
        self.add(&opposite)
    }

    pub fn agrees(&self) -> bool {
        self.aligned(4).abs() < BigInt::from(50)
    }
}

impl std::fmt::Display for Amount {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let places = self.places as usize;
        let digits = self.coefficient.abs().to_string();
        let digits = format!("{digits:0>width$}", width = places + 1);
        let sign = if self.negative { "-" } else { "" };
        if places == 0 {
            return write!(out, "{sign}{digits}");
        }
        let (whole, fraction) = digits.split_at(digits.len() - places);
        write!(out, "{sign}{whole}.{fraction}")
    }
}

pub fn parse(text: &str) -> Result<Amount, String> {
    let text = text.trim_matches(space);
    if text.is_empty() {
        return Ok(Amount::zero());
    }
    let negative = text.starts_with('-');
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let groups: Vec<_> = whole.split(',').collect();
    let sizes: Vec<_> = groups.iter().map(|s| s.chars().count()).collect();
    let valid_whole = if groups.len() == 1 {
        (1..=15).contains(&sizes[0])
    } else {
        (2..=5).contains(&groups.len())
            && (1..=3).contains(&sizes[0])
            && sizes[1..].iter().all(|&n| n == 3)
    };
    let places = fraction.chars().count();
    let invalid = || "Expected a finite signed amount with at most 4 decimal places.".to_owned();
    if !valid_whole || places > 4 || (unsigned.contains('.') && places == 0) {
        return Err(invalid());
    }
    let digits = whole
        .chars()
        .filter(|&ch| ch != ',')
        .chain(fraction.chars())
        .map(digit)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(invalid)?;
    let mut coefficient = digits.iter().fold(BigInt::zero(), |n, &d| n * 10 + d);
    if negative {
        coefficient = -coefficient;
    }
    Ok(Amount {
        coefficient,
        places: places as u32,
        negative,
    })
}
