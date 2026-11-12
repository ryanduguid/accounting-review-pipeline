// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Public compatibility contracts exercised alongside the feature-gated probe.
#![cfg(feature = "probe")]

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use std::{io::Write, process::Stdio};
use xero_trial_balance_export::{amount::Amount, cli};

#[test]
fn probe_refusals_preserve_the_next_request_and_response_json_semantics() {
    let mut child = child_profile::command(env!("CARGO_BIN_EXE_xero-tb-probe"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        "{malformed",
        r#"{"op":"fabricated_unknown"}"#,
        r#"{"op":"json_response","args":["{malformed"]}"#,
        r#"{"op":"json_response","args":["{\"duplicate\":1,\"duplicate\":2,\"escapes\":\"\\/\\b\\f\"}"]}"#,
    ];
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}\n", requests.join("\n")).as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!(
            r#"{"code":1,"stderr":"invalid probe JSON"}"#,
            "\n",
            r#"{"code":1,"stderr":"unknown probe operation: fabricated_unknown\n","stdout":""}"#,
            "\n",
            r#"{"code":1,"stderr":"invalid JSON\n","stdout":""}"#,
            "\n",
            r#"{"ok":{"duplicate":2,"escapes":"/\b\f"}}"#,
            "\n",
        )
    );
}

#[test]
fn grouped_display_uses_exact_half_even_rounding_and_preserves_negative_zero() {
    for (input, expected) in [
        ("1234.565", "1,234.56"),
        ("1234.575", "1,234.58"),
        ("9.999", "10.00"),
        ("-0.004", "-0.00"),
        ("1234", "1,234.00"),
    ] {
        assert_eq!(
            Amount::parse(input).unwrap().grouped_two(),
            expected,
            "{input}"
        );
    }
}

#[test]
fn iso_week_dates_keep_weekdays_year_boundaries_and_separator_rules() {
    for (input, expected) in [
        ("2026-W27-4", "2026-07-02"),
        ("2026-W27-5", "2026-07-03"),
        ("2026-W27-6", "2026-07-04"),
        ("2026-W27-7", "2026-07-05"),
        ("2015-W53-7", "2016-01-03"),
        ("2020-W53-1", "2020-12-28"),
    ] {
        assert_eq!(cli::iso_date(input).unwrap(), expected, "{input}");
    }
    for input in [
        "2026-W274",
        "2026W27-4",
        "2026-W2700",
        "2026-W1",
        "2021-W53-1",
    ] {
        let error = cli::iso_date(input).unwrap_err();
        assert_eq!(error.code, 1, "{input}");
        assert!(error.stdout.is_empty(), "{input}");
        assert!(
            error
                .stderr
                .starts_with(&format!("\"{input}\" is not YYYY-MM-DD"))
        );
    }
}
