//! The subset of Python's `decimal` module the gateway uses.
//!
//! Parsing follows CPython's `Decimal(str)` (the C `_decimal` module): exact,
//! with surrounding Python whitespace, underscores and any Unicode decimal
//! digit accepted. Arithmetic ports CPython's reference `_pydecimal`
//! algorithms (`__add__`, `__mul__`, `__truediv__`, `quantize`, `_fix`,
//! `_rescale`) for a context of 28 digits, Emax 999999, Emin -999999 and
//! clamp 0. Close review uses 28 digits, reconciliation 40 and schedules 80.

use std::cmp::Ordering;

use num_bigint::BigUint;
use num_integer::Integer;
use num_traits::Zero;

use crate::pytext;

thread_local! {
    static PRECISION: std::cell::Cell<i128> = const { std::cell::Cell::new(28) };
}

fn precision() -> i128 {
    PRECISION.get()
}

fn etiny() -> i128 {
    EMIN - precision() + 1
}

fn etop() -> i128 {
    EMAX - precision() + 1
}

/// Isolate a command's arithmetic from callers and restore nested contexts.
pub fn with_precision<T>(digits: i128, action: impl FnOnce() -> T) -> T {
    assert!(matches!(digits, 28 | 40 | 80));
    struct Restore(i128);
    impl Drop for Restore {
        fn drop(&mut self) {
            PRECISION.set(self.0);
        }
    }
    let _restore = Restore(PRECISION.replace(digits));
    action()
}

const EMAX: i128 = 999_999;
const EMIN: i128 = -999_999;

// Exact conversion runs under libmpdec's maximum context on 64-bit builds.
const MAX_EMAX: i128 = 999_999_999_999_999_999;
const MAX_PREC: i128 = 999_999_999_999_999_999;
const MAX_ETINY: i128 = -MAX_EMAX - (MAX_PREC - 1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    HalfEven,
    HalfUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Finite,
    Infinite,
    NaN,
}

/// A decimal value: `(-1)**negative * coefficient * 10**exp` for finite values.
#[derive(Clone, Debug)]
pub struct Decimal {
    negative: bool,
    /// Coefficient digits without leading zeros ("0" for zero), as `_pydecimal._int`.
    digits: String,
    exp: i128,
    kind: Kind,
}

/// A signal that CPython's default traps raise as an exception.
#[derive(Debug, PartialEq, Eq)]
pub enum Signal {
    InvalidOperation,
    Inexact,
}

/// Arithmetic result plus the `Inexact` flag, which the CSV loader traps.
pub struct Outcome {
    pub value: Decimal,
    pub inexact: bool,
}

fn big(digits: &str) -> BigUint {
    BigUint::parse_bytes(digits.as_bytes(), 10).expect("coefficient digits are ASCII")
}

fn ten_pow(n: usize) -> BigUint {
    num_traits::pow(BigUint::from(10u32), n)
}

fn finite(negative: bool, digits: String, exp: i128) -> Decimal {
    let trimmed = digits.trim_start_matches('0');
    let digits = if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    };
    Decimal {
        negative,
        digits,
        exp,
        kind: Kind::Finite,
    }
}

impl Decimal {
    pub fn zero() -> Decimal {
        finite(false, "0".into(), 0)
    }

    /// `Decimal(text)` with CPython's exact string conversion.
    pub fn parse(text: &str) -> Result<Decimal, Signal> {
        let stripped = pytext::strip(text);
        let mut ascii = String::with_capacity(stripped.len());
        for ch in stripped.chars() {
            let cp = ch as u32;
            if ch == '_' {
                continue;
            }
            if cp > 0 && cp <= 127 {
                ascii.push(ch);
            } else if pytext::is_space(cp) {
                ascii.push(' ');
            } else if let Some(value) = pytext::decimal_value(cp) {
                ascii.push(char::from_digit(value, 10).expect("digit value below 10"));
            } else {
                return Err(Signal::InvalidOperation);
            }
        }
        parse_ascii(&ascii).ok_or(Signal::InvalidOperation)
    }

    pub fn is_finite(&self) -> bool {
        self.kind == Kind::Finite
    }

    pub fn is_zero(&self) -> bool {
        self.is_finite() && self.digits == "0"
    }

