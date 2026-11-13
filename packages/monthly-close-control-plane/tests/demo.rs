// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! `close-control demo`, which needs no checkout: its pack must be the one
//! `just demo` writes from the files in `examples/`.

#[path = "support/temp.rs"]
mod temp;

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use std::{collections::BTreeMap, fs, path::Path};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn files(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            assert!(entry.file_type().unwrap().is_file(), "{entry:?}");
            let name = entry.file_name().into_string().unwrap();
            (name, fs::read(entry.path()).unwrap())
        })
        .collect()
}

#[test]
fn the_demo_pack_is_the_just_demo_pack_byte_for_byte() {
    let temp = TempDir::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let example = |name: &str| examples.join(name);
    let expected = temp.join("just-demo");
    let review = child_profile::command(BIN)
        .arg("review")
        .arg("--current")
        .arg(example("current_trial_balance.csv"))
        .arg("--prior")
        .arg(example("prior_trial_balance.csv"))
        .arg("--mapping")
        .arg(example("account_mapping.csv"))
        .arg("--subledger")
        .arg(example("subledger_balances.csv"))
        .args(["--absolute-threshold", "10000"])
        .args(["--percentage-threshold", "0.10"])
        .args(["--reconciliation-tolerance", "0.01"])
        .arg("--review-note")
        .arg(example("review_note.json"))
        .arg("--output")
        .arg(&expected)
        .output()
        .unwrap();
    assert_eq!(review.status.code(), Some(2), "{review:?}");

    let demo_dir = temp.join("demo");
    let demo = child_profile::command(BIN)
        .arg("demo")
        .arg("--output")
        .arg(&demo_dir)
        .output()
        .unwrap();
    assert_eq!(demo.status.code(), Some(2), "{demo:?}");
    assert!(demo.stderr.is_empty(), "{demo:?}");
    let summary = String::from_utf8(demo.stdout).unwrap();
    assert!(summary.starts_with("close-control: REVIEW; "), "{summary}");

    let expected_files = files(&expected);
    assert!(!expected_files.is_empty());
    assert_eq!(files(&demo_dir.join("pack")), expected_files);
    for (name, bytes) in files(&demo_dir.join("inputs")) {
        assert_eq!(bytes, fs::read(example(&name)).unwrap(), "{name}");
    }
    assert_eq!(files(&demo_dir.join("inputs")).len(), 5);

    let view = child_profile::command(BIN)
        .arg("view")
        .arg("--pack-dir")
        .arg(demo_dir.join("pack"))
        .output()
        .unwrap();
    assert_eq!(view.status.code(), Some(0), "{view:?}");
}

#[test]
fn the_demo_writes_into_an_empty_directory_and_refuses_one_with_anything_in_it() {
    let temp = TempDir::new();
    let empty = temp.join("empty");
    fs::create_dir(&empty).unwrap();
    let demo = |output: &Path| {
        child_profile::command(BIN)
            .arg("demo")
            .arg("--output")
            .arg(output)
            .output()
            .unwrap()
    };
    assert_eq!(demo(&empty).status.code(), Some(2));
    assert!(empty.join("pack").is_dir());

    let occupied = temp.join("occupied");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("keep.txt"), "a reviewer's file").unwrap();
    let refused = demo(&occupied);
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert!(refused.stdout.is_empty());
    let message = String::from_utf8(refused.stderr).unwrap();
    assert!(
        message.starts_with("close-control demo: ")
            && message.contains("is not an empty directory"),
        "{message}"
    );
    assert_eq!(fs::read_dir(&occupied).unwrap().count(), 1);

    let file = temp.join("a-file");
    fs::write(&file, "not a directory").unwrap();
    assert_eq!(demo(&file).status.code(), Some(1));
    assert_eq!(fs::read_to_string(&file).unwrap(), "not a directory");
}

#[test]
fn the_demo_refuses_an_output_inside_a_checkout() {
    let temp = TempDir::new();
    fs::create_dir(temp.join(".git")).unwrap();
    let inside = temp.join("demo");
    let refused = child_profile::command(BIN)
        .arg("demo")
        .arg("--output")
        .arg(&inside)
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let message = String::from_utf8(refused.stderr).unwrap();
    assert!(
        message.contains("inside the version-control checkout"),
        "{message}"
    );
    assert!(!inside.exists());
}
