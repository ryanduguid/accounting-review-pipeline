// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Public compatibility contracts exercised alongside the feature-gated probe.
#![cfg(feature = "probe")]

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/scratch.rs"]
mod scratch;

use base64::Engine;
use scratch::Scratch;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, process::Stdio};
use xero_trial_balance_export::{
    amount::Amount,
    cli,
    pyjson::{self, Json},
};

fn probe_output(requests: &[&str]) -> String {
    let scratch = Scratch::new();
    let mut child = child_profile::command(env!("CARGO_BIN_EXE_xero-tb-probe"))
        .current_dir(&scratch.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}\n", requests.join("\n")).as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(fs::read_dir(&scratch.0).unwrap().next().is_none());
    String::from_utf8(output.stdout).unwrap()
}

fn probe_responses(requests: &[&str]) -> Vec<Json> {
    let output = probe_output(requests);
    let responses: Vec<_> = output
        .lines()
        .map(|line| pyjson::loads(line).unwrap())
        .collect();
    assert_eq!(responses.len(), requests.len());
    responses
}

fn assert_json(actual: &Json, expected: &str) {
    assert_eq!(
        pyjson::canonical(actual),
        pyjson::canonical(&pyjson::loads(expected).unwrap())
    );
}

fn assert_refusal(response: &Json, message: &str) {
    assert_json(response.at("code"), "1");
    assert!(response.at("stdout").is_str(""));
    assert!(response.at("stderr").is_str(message), "{response:?}");
}

