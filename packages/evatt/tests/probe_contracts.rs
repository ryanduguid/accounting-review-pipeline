// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

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
    assert!(!array(responses[3].at("structured")).is_empty());
    assert!(!array(responses[3].at("email_scanner")).is_empty());
    assert!(!array(responses[3].at("numbers")).is_empty());
    assert!(!array(responses[3].at("unknowns")).is_empty());
    assert_measurement(&responses[4], 11, 0, 0, 0);
}
