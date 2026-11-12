// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

#![cfg(feature = "tools")]

use std::{ffi::OsStr, fs, path::Path, process::Command};

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/temp.rs"]
mod temp;

fn command(directory: &Path) -> Command {
    let mut command = child_profile::command(env!("CARGO_BIN_EXE_close-control-fuzz"));
    command.current_dir(directory);
    command
}

fn assert_refused(directory: &Path, arguments: &[&OsStr], diagnostic: &str) {
    let output = command(directory).args(arguments).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{arguments:?}");
    assert!(output.stdout.is_empty(), "{arguments:?}");
    assert_eq!(
        String::from_utf8(output.stderr)
            .unwrap()
            .replace("\r\n", "\n"),
        format!("close-control-fuzz: {diagnostic}\n"),
        "{arguments:?}",
    );
}

#[test]
fn corpus_replay_and_bounded_mutations_preserve_inputs() {
    let directory = temp::TempDir::new();
    let corpus = directory.join("corpus");
    let working = directory.join("working");
    fs::create_dir(&corpus).unwrap();
    fs::create_dir(&working).unwrap();
    let seeds: [(&str, &[u8]); 3] = [
        ("empty", b""),
        ("malformed", &[0xff, 0x00]),
        ("transactions.csv", b"account,amount\nfixture,12.50\n"),
    ];
    for (name, bytes) in seeds {
        fs::write(corpus.join(name), bytes).unwrap();
    }
    for seconds in ["0", "1"] {
        let output = command(&working)
            .arg(&corpus)
            .arg(seconds)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n");
        if seconds == "0" {
            assert_eq!(stdout, "replayed 3 seeds; 0 mutations; no panic\n");
        } else {
            let count = stdout
                .strip_prefix("replayed 3 seeds; ")
                .and_then(|s| s.strip_suffix(" mutations; no panic\n"))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            assert!(count > 0, "{stdout}");
        }
        for (name, bytes) in seeds {
            assert_eq!(fs::read(corpus.join(name)).unwrap(), bytes, "{name}");
        }
        assert_eq!(fs::read_dir(&corpus).unwrap().count(), seeds.len());
        assert!(fs::read_dir(&working).unwrap().next().is_none());
    }
}

#[test]
fn invalid_arguments_and_corpora_are_refused() {
    let directory = temp::TempDir::new();
    let missing = directory.join("missing");
    let empty = directory.join("empty");
    let nested = directory.join("nested");
    let oversized = directory.join("oversized");
    fs::create_dir(&empty).unwrap();
    fs::create_dir(&nested).unwrap();
    fs::create_dir(nested.join("subdirectory")).unwrap();
    fs::create_dir(&oversized).unwrap();
    fs::write(oversized.join("seed"), vec![b'x'; 4097]).unwrap();
    assert_refused(
        &directory.0,
        &[],
        "usage: close-control-fuzz CORPUS SECONDS (0..600)",
    );
    for (corpus, seconds, diagnostic) in [
        (&missing, "601", "seconds must be at most 600"),
        (&empty, "0", "corpus is empty"),
        (&nested, "0", "corpus entries must be files"),
        (&oversized, "0", "corpus seed exceeds 4096 bytes"),
    ] {
        assert_refused(
            &directory.0,
            &[corpus.as_os_str(), OsStr::new(seconds)],
            diagnostic,
        );
    }
}

#[test]
fn maximum_size_seed_can_be_replayed_without_mutations() {
    let directory = temp::TempDir::new();
    let corpus = directory.join("corpus");
    fs::create_dir(&corpus).unwrap();
    let bytes = vec![b'x'; 4096];
    fs::write(corpus.join("seed"), &bytes).unwrap();
    let output = command(&directory.0)
        .arg(&corpus)
        .arg("0")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "replayed 1 seeds; 0 mutations; no panic\n",
    );
    assert_eq!(fs::read(corpus.join("seed")).unwrap(), bytes);
    assert_eq!(fs::read_dir(&corpus).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
}