#[test]
fn probe_refusals_preserve_the_next_request_and_response_json_semantics() {
    let requests = [
        "{malformed",
        r#"{"op":"fabricated_unknown"}"#,
        r#"{"op":"json_response","args":["{malformed"]}"#,
        r#"{"op":"json_response","args":["{\"duplicate\":1,\"duplicate\":2,\"escapes\":\"\\/\\b\\f\"}"]}"#,
    ];
    assert_eq!(
        probe_output(&requests),
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
fn probe_amounts_keep_decimal_precision_and_component_order() {
    let responses = probe_responses(&[
        r#"{"op":"row_amounts","args":[{"Debit":"0.10","Credit":"2.00","YTD Debit":"30.50","YTD Credit":"4.25"}]}"#,
        r#"{"op":"add_totals","args":[["0.1","10","100","1000"],["0.2","-3","-25","-250"]]}"#,
        r#"{"op":"add_totals","args":[["1e30","0","0","0"],["1e-30","0","0","0"]]}"#,
    ]);
    assert_json(responses[0].at("ok"), r#"["0.1","2.0","30.5","4.25"]"#);
    assert_json(responses[1].at("ok"), r#"["0.3","7.0","75.0","750.0"]"#);
    assert_refusal(
        &responses[2],
        "Nothing written - a reported amount needs more precision than the balance check can carry exactly, so the totals cannot be trusted. Check the report for a malformed cell.\n",
    );
}

const BALANCED_ROWS: &str = concat!(
    r#"[{"AccountID":"fabricated-cash","Account":"Cash (100)","Section":"Assets","Debit":"0.1","Credit":"0","YTD Debit":"1","YTD Credit":"0"},"#,
    r#"{"AccountID":"fabricated-capital","Account":"Capital (200)","Section":"Equity","Debit":"0","Credit":"0.1","YTD Debit":"0","YTD Credit":"1"}]"#
);

#[test]
fn probe_shape_and_tenant_refusals_allow_the_next_request() {
    let missing_id = format!(
        r#"{{"op":"csv_artifact","args":[{BALANCED_ROWS},{{"tenantName":"Fabricated Organisation"}},"2026-06-30"]}}"#
    );
    let missing_name = format!(
        r#"{{"op":"csv_artifact","args":[{BALANCED_ROWS},{{"tenantId":"fabricated-tenant"}},"2026-06-30"]}}"#
    );
    let responses = probe_responses(&[
        r#"{"op":"add_totals","args":[{},["0","0","0","0"]]}"#,
        r#"{"op":"add_totals","args":[["0","0","0"],["0","0","0","0"]]}"#,
        r#"{"op":"select_tenant","args":[{}]}"#,
        &missing_id,
        &missing_name,
        r#"{"op":"to_number","args":["2.50"]}"#,
    ]);
    for (response, message) in responses.iter().zip([
        "expected four amounts\n",
        "expected four amounts\n",
        "expected connections\n",
        "'tenantId'\n",
        "'tenantName'\n",
    ]) {
        assert_refusal(response, message);
    }
    assert_json(responses[5].at("ok"), r#""2.5""#);
}

#[test]
fn probe_missing_operations_refuse_without_stopping_the_stream() {
    let responses = probe_responses(&["{}", "[]", r#"{"op":"to_number","args":["2.50"]}"#]);
    assert_refusal(&responses[0], "missing probe operation\n");
    assert_refusal(&responses[1], "missing probe operation\n");
    assert_json(responses[2].at("ok"), r#""2.5""#);
}

#[test]
fn probe_manifest_arguments_refuse_without_stopping_the_stream() {
    let responses = probe_responses(&[
        r#"{"op":"manifest","args":["","fabricated-digest",{"tenantId":"fabricated","tenantName":"Fabricated"},"2026-06-30","accrual","2026-06-30T00:00:00+00:00"]}"#,
        r#"{"op":"manifest","args":["nested/report.csv","fabricated-digest",{"tenantName":"Fabricated"},"2026-06-30","accrual","2026-06-30T00:00:00+00:00"]}"#,
        r#"{"op":"manifest","args":["nested/report.csv","fabricated-digest",{"tenantId":"fabricated"},"2026-06-30","accrual","2026-06-30T00:00:00+00:00"]}"#,
        r#"{"op":"to_number","args":["2.50"]}"#,
    ]);
    assert_refusal(&responses[0], "expected a manifest CSV filename\n");
    assert_refusal(&responses[1], "'tenantId'\n");
    assert_refusal(&responses[2], "'tenantName'\n");
    assert_json(responses[3].at("ok"), r#""2.5""#);
}

fn assert_manifest(
    value: &Json,
    csv: &str,
    digest: &str,
    date: &str,
    basis: &str,
    generated: &str,
) {
    let expected = r#"{
        "schema_version":"xero-source-manifest.v1",
        "mode":"live",
        "source_system":"xero-trial-balance-export",
        "entity_ref":{"tenant_id":"fabricated-tenant","tenant_name":"Fabricated Organisation"},
        "report":{"name":"Trial Balance","as_at":"$DATE","basis":"$BASIS"},
        "export":{"schema":"xero-tb-csv.v1","csv":"$CSV","sha256":"$DIGEST","generated_at":"$GENERATED"}
    }"#
    .replace("$DATE", date)
    .replace("$BASIS", basis)
    .replace("$CSV", csv)
    .replace("$DIGEST", digest)
    .replace("$GENERATED", generated);
    assert_json(value, &expected);
}

#[test]
fn probe_csv_bytes_and_manifests_preserve_their_arguments() {
    let expected_csv = concat!(
        "\u{feff}ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\r\n",
        "2026-06-30,Fabricated Organisation,Assets,fabricated-cash,Cash,100,0.1,0.0,1.0,0.0\r\n",
        "2026-06-30,Fabricated Organisation,Equity,fabricated-capital,Capital,200,0.0,0.1,0.0,1.0\r\n",
    );
    let digest: String = Sha256::digest(expected_csv.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let tenant = r#"{"tenantId":"fabricated-tenant","tenantName":"Fabricated Organisation"}"#;
    let csv_request =
        format!(r#"{{"op":"csv_artifact","args":[{BALANCED_ROWS},{tenant},"2026-06-30"]}}"#);
    let manifest_request = format!(
        r#"{{"op":"manifest","args":["nested/second.csv","{digest}",{tenant},"2026-07-31","cash","2026-08-01T01:02:03+00:00"]}}"#
    );
    let responses = probe_responses(&[&csv_request, &manifest_request]);
    let artifact = responses[0].at("ok");
    let csv = artifact.at("csv").as_pystr().unwrap().os_text();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(csv)
            .unwrap(),
        expected_csv.as_bytes()
    );
    let manifest = pyjson::loads(&artifact.at("manifest").as_pystr().unwrap().os_text()).unwrap();
    assert_manifest(
        &manifest,
        "report.csv",
        &digest,
        "2026-06-30",
        "accrual",
        "2026-06-30T12:34:56+00:00",
    );
    assert_manifest(
        responses[1].at("ok"),
        "second.csv",
        &digest,
        "2026-07-31",
        "cash",
        "2026-08-01T01:02:03+00:00",
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
