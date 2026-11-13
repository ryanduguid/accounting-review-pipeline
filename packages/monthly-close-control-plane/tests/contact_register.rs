// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! `contact_register`: duplicate contacts, ABNs that fail their check digits
//! and one ABN across several contacts, from a fabricated contacts export.

#[path = "support/temp.rs"]
mod temp;

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::{
    pyjson::{self, Json},
    viewer,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

/// A close on which every control that runs passes.
fn trial_balance(temp: &TempDir, name: &str, date: &str) -> PathBuf {
    let path = temp.join(name);
    let csv = format!(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
         {date},Fabricated Varrock,Assets,A1,Bank,090,0.00,0.00,100.00,0.00\n\
         {date},Fabricated Varrock,Equity,E1,Capital,300,0.00,0.00,0.00,100.00\n"
    );
    fs::write(&path, csv).unwrap();
    path
}

fn review(
    temp: &TempDir,
    output: &str,
    contacts: Option<&str>,
    extra: &[&str],
) -> (Output, PathBuf) {
    let output = temp.join(output);
    let mut command = child_profile::command(BIN);
    command
        .arg("review")
        .arg("--current")
        .arg(trial_balance(temp, "current.csv", "2026-08-31"))
        .arg("--prior")
        .arg(trial_balance(temp, "prior.csv", "2026-07-31"))
        .args(extra)
        .arg("--output")
        .arg(&output);
    if let Some(csv) = contacts {
        let path = temp.join("contacts.csv");
        fs::write(&path, csv).unwrap();
        command.arg("--contacts").arg(path);
    }
    (command.output().unwrap(), output)
}

fn pack(output: &Path) -> Json {
    pyjson::loads(&fs::read_to_string(output.join("close-review-pack.json")).unwrap()).unwrap()
}

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}

fn reasons(document: &Json) -> Vec<String> {
    document
        .at("exceptions")
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row.at("control").is_str("contact_register"))
        .map(|row| {
            assert!(row.at("status").is_str("REVIEW"));
            assert!(row.at("tenant").is_str("Fabricated Varrock"));
            assert!(row.at("account_id").is_str(""));
            text(row.at("reason"))
        })
        .collect()
}

fn strings(document: &Json, member: &str) -> Vec<String> {
    document
        .at(member)
        .as_array()
        .unwrap()
        .iter()
        .map(text)
        .collect()
}

fn state(document: &Json) -> String {
    text(document.at("required_controls").as_array().unwrap()[0].at("state"))
}

/// Fabricated contacts. `80 000 000 001` and `61 000 000 002` pass the ABN
/// check digits; `80 000 000 002` does not.
const CONTACTS: &str = "ContactID,Name,ABN\n\
    C1,Fabricated Lumbridge Supplies Pty Ltd,80 000 000 001\n\
    C2,Fabricated Draynor Hauling,61000000002\n\
    C3,fabricated  lumbridge supplies PTY LTD,\n\
    C4,Fabricated Varrock Smithing,80000000001\n\
    C5,Fabricated Falador Masonry,80 000 000 002\n\
    C6,Fabricated Edgeville Traders,ABN pending\n\
    C7,Fabricated Karamja Imports,\n";

#[test]
fn duplicate_contacts_invalid_abns_and_a_shared_abn_are_each_raised() {
    let temp = TempDir::new();
    let (result, output) = review(&temp, "pack", Some(CONTACTS), &[]);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let document = pack(&output);
    assert!(document.at("overall_status").is_str("REVIEW"));
    assert_eq!(
        reasons(&document),
        [
            "2 contacts have the same name once case and spacing are ignored: 'C1' ('Fabricated Lumbridge Supplies Pty Ltd'), 'C3' ('fabricated  lumbridge supplies PTY LTD').",
            "ABN 80000000001 is recorded on 2 contacts: 'C1' ('Fabricated Lumbridge Supplies Pty Ltd'), 'C4' ('Fabricated Varrock Smithing').",
            "Contact 'C5' ('Fabricated Falador Masonry') has ABN '80 000 000 002', which fails the ABN check digits, so it is not a valid ABN.",
            "Contact 'C6' ('Fabricated Edgeville Traders') has ABN 'ABN pending', which is not 11 digits, so it is not a valid ABN.",
        ]
    );
    assert!(document.at("client_queries").as_array().unwrap().is_empty());
    assert!(document.at("source_sha256").get("contacts").is_some());
    assert!(
        strings(&document, "controls_not_run")
            .iter()
            .all(|name| name != "contact_register")
    );
    assert!(viewer::verify_pack(&output).is_ok());
}

#[test]
fn all_whitespace_is_ignored_in_names_and_abns() {
    // A name split by a space matches the same name without one, and an ABN
    // separated by non-breaking spaces is read as its 11 digits.
    let temp = TempDir::new();
    let csv = "ContactID,Name,ABN\nC1,Fabricated Acme Pty,80\u{a0}000\u{a0}000\u{a0}001\n\
               C2,Fabricated AcmePty,\n";
    let (result, output) = review(&temp, "pack", Some(csv), &[]);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert_eq!(
        reasons(&pack(&output)),
        [
            "2 contacts have the same name once case and spacing are ignored: 'C1' ('Fabricated Acme Pty'), 'C2' ('Fabricated AcmePty')."
        ]
    );
}

#[test]
fn a_clean_contacts_export_raises_nothing_and_counts_as_run() {
    let temp = TempDir::new();
    let clean = "ContactID,Name,ABN\nC1,Fabricated Lumbridge Supplies,80 000 000 001\n\
                 C2,Fabricated Draynor Hauling,61 000 000 002\nC7,Fabricated Karamja Imports,\n";
    let (result, output) = review(
        &temp,
        "pack",
        Some(clean),
        &["--require-control", "contact_register"],
    );
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let document = pack(&output);
    assert!(document.at("overall_status").is_str("PASS"));
    assert_eq!(state(&document), "ran");
    assert!(viewer::verify_pack(&output).is_ok());
}

#[test]
fn a_required_register_with_no_contacts_is_skipped_or_failed() {
    let temp = TempDir::new();
    let require = ["--require-control", "contact_register"];
    let (result, output) = review(&temp, "absent", None, &require);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let document = pack(&output);
    assert_eq!(state(&document), "skipped");
    assert!(document.at("source_sha256").get("contacts").is_none());
    assert!(
        strings(&document, "controls_not_run")
            .iter()
            .all(|name| name != "contact_register")
    );

    let (result, output) = review(&temp, "empty", Some("ContactID,Name,ABN\n"), &require);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let document = pack(&output);
    assert_eq!(state(&document), "failed");
    assert!(strings(&document, "controls_not_run").contains(&"contact_register".to_owned()));
    assert!(viewer::verify_pack(&output).is_ok());
}

#[test]
fn a_repeated_contact_id_or_blank_name_is_refused_before_any_pack_is_written() {
    let temp = TempDir::new();
    for (name, csv, message) in [
        (
            "repeated",
            "ContactID,Name,ABN\nC1,Fabricated A,\nC1,Fabricated B,\n",
            "duplicate ContactID 'C1'",
        ),
        ("blank", "ContactID,Name,ABN\nC1,,\n", "has an empty Name"),
        (
            "columns",
            "ContactID,Name\nC1,Fabricated A\n",
            "canonical schema mismatch (missing ABN)",
        ),
    ] {
        let (result, output) = review(&temp, name, Some(csv), &[]);
        assert_eq!(result.status.code(), Some(1), "{name}: {result:?}");
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains(message), "{name}: {error}");
        assert!(!output.exists(), "{name}");
    }
}