    pub fn exponent(&self) -> i128 {
        self.exp
    }

    pub fn is_integral(&self) -> bool {
        self.is_zero()
            || self.exp >= 0
            || self.digits.chars().rev().take_while(|&c| c == '0').count() as i128 >= -self.exp
    }

    /// Fixed-point formatting rounds independently of the arithmetic context.
    pub fn format_places(&self, places: i128) -> Option<String> {
        if !(0..=1 << 30).contains(&places) {
            return None;
        }
        let integral = if self.is_zero() {
            1
        } else {
            (self.adjusted() + 1).max(1)
        };
        // Apply the fixed-output memory ceiling before rescale allocates its padding.
        if integral + places + 2 > 1 << 30 {
            return None;
        }
        rescale(self, -places, Rounding::HalfEven).to_fixed_string()
    }

    pub fn is_signed(&self) -> bool {
        self.negative
    }

    pub fn copy_abs(&self) -> Decimal {
        Decimal {
            negative: false,
            ..self.clone()
        }
    }

    pub fn adjusted(&self) -> i128 {
        self.exp + self.digits.len() as i128 - 1
    }

    fn negate(&self) -> Decimal {
        Decimal {
            negative: !self.negative,
            ..self.clone()
        }
    }

    pub fn add(&self, other: &Decimal) -> Outcome {
        add(self, other)
    }

    pub fn sub(&self, other: &Decimal) -> Outcome {
        add(self, &other.negate())
    }

    pub fn mul(&self, other: &Decimal) -> Decimal {
        self.mul_outcome(other).value
    }

    pub fn mul_outcome(&self, other: &Decimal) -> Outcome {
        let negative = self.negative ^ other.negative;
        let exp = self.exp + other.exp;
        if self.is_zero() || other.is_zero() {
            return fix(finite(negative, "0".into(), exp), Rounding::HalfEven);
        }
        let product = big(&self.digits) * big(&other.digits);
        fix(
            finite(negative, product.to_string(), exp),
            Rounding::HalfEven,
        )
    }

    /// `self / other`; the caller guarantees a non-zero divisor.
    pub fn div(&self, other: &Decimal) -> Decimal {
        assert!(
            !other.is_zero(),
            "division by zero is guarded by the caller"
        );
        let negative = self.negative ^ other.negative;
        if self.is_zero() {
            return fix(
                finite(negative, "0".into(), self.exp - other.exp),
                Rounding::HalfEven,
            )
            .value;
        }
        let shift = other.digits.len() as i128 - self.digits.len() as i128 + precision() + 1;
        let mut exp = self.exp - other.exp - shift;
        let (mut coeff, remainder) = if shift >= 0 {
            (big(&self.digits) * ten_pow(shift as usize)).div_rem(&big(&other.digits))
        } else {
            big(&self.digits).div_rem(&(big(&other.digits) * ten_pow((-shift) as usize)))
        };
        if !remainder.is_zero() {
            if (&coeff % 5u32).is_zero() {
                coeff += 1u32;
            }
        } else {
            let ideal = self.exp - other.exp;
            let ten = BigUint::from(10u32);
            while exp < ideal && (&coeff % &ten).is_zero() {
                coeff /= &ten;
                exp += 1;
            }
        }
        fix(finite(negative, coeff.to_string(), exp), Rounding::HalfEven).value
    }

    /// `abs(self)` under the context, which also rounds.
    pub fn abs(&self) -> Decimal {
        let positive = Decimal {
            negative: false,
            ..self.clone()
        };
        fix(positive, Rounding::HalfEven).value
    }

    /// `self.quantize(Decimal(f"1E{exp}"), rounding=rounding)`.
    pub fn quantize(&self, exp: i128, rounding: Rounding) -> Result<Decimal, Signal> {
        if !(etiny()..=EMAX).contains(&exp) {
            return Err(Signal::InvalidOperation);
        }
        if self.is_zero() {
            return Ok(fix(finite(self.negative, "0".into(), exp), rounding).value);
        }
        if self.adjusted() > EMAX || self.adjusted() - exp + 1 > precision() {
            return Err(Signal::InvalidOperation);
        }
        let answer = rescale(self, exp, rounding);
        if answer.adjusted() > EMAX || answer.digits.len() as i128 > precision() {
            return Err(Signal::InvalidOperation);
        }
        Ok(fix(answer, rounding).value)
    }

