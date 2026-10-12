//! Ports of tests/test_assurance_claims.py, tests/test_trial_balance_conformance.py
//! and the README grammar check from tests/test_gateway.py.

use std::path::Path;

use super::*;
use crate::gateway::{CANONICAL_COLUMNS, load_tb_snapshot};
use crate::util::{Location, sha256_hex, snapshot_file};

const COMPONENT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(COMPONENT).join(rel)).unwrap()
}

fn normalise(text: &str) -> String {
    text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn test_readme_states_the_assurance_boundary_before_product_claims() {
    let readme = read("README.md");
    let opening = normalise(&readme[..readme.find("```").unwrap()]);
    assert!(opening.contains("synthetic-only"));
    assert!(opening.contains("unkeyed"));
    assert!(opening.contains("anyone who can replace the artefacts can replace the receipt"));
}

#[test]
fn test_public_docs_define_only_local_checksum_integrity() {
    for document in [read("README.md"), read("DATA-FLOW.md")] {
        let text = normalise(&document);
        assert!(text.contains("synthetic-only"));
        assert!(text.contains("unkeyed"));
        assert!(text.contains("local sha-256 checksum binding"));
        assert!(text.contains("does not prove authorship, source system, origin, time, or immutability"));
    }
}

#[test]
fn test_retired_assurance_claims_do_not_return() {
    let public_docs = normalise(&format!("{}\n{}", read("README.md"), read("DATA-FLOW.md")));
    for claim in [
        "a validated xero tb export",
        "validated xero trial balance export",
        "cryptographic sha-256 sealing",
        "signed working paper",
        "receipt seals",
        "cryptographic evidence",
        "tamper-evident sha-256",
    ] {
        assert!(!public_docs.contains(claim), "{claim}");
    }
}

#[test]
fn test_the_readme_states_the_timestamp_grammar_the_gateway_enforces() {
    let readme = read("README.md");
    let grammar = readme
        .lines()
        .find(|line| line.starts_with("- Artefact timestamps"))
        .unwrap();
    for documented in ["`T`, `t`, or a space", "`Z`, `z`, or `+/-HH:MM` with optional `:SS`"] {
        assert!(grammar.contains(documented));
    }
    for refused in ["+10", "week date", "20260809T000000+0000"] {
        assert!(grammar.contains(refused));
    }
}

fn corpus() -> std::path::PathBuf {
    Path::new(COMPONENT).join("../../contracts/xero-trial-balance-v1")
}

#[test]
fn test_root_corpus_preserves_the_exporter_owned_bytes() {
    for (relative, expected) in [
        (
            "expected_results.json",
            "af6b1db093e47137330f3228dd5ce1c5d663f497fcc5e3ffd00493ecd829461d",
        ),
        (
            "fixtures/passing.csv",
            "2cbe9997a8e7210936ff3c59b5d3fdb0041c1b375b0f9c88cf9ee30d0f356a09",
        ),
        (
            "fixtures/failing_movement.csv",
            "702175df967b2854e7897cd27fdc4aca441e21b52438381108fabe88ff3153e4",
        ),
        (
            "fixtures/failing_ytd.csv",
            "ec757f12d13866360fbab189228ebb425893c6f8b299809c6f8567bf5817c64b",
        ),
    ] {
        let content = std::fs::read(corpus().join(relative)).unwrap();
        assert!(!content.windows(2).any(|w| w == b"\r\n"));
        assert_eq!(sha256_hex(&content), expected);
    }
}

#[test]
fn test_gateway_satisfies_the_pinned_exporter_contract() {
    let contract = parse(&std::fs::read_to_string(corpus().join("expected_results.json")).unwrap());
    assert_eq!(pyjson::canonical(contract.at("schema_version")), "2");
    assert_eq!(text(contract.at("corpus_id")), "xero-tb-csv.v1");
    assert_eq!(
        text(contract.at("owner_repository")),
        "https://github.com/ryanduguid/accounting-review-pipeline"
    );
    let Json::Array(columns) = contract.at("canonical_columns") else {
        panic!()
    };
    assert_eq!(columns.iter().map(text).collect::<Vec<_>>(), CANONICAL_COLUMNS);
    let Json::Array(scenarios) = contract.at("scenarios") else {
        panic!()
    };
    let dir = TempDir::new();
    let env = env(dir.path());
    for scenario in scenarios {
        let fixture = corpus().join("fixtures").join(text(scenario.at("fixture")));
        assert_eq!(
            sha256_hex(&std::fs::read(&fixture).unwrap()),
            text(scenario.at("sha256"))
        );
        let loaded = snapshot_file(&env, &Location::Disk(fixture), "source CSV").and_then(|s| load_tb_snapshot(&s));
        let expectation = scenario.at("conformance");
        if matches!(expectation.at("accept"), Json::Bool(true)) {
            // Tenant plus the stable AccountID is the control identity in every
            // consumer, and the account code's leading zero survives as text.
            let rows = loaded.unwrap();
            let tenant = "Catherby Fisheries Pty Ltd";
            let identity: Vec<(&str, &str)> = rows
                .iter()
                .map(|r| (r.tenant.as_str(), r.account_id.as_str()))
                .collect();
            assert_eq!(
                identity,
                [
                    (tenant, "00000000-0000-0000-0000-000000000001"),
                    (tenant, "00000000-0000-0000-0000-000000000002")
                ]
            );
            let codes: Vec<&str> = rows.iter().map(|r| r.account_code.as_str()).collect();
            assert_eq!(codes, ["090", "800"]);
        } else {
            assert_refused(loaded, &text(expectation.at("error_contains")));
        }
    }
}

#[test]
fn test_component_policy_points_only_to_the_canonical_contract() {
    let stale = "tests/conformance/xero_trial_balance_v1";
    let (ignore, contributing, readme) = (read(".gitignore"), read("CONTRIBUTING.md"), read("README.md"));
    assert!(!ignore.contains(&format!("!{stale}/fixtures/")));
    assert!(!contributing.contains(stale));
    assert!(!readme.contains(stale));
    for document in [&contributing, &readme] {
        assert!(document.contains("contracts/xero-trial-balance-v1/"));
    }
}
