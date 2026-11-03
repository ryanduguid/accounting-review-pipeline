//! Committed fixtures and evaluation data may not carry a checksum-valid
//! Australian identifier unless `synthetic-identifiers.txt` documents it. The
//! screen runs evatt's own detectors and check digits over every tracked file
//! under a fixture directory anywhere in the repository. Findings name the
//! file, line and kind, never the value, so a CI log never repeats one.
use evatt::{patterns, pytext, redact::Lines};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Directory names that hold fabricated inputs and expected results across
/// the components, plus `tests/data`. Other `data` directories hold vendored
/// files such as the exporter's CA bundle.
const FIXTURE_DIRS: [&str; 6] = ["corpus", "evaluation", "examples", "fixtures", "samples", "seeds"];

/// Kinds with a check digit. BSB, email and phone have none, so a match
/// cannot separate a real value from a fabricated one.
const CHECKED: [&str; 4] = ["tfn", "abn", "acn", "medicare"];

fn is_fixture(path: &str) -> bool {
    let mut dirs: Vec<_> = path.split('/').collect();
    dirs.pop();
    dirs.iter().any(|d| FIXTURE_DIRS.contains(d)) || dirs.windows(2).any(|w| w == ["tests", "data"])
}

/// The line, kind and digits of each checksum-valid identifier evatt detects.
fn identifiers(text: &str) -> patterns::Result<BTreeSet<(usize, String, String)>> {
    // evatt skips a bare number that a comma joins to another digit, as part of
    // a longer number. That hides a CSV or JSON value beside a numeric field,
    // so the screen reads each comma as a field break.
    let text = pytext::nfc(&text.replace("\r\n", "\n").replace(',', ";"));
    let lines = Lines::new(&text);
    let mut found = BTreeSet::new();
    for span in patterns::structured_spans(&text)? {
        if !CHECKED.contains(&span.kind.as_str()) {
            continue;
        }
        // A Medicare span can carry the individual reference number after its 10 checked digits.
        let width = if span.kind == "medicare" { 10 } else { usize::MAX };
        let digits: String = patterns::digits(&span.value).chars().take(width).collect();
        if patterns::valid(&span.kind, &digits)? {
            found.insert((lines.locate(span.start).0, span.kind, digits));
        }
    }
    // structured_spans leaves a number that runs into a list marker for triage.
    for a in patterns::ambiguous(&text)? {
        let digits = patterns::digits(&a.value);
        for kind in CHECKED {
            if patterns::valid(kind, &digits)? {
                found.insert((lines.locate(a.digit_start).0, kind.into(), digits));
                break;
            }
        }
    }
    Ok(found)
}

fn screen(path: &str, bytes: &[u8], allowed: &HashSet<String>) -> Vec<String> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => return vec![format!("{path}: not UTF-8 text, so it cannot be screened: {error}")],
    };
    match identifiers(text) {
        Ok(found) => found
            .into_iter()
            .filter(|(_, _, digits)| !allowed.contains(digits))
            .map(|(line, kind, _)| format!("{path}:{line}: checksum-valid {kind}"))
            .collect(),
        Err(error) => vec![format!("{path}: {error}")],
    }
}

fn allowlist() -> HashSet<String> {
    include_str!("synthetic-identifiers.txt")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (digits, reason) = line.split_once(' ').unwrap_or((line, ""));
            assert!(
                !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && !reason.trim().is_empty(),
                "an allowlist entry is digits, a space and a reason: {line}"
            );
            digits.to_string()
        })
        .collect()
}

fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "git {args:?} failed: {stderr}");
    output.stdout
}

fn tracked_fixtures() -> (PathBuf, Vec<String>) {
    let top = git(Path::new(env!("CARGO_MANIFEST_DIR")), &["rev-parse", "--show-toplevel"]);
    let root = PathBuf::from(String::from_utf8(top).expect("UTF-8 checkout path").trim_end());
    let paths = git(&root, &["ls-files", "-z"])
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8(p.to_vec()).expect("tracked paths are UTF-8"))
        .filter(|p| is_fixture(p))
        .collect();
    (root, paths)
}

#[test]
fn committed_fixtures_carry_no_undocumented_identifier() {
    let (root, paths) = tracked_fixtures();
    assert!(
        !paths.is_empty(),
        "no tracked fixture files found under {}",
        root.display()
    );
    let allowed = allowlist();
    // One thread per file: in a debug build the largest recordings take most of a minute each.
    let failures: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = paths
            .iter()
            .map(|path| {
                let (root, allowed) = (&root, &allowed);
                scope.spawn(move || match std::fs::read(root.join(path)) {
                    Ok(bytes) => screen(path, &bytes, allowed),
                    Err(error) => vec![format!("{path}: unreadable: {error}")],
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("screen thread"))
            .collect()
    });
    assert!(
        failures.is_empty(),
        "{} findings in {} screened fixture files. Replace a real value, or document a synthetic one in \
         packages/evatt/tests/synthetic-identifiers.txt:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n")
    );
}

#[test]
fn a_planted_checksum_valid_tfn_fails_and_an_invalid_one_passes() {
    let none = HashSet::new();
    let planted = "Client: Sample Holdings Pty Ltd\nTFN: 123 456 782\n";
    assert_eq!(
        screen("p.md", planted.as_bytes(), &none),
        ["p.md:2: checksum-valid tfn"]
    );
    assert_eq!(
        screen("p.csv", b"id,tfn,code\n1,123456782,7\n", &none),
        ["p.csv:2: checksum-valid tfn"]
    );
    // A value that wraps onto a list-like line is triage for redact, and still a finding here.
    assert_eq!(
        screen("p.md", b"TFN 123 456\n782. Next\n", &none),
        ["p.md:1: checksum-valid tfn"]
    );
    assert!(screen("p.md", b"TFN: 123 456 789\n", &none).is_empty());
    assert!(screen("p.csv", b"id,tfn,code\n1,123456789,7\n", &none).is_empty());
    assert!(screen("p.md", planted.as_bytes(), &allowlist()).is_empty());
    assert_eq!(screen("p.csv", b"TFN \xff", &none).len(), 1);
}