    /// `str(self)`, Python's to-sci-string with a capital `E`.
    pub fn to_sci_string(&self) -> String {
        let sign = if self.negative { "-" } else { "" };
        match self.kind {
            Kind::Infinite => return format!("{sign}Infinity"),
            Kind::NaN => return format!("{sign}NaN"),
            Kind::Finite => {}
        }
        let len = self.digits.len() as i128;
        let leftdigits = self.exp + len;
        let dotplace = if self.exp <= 0 && leftdigits > -6 {
            leftdigits
        } else {
            1
        };
        let (intpart, fracpart) = place_point(&self.digits, dotplace, true);
        let exp = if leftdigits == dotplace {
            String::new()
        } else {
            format!("E{:+}", leftdigits - dotplace)
        };
        format!("{sign}{intpart}{fracpart}{exp}")
    }

    /// `format(self, "f")`, or `None` where CPython runs out of memory: a zero
    /// the gateway admits can carry an exponent near -10**18, whose fixed-point
    /// form has that many digits.
    pub fn to_fixed_string(&self) -> Option<String> {
        let sign = if self.negative { "-" } else { "" };
        let value = if self.is_zero() && self.exp > 0 {
            finite(self.negative, "0".into(), 0)
        } else {
            self.clone()
        };
        let dotplace = value.exp + value.digits.len() as i128;
        let length = value.digits.len() as i128 + dotplace.unsigned_abs() as i128;
        // shortcut: the cut-off where CPython raises MemoryError depends on the
        // machine; 1 GiB of digits is treated as out of memory here.
        // Set a measured memory budget if a legitimate input needs more digits.
        if length > 1 << 30 {
            return None;
        }
        let (intpart, fracpart) = place_point(&value.digits, dotplace, false);
        Some(format!("{sign}{intpart}{fracpart}"))
    }

    pub fn cmp_value(&self, other: &Decimal) -> Ordering {
        debug_assert!(self.is_finite() && other.is_finite());
        let sign = |d: &Decimal| {
            if d.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        };
        match (self.is_zero(), other.is_zero()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return sign(other).reverse(),
            (false, true) => return sign(self),
            _ => {}
        }
        if self.negative != other.negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let magnitude = match self.adjusted().cmp(&other.adjusted()) {
            Ordering::Equal => {
                let pad = |d: &Decimal, e: i128| {
                    d.digits.clone() + &"0".repeat((d.exp - e).max(0) as usize)
                };
                let low = self.exp.min(other.exp);
                pad(self, low).cmp(&pad(other, low))
            }
            ordering => ordering,
        };
        if self.negative {
            magnitude.reverse()
        } else {
            magnitude
        }
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Decimal) -> bool {
        self.cmp_value(other) == Ordering::Equal
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Decimal) -> Option<Ordering> {
        Some(self.cmp_value(other))
    }
}

fn place_point(digits: &str, dotplace: i128, sci: bool) -> (String, String) {
    let len = digits.len() as i128;
    if dotplace <= 0 && (sci || dotplace < 0) {
        (
            "0".into(),
            format!(".{}{digits}", "0".repeat((-dotplace) as usize)),
        )
    } else if dotplace >= len {
        (
            format!("{digits}{}", "0".repeat((dotplace - len) as usize)),
            String::new(),
        )
    } else {
        let (int, frac) = digits.split_at(dotplace as usize);
        let int = if int.is_empty() { "0" } else { int };
        (int.to_string(), format!(".{frac}"))
    }
}

