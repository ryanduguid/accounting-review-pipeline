// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use review_ready_gate::loader::parse_money;
use std::{cmp::Ordering, path::Path};

#[test]
fn parsed_money_preserves_signed_zero_precision_and_negative_ordering() {
    let money = |text| parse_money(Some(text), "amount", 2, Path::new("fabricated.csv")).unwrap();
    assert_eq!(money("-0.00").mul(&money("2.5")).to_sci_string(), "-0.000");
    assert_eq!(
        money("1000000000000000000000000000000")
            .div(&money("1"))
            .to_sci_string(),
        "1.000000000000000000000000000E+30"
    );
    assert_eq!(money("-2").cmp_value(&money("1")), Ordering::Less);
    assert_eq!(money("1").cmp_value(&money("-2")), Ordering::Greater);
    assert_eq!(money("-2").cmp_value(&money("-1")), Ordering::Less);
}

#[test]
fn parsed_money_formats_half_even_and_refuses_invalid_precision() {
    let money = |text| parse_money(Some(text), "amount", 2, Path::new("fabricated.csv")).unwrap();
    for places in [-1, 1_073_741_825] {
        assert_eq!(money("1.25").format_places(places), None);
    }
    assert_eq!(money("1.245").format_places(2).as_deref(), Some("1.24"));
    assert_eq!(money("0.001").format_places(1).as_deref(), Some("0.0"));
    assert_eq!(format!("{}", money("-2.50")), "-2.50");
}
