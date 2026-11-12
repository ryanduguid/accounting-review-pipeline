// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! The exporter-owned `xero-tb-csv.v1` contract at the repository root: its
//! bytes, and this exporter's offline integrity runner's outcome for every
//! fixture. The offline review consumers meet the same fixtures in the joined
//! conformance crate, tests/joined.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/scratch.rs"]
mod scratch;

use scratch::Scratch;
use sha2::{Digest, Sha256};
use std::{ffi::OsString, path::Path};
use xero_trial_balance_export::pycsv;
use xero_trial_balance_export::pyjson::{self, Json};

const CONTRACT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/xero-trial-balance-v1"
);
const HEADER: [&str; 10] = [
    "ReportDate",
    "Tenant",
    "Section",
    "AccountID",
    "AccountName",
    "AccountCode",
    "Debit",
    "Credit",
    "YTDDebit",
    "YTDCredit",
];
const FIXTURE_DIGESTS: [(&str, &str); 3] = [
    (
        "passing.csv",
        "2cbe9997a8e7210936ff3c59b5d3fdb0041c1b375b0f9c88cf9ee30d0f356a09",
    ),
    (
        "failing_movement.csv",
        "702175df967b2854e7897cd27fdc4aca441e21b52438381108fabe88ff3153e4",
    ),
    (
        "failing_ytd.csv",
        "ec757f12d13866360fbab189228ebb425893c6f8b299809c6f8567bf5817c64b",
    ),
];

fn text(value: &Json) -> String {
    value.as_pystr().and_then(|s| s.to_utf8()).unwrap()
}

fn items(value: &Json) -> &[Json] {
    match value {
        Json::Array(items) => items,
        other => panic!("expected a list, got {other:?}"),
    }
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// The fixture directory, the digests and the scenarios name the same files, or
/// a new CSV joins the tree with no digest and no scenario, and never reaches
/// the exporter or its consumers.
fn assert_one_fixture_set(contract: &Path, scenarios: &[Json]) {
    let digests = sorted(
        FIXTURE_DIGESTS
            .iter()
            .map(|(name, _)| name.to_string())
            .collect(),
    );
    let fixtures = sorted(
        std::fs::read_dir(contract.join("fixtures"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".csv"))
            .collect(),
    );
    assert_eq!(fixtures, digests);
    let named = sorted(
        scenarios
            .iter()
            .map(|item| text(item.at("fixture")))
            .collect(),
    );
    assert_eq!(named, digests);
}

/// The fixture's bytes, then the integrity runner's exit code and output.
fn assert_fixture(fixture: &Path, digest: &str, scenario: &Json) {
    let content = std::fs::read(fixture).unwrap();
    assert!(!content.windows(2).any(|pair| pair == b"\r\n"));
    let actual: String = Sha256::digest(&content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(actual, digest);
    let source = std::str::from_utf8(&content).unwrap();
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let first = pycsv::Reader::new(source).next().unwrap().unwrap();
    assert_eq!(first, HEADER);

    let result = child_profile::command(env!("CARGO_BIN_EXE_xero-tb-evaluate"))
        .arg(fixture)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let code = result.status.code().map(|code| code.to_string());
    let want = pyjson::canonical(scenario.at("exit_code"));
    assert_eq!(code, Some(want), "{output}");
    for marker in items(scenario.at("output_contains")) {
        assert!(output.contains(&text(marker)), "{output}");
    }
}

#[test]
fn exporter_owned_bytes_and_outcomes_are_the_root_contract() {
    let contract = Path::new(CONTRACT);
    let expected = std::fs::read_to_string(contract.join("expected_results.json")).unwrap();
    let expected = pyjson::loads(&expected).unwrap();
    assert!(expected.at("corpus_id").is_str("xero-tb-csv.v1"));
    let columns: Vec<String> = items(expected.at("canonical_columns"))
        .iter()
        .map(text)
        .collect();
    assert_eq!(columns, HEADER);
    assert_eq!(
        std::fs::read(contract.join("schema.csv")).unwrap(),
        format!("{}\n", HEADER.join(",")).into_bytes()
    );

    let scenarios = items(expected.at("scenarios"));
    assert_one_fixture_set(contract, scenarios);
    for (name, digest) in FIXTURE_DIGESTS {
        eprintln!("fixture {name}");
        let scenario = scenarios
            .iter()
            .find(|item| item.at("fixture").is_str(name))
            .unwrap();
        assert_fixture(&contract.join("fixtures").join(name), digest, scenario);
    }
}

#[test]
fn evaluator_refuses_wrong_arity_and_undeclared_fixture_paths() {
    let directory = Scratch::new();
    let fixtures = Path::new(CONTRACT).join("fixtures");
    let passing = fixtures.join("passing.csv");
    let failing = fixtures.join("failing_movement.csv");
    let undeclared = fixtures.join("not-declared.csv");
    assert!(!undeclared.exists());
    let external = directory.0.join("passing.csv");
    let bytes = std::fs::read(&passing).unwrap();
    std::fs::write(&external, &bytes).unwrap();
    let usage = "usage: xero-tb-evaluate FIXTURE.csv\n";
    let policy = "error: fixture path must name a declared fabricated CSV fixture\n";
    let cases = [
        (vec![], usage),
        (
            vec![passing.clone().into_os_string(), failing.into_os_string()],
            usage,
        ),
        (vec![undeclared.into_os_string()], policy),
        (vec![external.clone().into_os_string()], policy),
    ];
    let windows_device = cfg!(windows).then_some((
        vec![OsString::from(r"\\.\NUL")],
        "error: fixture path must not use a Windows device namespace\n",
    ));
    for (arguments, diagnostic) in cases.into_iter().chain(windows_device) {
        assert_evaluator_refusal(&arguments, &directory.0, diagnostic);
        assert_eq!(std::fs::read(&external).unwrap(), bytes);
        assert_eq!(std::fs::read(&passing).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }
}

fn assert_evaluator_refusal(arguments: &[OsString], directory: &Path, diagnostic: &str) {
    let output = child_profile::command(env!("CARGO_BIN_EXE_xero-tb-evaluate"))
        .args(arguments)
        .current_dir(directory)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{arguments:?}");
    assert!(output.stdout.is_empty(), "{arguments:?}");
    assert_eq!(
        String::from_utf8(output.stderr)
            .unwrap()
            .replace("\r\n", "\n"),
        diagnostic,
        "{arguments:?}",
    );
}

#[test]
fn quick_proof_check_and_argument_diagnostics_preserve_committed_assets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let assets = ["assets/quick-proof.svg", "assets/quick-proof.md"];
    let prior = assets.map(|name| std::fs::read(root.join(name)).unwrap());
    for (argument, code, stdout, stderr) in [
        ("--check", 0, "quick proof is current\n", ""),
        ("--help", 0, "usage: xero-tb-proof [--check]\n", ""),
        ("-h", 0, "usage: xero-tb-proof [--check]\n", ""),
        ("--invalid", 2, "", "usage: xero-tb-proof [--check]\n"),
    ] {
        let output = child_profile::command(env!("CARGO_BIN_EXE_xero-tb-proof"))
            .arg(argument)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code));
        assert_eq!(
            String::from_utf8(output.stdout)
                .unwrap()
                .replace("\r\n", "\n"),
            stdout
        );
        assert_eq!(
            String::from_utf8(output.stderr)
                .unwrap()
                .replace("\r\n", "\n"),
            stderr
        );
        for (name, bytes) in assets.iter().zip(&prior) {
            assert_eq!(&std::fs::read(root.join(name)).unwrap(), bytes);
        }
    }
}
