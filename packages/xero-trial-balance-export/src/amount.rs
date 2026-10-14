//! Exact finite decimals for the exporter's precision-60, Inexact-trapping sums.
use crate::{Failure, Result, pytext};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};

#[derive(Clone, Debug)]
pub struct Amount {
    coefficient: BigInt,
    exponent: i64,
    negative_zero: bool,
}

impl Amount {
    pub fn zero() -> Self {
        Self {
            coefficient: BigInt::zero(),
            exponent: 0,
            negative_zero: false,
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        let text = pytext::strip(value);
        if text.is_empty() {
            return Ok(Self::zero());
        }
        let Some((negative, digits, exponent)) = literal_parts(text) else {
            return Err(Failure::message(format!(
                "error: report cell \"{}\" is not an amount. The API shape may have changed, or this is not a trial balance report.",
                shown(text)
            )));
        };
        let adjusted =
            i128::from(exponent) + digits.trim_start_matches('0').len().max(1) as i128 - 1;
        if adjusted > 30 {
            return Err(Failure::message(format!(
                "error: report cell \"{}\" is {} digits long, which is not a ledger balance. The API shape may have changed, or this is not a trial balance report.",
                shown(text),
                adjusted + 1
            )));
        }
        if adjusted < -30 {
            return Err(Failure::message(format!(
                "error: report cell \"{}\" is smaller than any ledger balance. The API shape may have changed, or this is not a trial balance report.",
                shown(text)
            )));
        }
        Ok(from_parts(negative, digits, exponent))
    }

    pub fn add(&self, other: &Self) -> Result<Self> {
        let exponent = self.exponent.min(other.exponent);
        let power = |n: i64| BigInt::from(10).pow(n.try_into().expect("bounded ledger exponent"));
        let coefficient = &self.coefficient * power(self.exponent - exponent)
            + &other.coefficient * power(other.exponent - exponent);
        let mut result = Self {
            negative_zero: self.negative() && other.negative() && coefficient.is_zero(),
            coefficient,
            exponent,
        };
        result.apply_precision()?;
        Ok(result)
    }

    pub fn sub(&self, other: &Self) -> Result<Self> {
        let opposite = Self {
            coefficient: -&other.coefficient,
            exponent: other.exponent,
            negative_zero: !other.negative() && other.coefficient.is_zero(),
        };
        self.add(&opposite)
    }

    fn apply_precision(&mut self) -> Result<()> {
        let digits = self.coefficient.abs().to_string().len();
        if digits <= 60 {
            return Ok(());
        }
        let divisor = BigInt::from(10).pow((digits - 60).try_into().expect("literal length fits"));
        if (&self.coefficient % &divisor).is_zero() {
            self.coefficient /= divisor;
            self.exponent += (digits - 60) as i64;
            Ok(())
        } else {
            Err(Failure::message(
                "Nothing written - a reported amount needs more precision than the balance check can carry exactly, so the totals cannot be trusted. Check the report for a malformed cell.",
            ))
        }
    }

    pub fn is_zero(&self) -> bool {
        self.coefficient.is_zero()
    }
    fn negative(&self) -> bool {
        self.coefficient.is_negative() || self.negative_zero
    }

    pub fn format(&self) -> String {
        let mut digits = self.coefficient.abs().to_string();
        let exponent = if self.is_zero() && self.exponent > 0 {
            0
        } else {
            self.exponent
        };
        if exponent >= 0 {
            digits.extend(std::iter::repeat_n('0', exponent as usize));
            digits.push_str(".0");
        } else {
            let place = digits.len() as i64 + exponent;
            if place <= 0 {
                digits = format!("0.{}{}", "0".repeat((-place) as usize), digits);
            } else {
                digits.insert(place as usize, '.');
            }
            while digits.ends_with('0') {
                digits.pop();
            }
            if digits.ends_with('.') {
                digits.push('0');
            }
        }
        if self.negative() {
            digits.insert(0, '-');
        }
        digits
    }

    pub fn equal(&self, other: &Self) -> bool {
        let exponent = self.exponent.min(other.exponent);
        &self.coefficient * BigInt::from(10).pow((self.exponent - exponent) as u32)
            == &other.coefficient * BigInt::from(10).pow((other.exponent - exponent) as u32)
    }

    pub fn grouped_two(&self) -> String {
        let mut magnitude = self.coefficient.abs();
        let shift = self.exponent + 2;
        if shift >= 0 {
            magnitude *= BigInt::from(10).pow(shift as u32);
        } else {
            let divisor = BigInt::from(10).pow((-shift) as u32);
            let remainder = &magnitude % &divisor;
            magnitude /= &divisor;
            let twice = remainder * 2;
            if twice > divisor || (twice == divisor && (&magnitude % 2u8) != BigInt::zero()) {
                magnitude += 1u8;
            }
        }
        let digits = magnitude.to_string();
        let digits = if digits.len() < 3 {
            format!("{:0>3}", digits)
        } else {
            digits
        };
        let split = digits.len() - 2;
        let mut grouped = String::new();
        for (index, ch) in digits[..split].chars().enumerate() {
            if index > 0 && (split - index) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(ch);
        }
        format!(
            "{}{}.{}",
            if self.negative() { "-" } else { "" },
            grouped,
            &digits[split..]
        )
    }
}

#[cfg(any(test, feature = "probe"))]
pub(crate) fn parse_literal(text: &str) -> Option<Amount> {
    let (negative, digits, exponent) = literal_parts(text)?;
    Some(from_parts(negative, digits, exponent))
}

fn literal_parts(text: &str) -> Option<(bool, String, i64)> {
    let mut ascii = String::new();
    for ch in text.chars().filter(|&ch| ch != '_') {
        if ch.is_ascii() {
            ascii.push(ch);
        } else {
            let n = pytext::decimal_value(ch as u32)?;
            ascii.push(char::from_digit(n, 10)?);
        }
    }
    let (negative, unsigned) = match ascii.as_bytes().first()? {
        b'-' => (true, &ascii[1..]),
        b'+' => (false, &ascii[1..]),
        _ => (false, ascii.as_str()),
    };
    let (mantissa, mut exponent) = match unsigned.find(['e', 'E']) {
        Some(index) => (
            &unsigned[..index],
            unsigned[index + 1..].parse::<i64>().ok()?,
        ),
        None => (unsigned, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    exponent = exponent.checked_sub(fraction.len().try_into().ok()?)?;
    if !(-1_999_999_999_999_999_996..=999_999_999_999_999_999).contains(&exponent) {
        return None;
    }
    Some((negative, digits, exponent))
}

fn from_parts(negative: bool, digits: String, exponent: i64) -> Amount {
    let mut coefficient = digits.parse::<BigInt>().expect("validated decimal digits");
    if negative {
        coefficient = -coefficient;
    }
    let negative_zero = negative && coefficient.is_zero();
    Amount {
        coefficient,
        exponent,
        negative_zero,
    }
}

pub fn shown(text: &str) -> String {
    let mut result = String::new();
    for cp in pytext::code_points(text)
        .take(40)
        .filter(|&cp| char::from_u32(cp).is_some_and(pytext::printable))
    {
        pytext::push_code_point(&mut result, cp);
    }
    result
}

pub type Totals = [Amount; 4];

pub fn add_totals(totals: &Totals, amounts: &Totals) -> Result<Totals> {
    Ok([
        totals[0].add(&amounts[0])?,
        totals[1].add(&amounts[1])?,
        totals[2].add(&amounts[2])?,
        totals[3].add(&amounts[3])?,
    ])
}

pub fn check_balanced(totals: &Totals, account_count: Option<usize>) -> Result<()> {
    let mut output = String::new();
    for (label, debit, credit) in [
        ("movement", &totals[0], &totals[1]),
        ("YTD", &totals[2], &totals[3]),
    ] {
        let difference = debit.sub(credit).map_err(|_| Failure::message(format!("Nothing written - the {label} difference cannot be computed exactly, so the balance check cannot be trusted.")))?;
        if !difference.is_zero() {
            let scope = account_count
                .map(|n| format!(" across {n} accounts"))
                .unwrap_or_default();
            output.push_str(&format!(
                "WARNING: {label} debits != credits{scope} (diff {})\n",
                difference.format()
            ));
        }
    }
    if output.is_empty() {
        Ok(())
    } else {
        output.push_str("Nothing written - report likely truncated or misparsed.\n");
        Err(Failure {
            code: 1,
            stdout: output,
            stderr: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decimal_literals_and_signed_zero() {
        for (input, output) in [
            ("1_200.00", "1200.0"),
            ("-0.000", "-0.0"),
            ("+٠.١", "0.1"),
            (".5", "0.5"),
            ("1e30", "1000000000000000000000000000000.0"),
        ] {
            assert_eq!(Amount::parse(input).unwrap().format(), output);
        }
        for text in [
            "NaN", "Infinity", "1e31", "1e-31", "True", "1,000", "1e", "1.2.3",
        ] {
            assert!(Amount::parse(text).is_err(), "{text}");
        }
    }
    #[test]
    fn sums_exactly_and_refuses_precision_loss() {
        assert_eq!(
            Amount::parse("0.1")
                .unwrap()
                .add(&Amount::parse("0.2").unwrap())
                .unwrap()
                .format(),
            "0.3"
        );
        let large = Amount::parse("1e30").unwrap();
        assert!(large.add(&Amount::parse("1e-30").unwrap()).is_err());
        assert!(!Amount::parse(".004").unwrap().is_zero());
    }
}
