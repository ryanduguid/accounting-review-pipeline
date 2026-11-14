// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use elizabeth_anne_alexander::decimal::{Decimal, Rounding, Signal};

fn decimal(value: &str) -> Decimal {
    Decimal::parse(value).unwrap()
}

#[test]
fn parsing_refuses_internal_whitespace_and_invalid_exponents() {
    for value in [
        "1\u{a0}0",
        "1e+",
        "1e2x",
        "1e12345678901234567890123456",
        "1e-12345678901234567890123456",
    ] {
        assert_eq!(Decimal::parse(value), Err(Signal::InvalidOperation), "{value:?}");
    }
}

#[test]
fn quantize_refuses_context_exponents_and_a_carry_beyond_precision() {
    let zero = Decimal::zero();
    for exponent in [-1_000_027, 1_000_000] {
        assert_eq!(
            zero.quantize(exponent, Rounding::HalfEven),
            Err(Signal::InvalidOperation)
        );
    }
    assert_eq!(
        decimal("9.9999999999999999999999999995").quantize(-27, Rounding::HalfEven),
        Err(Signal::InvalidOperation)
    );
    let adjacent = decimal("9.9999999999999999999999999985")
        .quantize(-27, Rounding::HalfEven)
        .unwrap();
    assert_eq!(adjacent.to_sci_string(), "9.999999999999999999999999998");
}

#[test]
fn special_values_preserve_signed_scientific_and_display_text() {
    for value in ["-Infinity", "-NaN"] {
        let parsed = decimal(value);
        assert!(!parsed.is_finite());
        assert_eq!(parsed.to_sci_string(), value);
        assert_eq!(parsed.to_string(), value);
    }
}

#[test]
fn addition_reports_signed_nonzero_underflow_and_the_exact_boundary() {
    let zero = Decimal::zero();
    for (value, expected) in [("1E-1000028", "0E-1000026"), ("-1E-1000028", "-0E-1000026")] {
        let outcome = decimal(value).add(&zero);
        assert!(outcome.inexact);
        assert!(outcome.value.is_zero());
        assert_eq!(outcome.value.exponent(), -1_000_026);
        assert_eq!(outcome.value.to_sci_string(), expected);
    }
    let adjacent = decimal("1E-1000026").add(&zero);
    assert!(!adjacent.inexact);
    assert!(!adjacent.value.is_zero());
    assert_eq!(adjacent.value.exponent(), -1_000_026);
    assert_eq!(adjacent.value.to_sci_string(), "1E-1000026");
}