/// libmpdec's string grammar after CPython has stripped and converted the text.
fn parse_ascii(text: &str) -> Option<Decimal> {
    let (negative, body) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    if let Some(special) = special_value(negative, &body.to_ascii_lowercase()) {
        return special;
    }
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    let (int, frac) = match mantissa.find('.') {
        Some(at) => (&mantissa[..at], &mantissa[at + 1..]),
        None => (mantissa, ""),
    };
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if int.len() + frac.len() == 0 || !all_digits(int) || !all_digits(frac) {
        return None;
    }
    let exp = match exponent {
        Some(exponent) => exponent_value(exponent)?,
        None => 0,
    } - frac.len() as i128;
    let value = finite(negative, format!("{int}{frac}"), exp);
    // Exact conversion: any rounding or clamping under the maximum context is InvalidOperation.
    if value.is_zero() {
        if !(MAX_ETINY..=MAX_EMAX).contains(&value.exp) {
            return None;
        }
    } else if value.adjusted() > MAX_EMAX || value.exp < MAX_ETINY {
        return None;
    }
    Some(value)
}

/// Infinity and the NaNs: `Some(None)` for a NaN with a malformed payload,
/// `None` when the text is not a special value at all.
fn special_value(negative: bool, lower: &str) -> Option<Option<Decimal>> {
    let special = |kind| Decimal {
        negative,
        digits: "0".into(),
        exp: 0,
        kind,
    };
    if lower == "inf" || lower == "infinity" {
        return Some(Some(special(Kind::Infinite)));
    }
    ["snan", "nan"].iter().find_map(|prefix| {
        let payload = lower.strip_prefix(prefix)?;
        Some(
            payload
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| special(Kind::NaN)),
        )
    })
}

