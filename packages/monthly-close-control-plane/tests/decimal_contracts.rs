// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use monthly_close_control_plane::decimal::{Decimal, Rounding, Signal, with_precision};
use std::cmp::Ordering;

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
        assert_eq!(
            Decimal::parse(value),
            Err(Signal::InvalidOperation),
            "{value:?}"
        );
    }
}

#[test]
fn fixed_formatting_refuses_invalid_places_and_preserves_rounded_zero_signs() {
    let one = decimal("1");
    assert_eq!(one.format_places(-1), None);
    assert_eq!(one.format_places(1_073_741_825), None);
    for (value, expected) in [("0.0001", "0.00"), ("-0.0001", "-0.00")] {
        assert_eq!(decimal(value).format_places(2).as_deref(), Some(expected));
    }
}

#[test]
fn special_values_preserve_their_sign_in_scientific_and_display_output() {
    for value in ["-Infinity", "-NaN"] {
        let parsed = decimal(value);
        assert!(!parsed.is_finite());
        assert_eq!(parsed.to_sci_string(), value);
        assert_eq!(parsed.to_string(), value);
    }
}

#[test]
fn division_rounds_a_coefficient_longer_than_the_context_precision() {
    with_precision(28, || {
        let result = decimal("1234567890123456789012345678901").div(&decimal("1"));
        assert_eq!(result.to_sci_string(), "1.234567890123456789012345679E+30");
    });
}

#[test]
fn opposite_sign_ordering_and_signed_zero_equality_are_numeric() {
    let negative = decimal("-1");
    let positive = decimal("1");
    assert_eq!(negative.cmp_value(&positive), Ordering::Less);
    assert_eq!(positive.partial_cmp(&negative), Some(Ordering::Greater));
    assert_eq!(decimal("-0"), decimal("0"));
}

#[test]
fn quantize_refuses_exponent_limits_and_a_rounding_carry_beyond_precision() {
    with_precision(28, || {
        let one = decimal("1");
        for exponent in [-1_000_027, 1_000_000] {
            assert_eq!(
                one.quantize(exponent, Rounding::HalfEven),
                Err(Signal::InvalidOperation)
            );
        }
        assert_eq!(
            decimal("9999999999999999999999999999.5").quantize(0, Rounding::HalfEven),
            Err(Signal::InvalidOperation)
        );
        let adjacent = decimal("9999999999999999999999999998.5")
            .quantize(0, Rounding::HalfEven)
            .unwrap();
        assert_eq!(adjacent.to_sci_string(), "9999999999999999999999999998");
    });
}

#[test]
fn nonzero_underflow_reports_inexact_signed_zero_at_the_context_limit() {
    with_precision(28, || {
        let one = decimal("1");
        for (value, expected) in [("1E-1000028", "0E-1000026"), ("-1E-1000028", "-0E-1000026")] {
            let outcome = decimal(value).mul_outcome(&one);
            assert!(outcome.inexact);
            assert!(outcome.value.is_zero());
            assert_eq!(outcome.value.exponent(), -1_000_026);
            assert_eq!(outcome.value.to_sci_string(), expected);
        }
        let adjacent = decimal("1E-1000026").mul_outcome(&one);
        assert!(!adjacent.inexact);
        assert!(!adjacent.value.is_zero());
        assert_eq!(adjacent.value.exponent(), -1_000_026);
        assert_eq!(adjacent.value.to_sci_string(), "1E-1000026");
    });
}
