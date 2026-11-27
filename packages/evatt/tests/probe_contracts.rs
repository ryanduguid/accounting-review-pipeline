// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

#![cfg(feature = "probe")]

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use evatt::{
    benchmark, patterns,
    pyjson::{self, Json},
    redact,
};
use num_traits::ToPrimitive;
use std::{io::Write, process::Stdio};

fn integer(value: &Json) -> i64 {
    value.as_plain_int().unwrap().to_i64().unwrap()
}

fn array(value: &Json) -> &[Json] {
    match value {
        Json::Array(items) => items,
        _ => panic!("expected an array: {value:?}"),
    }
}

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}

/// Each reported `[start, end, value]` (or `[start, end, kind, value]`) span,
/// with `kind` empty when the field has none.
fn reported(value: &Json) -> Vec<(i64, i64, String, String)> {
    array(value)
        .iter()
        .map(|item| match array(item) {
            [start, end, value] => (integer(start), integer(end), String::new(), text(value)),
            [start, end, kind, value] => (integer(start), integer(end), text(kind), text(value)),
            other => panic!("expected a span: {other:?}"),
        })
        .collect()
}

/// The library's spans in the probe's shape; the probe text is ASCII, so byte
/// offsets are code point offsets.
fn expected(spans: &[patterns::Span], kinds: bool) -> Vec<(i64, i64, String, String)> {
    spans
        .iter()
        .map(|s| {
            let kind = if kinds { s.kind.clone() } else { String::new() };
            (s.start as i64, s.end as i64, kind, s.value.clone())
        })
        .collect()
}

#[test]
fn small_benchmark_requests_report_the_measured_source_and_counters() {
    let requests = [
        (
            r#"{"family":"label","pattern":"TFN_LABELLED","prefix":"TFN","unit":" ","suffix":"","size":8}"#,
            11,
            0,
            0,
            0,
        ),
        (
            r#"{"family":"value","value":"A B","prefix":"A","unit":"*","suffix":"X","size":8}"#,
            10,
            0,
            0,
            0,
        ),
        (
            r#"{"family":"label","pattern":"TFN_LABELLED","prefix":"TFN","markup":true,"padding":"","size":8}"#,
            61,
            0,
            0,
            0,
        ),
    ];
    for (request, points, candidates, retries, examined) in requests {
        let measured = benchmark::measure(&pyjson::loads(request).unwrap()).unwrap();
        assert_measurement(&measured, points, candidates, retries, examined);
    }
    let attempts = patterns::name_attempts("Tax Court Tax Court ").unwrap();
    let measured = benchmark::measure(
        &pyjson::loads(r#"{"family":"name","prefix":"","unit":"Tax Court ","suffix":"","size":2}"#).unwrap(),
    )
    .unwrap();
    assert_measurement(
        &measured,
        20,
        attempts.len(),
        attempts.iter().filter(|a| a.2).count(),
        0,
    );

    let (_, stats) = redact::numbers_counted("<a <a <a <a ").unwrap();
    let measured = benchmark::measure(
        &pyjson::loads(r#"{"family":"number","prefix":"","unit":"<a ","suffix":"","size":4}"#).unwrap(),
    )
    .unwrap();
    assert_measurement(&measured, 12, stats.candidate_starts, 0, stats.characters_examined);
}

fn assert_measurement(measured: &Json, points: i64, candidates: usize, retries: usize, examined: usize) {
    assert!(measured.has_exact_keys(&[
        "seconds",
        "code_points",
        "candidate_starts",
        "retries",
        "characters_examined",
    ]));
    let Json::Float(seconds) = measured.at("seconds") else {
        panic!("expected elapsed seconds");
    };
    assert!(seconds.is_finite() && *seconds >= 0.0);
    assert_eq!(integer(measured.at("code_points")), points);
    assert_eq!(integer(measured.at("candidate_starts")), candidates as i64);
    assert_eq!(integer(measured.at("retries")), retries as i64);
    assert_eq!(integer(measured.at("characters_examined")), examined as i64);
}

#[test]
fn unknown_benchmark_families_return_an_error() {
    let request = pyjson::loads(r#"{"family":"unknown","prefix":"","unit":"","suffix":"","size":0}"#).unwrap();
    assert_eq!(benchmark::measure(&request).unwrap_err(), "unknown benchmark family");
}

#[test]
fn probe_preserves_unicode_offsets_and_continues_after_a_bad_json_line() {
    let requests = [
        "{",
        r#"{"text":"é Jane Roe","entries":[{"value":"Jane Roe","placeholder":"PERSON_01","kind":"person","added":"2026-10-13"}],"values":["Jane Roe"]}"#,
        r#"{"text":"ordinary text","values":["\udcff"]}"#,
        r#"{"text":"TFN: 123456782\nsample@example.test\n12345678\n12345678"}"#,
        r#"{"mode":"benchmark","family":"label","pattern":"TFN_LABELLED","prefix":"TFN","unit":" ","suffix":"","size":8}"#,
    ];
    let mut child = child_profile::command(env!("CARGO_BIN_EXE_evatt-probe"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all((requests.join("\n") + "\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty());
    let responses = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| pyjson::loads(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), requests.len());
    assert!(responses[0].has_exact_keys(&["error"]));
    assert!(array(responses[1].at("redact"))[0].is_str("é PERSON_01"));
    assert_eq!(integer(array(responses[1].at("redact"))[1].at("person")), 1);
    let name = &array(responses[1].at("names"))[0];
    assert_eq!(integer(&array(name)[0]), 2);
    assert_eq!(integer(&array(name)[1]), 10);
    assert!(array(name)[2].is_str("Jane Roe"));
    assert!(!array(&array(responses[1].at("values"))[0]).is_empty());
    assert!(array(&array(responses[2].at("values"))[0]).is_empty());
    let detected = "TFN: 123456782
sample@example.test
12345678
12345678";
    let structured = reported(responses[3].at("structured"));
    assert_eq!(structured[0], (5, 14, "tfn".to_owned(), "123456782".to_owned()));
    let (spans, _) = patterns::structured_and_ambiguous(detected).unwrap();
    assert_eq!(structured, expected(&spans, true));
    let email = reported(responses[3].at("email_scanner"));
    assert_eq!(email, [(15, 34, String::new(), "sample@example.test".to_owned())]);
    assert_eq!(email, expected(&evatt::email::spans(detected), false));
    let numbers = reported(responses[3].at("numbers"));
    assert!(numbers.contains(&(35, 43, String::new(), "12345678".to_owned())));
    assert!(numbers.contains(&(44, 52, String::new(), "12345678".to_owned())));
    assert_eq!(numbers, expected(&redact::numbers(detected).unwrap(), false));
    assert!(!array(responses[3].at("unknowns")).is_empty());
    assert_measurement(&responses[4], 11, 0, 0, 0);
}