/// The exponent after `e`, saturated like libmpdec's strtoexp; anything this
/// large fails the bounds check that follows.
fn exponent_value(exponent: &str) -> Option<i128> {
    let (negative, digits) = match exponent.as_bytes().first()? {
        b'-' => (true, &exponent[1..]),
        b'+' => (false, &exponent[1..]),
        _ => (false, exponent),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let significant = digits.trim_start_matches('0');
    let value = if significant.len() > 25 {
        i128::MAX / 4
    } else {
        significant.parse::<i128>().unwrap_or(0)
    };
    Some(if negative { -value } else { value })
}

fn all_zeros(digits: &str, from: usize) -> bool {
    digits
        .as_bytes()
        .get(from..)
        .is_none_or(|rest| rest.iter().all(|&b| b == b'0'))
}

fn exact_half(digits: &str, at: usize) -> bool {
    digits.as_bytes().get(at) == Some(&b'5') && all_zeros(digits, at + 1)
}

/// `_pydecimal`'s rounding functions: 1 increments, -1 truncates inexactly, 0 is exact.
fn round_change(digits: &str, keep: usize, rounding: Rounding) -> i8 {
    let half_up = || {
        if matches!(digits.as_bytes().get(keep), Some(b'5'..=b'9')) {
            1
        } else if all_zeros(digits, keep) {
            0
        } else {
            -1
        }
    };
    match rounding {
        Rounding::HalfUp => half_up(),
        Rounding::HalfEven => {
            let even = keep == 0
                || matches!(
                    digits.as_bytes()[keep - 1],
                    b'0' | b'2' | b'4' | b'6' | b'8'
                );
            if exact_half(digits, keep) && even {
                -1
            } else {
                half_up()
            }
        }
    }
}

fn rescale(value: &Decimal, exp: i128, rounding: Rounding) -> Decimal {
    if value.is_zero() {
        return finite(value.negative, "0".into(), exp);
    }
    if value.exp >= exp {
        let padded = value.digits.clone() + &"0".repeat((value.exp - exp) as usize);
        return finite(value.negative, padded, exp);
    }
    let mut source = value.clone();
    let mut keep = value.digits.len() as i128 + value.exp - exp;
    if keep < 0 {
        source = finite(value.negative, "1".into(), exp - 1);
        keep = 0;
    }
    let keep = keep as usize;
    let changed = round_change(&source.digits, keep, rounding);
    let mut coeff = source.digits[..keep].to_string();
    if coeff.is_empty() {
        coeff = "0".into();
    }
    if changed == 1 {
        coeff = (big(&coeff) + 1u32).to_string();
    }
    finite(value.negative, coeff, exp)
}

/// `_pydecimal.Decimal._fix`: round to the context and clamp zero exponents.
fn fix(value: Decimal, rounding: Rounding) -> Outcome {
    if value.is_zero() {
        let new_exp = value.exp.clamp(etiny(), EMAX);
        return Outcome {
            value: finite(value.negative, "0".into(), new_exp),
            inexact: false,
        };
    }
    let mut exp_min = value.digits.len() as i128 + value.exp - precision();
    assert!(
        exp_min <= etop(),
        "overflow is unreachable for the bounded amounts the gateway accepts"
    );
    exp_min = exp_min.max(etiny());
    if value.exp >= exp_min {
        return Outcome {
            value,
            inexact: false,
        };
    }
    let mut source = value.clone();
    let mut keep = value.digits.len() as i128 + value.exp - exp_min;
    if keep < 0 {
        source = finite(value.negative, "1".into(), exp_min - 1);
        keep = 0;
    }
    let keep = keep as usize;
    let changed = round_change(&source.digits, keep, rounding);
    let mut coeff = source.digits[..keep].to_string();
    if coeff.is_empty() {
        coeff = "0".into();
    }
    if changed > 0 {
        coeff = (big(&coeff) + 1u32).to_string();
        if coeff.len() as i128 > precision() {
            coeff.pop();
            exp_min += 1;
        }
    }
    assert!(
        exp_min <= etop(),
        "overflow is unreachable for the bounded amounts the gateway accepts"
    );
    Outcome {
        value: finite(value.negative, coeff, exp_min),
        inexact: changed != 0,
    }
}

fn add(a: &Decimal, b: &Decimal) -> Outcome {
    let exp = a.exp.min(b.exp);
    if a.is_zero() && b.is_zero() {
        return fix(
            finite(a.negative && b.negative, "0".into(), exp),
            Rounding::HalfEven,
        );
    }
    if a.is_zero() {
        let exp = exp.max(b.exp - precision() - 1);
        return fix(rescale(b, exp, Rounding::HalfEven), Rounding::HalfEven);
    }
    if b.is_zero() {
        let exp = exp.max(a.exp - precision() - 1);
        return fix(rescale(a, exp, Rounding::HalfEven), Rounding::HalfEven);
    }
    // _normalize: a tiny operand is replaced by 10**exp, which rounds identically.
    let (mut tmp, mut other) = if a.exp < b.exp {
        (b.clone(), a.clone())
    } else {
        (a.clone(), b.clone())
    };
    let floor = tmp.exp + (-1i128).min(tmp.digits.len() as i128 - precision() - 2);
    if other.digits.len() as i128 + other.exp - 1 < floor {
        other = finite(other.negative, "1".into(), floor);
    }
    let tmp_int = big(&tmp.digits) * ten_pow((tmp.exp - other.exp) as usize);
    tmp.exp = other.exp;
    let other_int = big(&other.digits);
    let (big_int, small_int, big_negative) = if tmp.negative != other.negative {
        match tmp_int.cmp(&other_int) {
            Ordering::Equal => return fix(finite(false, "0".into(), exp), Rounding::HalfEven),
            Ordering::Less => (other_int, tmp_int, other.negative),
            Ordering::Greater => (tmp_int, other_int, tmp.negative),
        }
    } else {
        (tmp_int, other_int, tmp.negative)
    };
    let result = if tmp.negative != other.negative {
        big_int - small_int
    } else {
        big_int + small_int
    };
    fix(
        finite(big_negative, result.to_string(), tmp.exp),
        Rounding::HalfEven,
    )
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_sci_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        Decimal::parse(text).unwrap()
    }

    #[test]
    fn private_contexts_preserve_precision_and_restore_the_caller() {
        let fraction = || d("1").div(&d("3")).to_sci_string();
        assert_eq!(fraction(), format!("0.{}", "3".repeat(28)));
        with_precision(40, || {
            assert_eq!(fraction(), format!("0.{}", "3".repeat(40)));
            with_precision(80, || {
                assert_eq!(fraction(), format!("0.{}", "3".repeat(80)));
                let amount = d("1E+60").add(&d("1"));
                assert!(!amount.inexact);
                assert_eq!(
                    amount.value.to_fixed_string().unwrap(),
                    "1".to_owned() + &"0".repeat(59) + "1"
                );
            });
            assert_eq!(fraction(), format!("0.{}", "3".repeat(40)));
        });
        assert!(d("1E+60").add(&d("1")).inexact);
        let panicked = std::panic::catch_unwind(|| with_precision(80, || panic!("abort context")));
        assert!(panicked.is_err());
        assert_eq!(fraction(), format!("0.{}", "3".repeat(28)));
    }

    #[test]
    fn parses_like_cpython() {
        assert_eq!(d("1_000").to_sci_string(), "1000");
        assert_eq!(d("\u{1c}1.5\u{1f}").to_sci_string(), "1.5");
        assert_eq!(d("\u{663}\u{664}").to_sci_string(), "34");
        assert_eq!(d("1E+2").to_sci_string(), "1E+2");
        assert_eq!(d("-0.00").to_sci_string(), "-0.00");
        assert_eq!(d("+.5").to_sci_string(), "0.5");
        assert_eq!(d("1.").to_sci_string(), "1");
        assert!(!d("nan123").is_finite());
        assert!(!d("-Infinity").is_finite());
        for bad in [
            "1 0",
            ".",
            "-",
            "1e",
            "infinit",
            "1,5",
            "1\u{0}",
            "1e9223372036854775807",
            "0e99999999999999999999",
        ] {
            assert!(Decimal::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            d("1e999999999999999999").adjusted(),
            999_999_999_999_999_999
        );
    }

    #[test]
    fn arithmetic_matches_cpython_probes() {
        let z = d("0E+999999999999999999");
        assert_eq!(z.add(&z).value.to_sci_string(), "0E+999999");
        assert_eq!(z.add(&d("1.00")).value.to_sci_string(), "1.00");
        let tiny = d("0E-999999999999999999");
        assert_eq!(tiny.add(&tiny).value.to_sci_string(), "0E-1000026");
        assert_eq!(
            d("0E-999999999").div(&d("100")).to_sci_string(),
            "0E-1000026"
        );
        assert_eq!(d("-0.00").sub(&d("0.00")).value.to_sci_string(), "-0.00");
        assert_eq!(d("-0").add(&d("-0")).value.to_sci_string(), "-0");
        assert_eq!(d("1.00").sub(&d("1.00")).value.to_sci_string(), "0.00");
        assert_eq!(d("10").div(&d("4")).to_sci_string(), "2.5");
        assert_eq!(
            d("1").div(&d("3")).to_sci_string(),
            "0.3333333333333333333333333333"
        );
        assert_eq!(d("100.00").div(&d("4")).to_sci_string(), "25.00");
        assert_eq!(d("1.00").div(&d("8")).to_sci_string(), "0.125");
        assert_eq!(d("12E+3").div(&d("4E-2")).to_sci_string(), "3E+5");
        let rounded = d("1E+18").sub(&d("1E-18"));
        assert_eq!(
            rounded.value.to_sci_string(),
            "1000000000000000000.000000000"
        );
        assert!(rounded.inexact);
        let carried = d("9999999999999999999999999999.5").add(&Decimal::zero());
        assert_eq!(
            carried.value.to_sci_string(),
            "1.000000000000000000000000000E+28"
        );
        let percent = d("2").div(&d("3")).mul(&d("100"));
        assert_eq!(
            percent
                .quantize(-4, Rounding::HalfUp)
                .unwrap()
                .to_sci_string(),
            "66.6667"
        );
        assert_eq!(
            d("1E+25").quantize(-4, Rounding::HalfUp),
            Err(Signal::InvalidOperation)
        );
    }

    #[test]
    fn fixed_format_matches_cpython() {
        let fixed = |text: &str| d(text).to_fixed_string().unwrap();
        assert_eq!(fixed("1E+3"), "1000");
        assert_eq!(fixed("-0.00"), "-0.00");
        assert_eq!(fixed("1.20E-5"), "0.0000120");
        assert_eq!(fixed("0E-3"), "0.000");
        assert_eq!(fixed("0E+3"), "0");
        assert_eq!(d("0E-999999999999").to_fixed_string(), None);
        assert_eq!(d("1E+999999999999999999").format_places(2), None);
        assert_eq!(
            d("-0E+999999999999999999").format_places(2),
            Some("-0.00".into())
        );
        assert_eq!(d("1.235").format_places(2), Some("1.24".into()));
    }
}
