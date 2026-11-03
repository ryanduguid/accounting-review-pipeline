//! Committed fixtures and test inputs may not carry a checksum-valid
//! Australian identifier unless `synthetic-identifiers.txt` lists it for that
//! file. The screen runs evatt's own detectors and check digits over the
//! tracked files `is_fixture` selects anywhere in the repository. Findings name
//! the file, line and kind, never the value, so a CI log never repeats one.
use evatt::{patterns, pytext, redact::Lines};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The allowlist, from the repository root. It names the values it allows,
/// so it is not screened itself.
const ALLOWLIST: &str = "packages/evatt/tests/synthetic-identifiers.txt";

/// Directory names whose every file is fabricated input or expected results.
const FIXTURE_DIRS: [&str; 6] = ["corpus", "evaluation", "examples", "fixtures", "samples", "seeds"];

/// Directory names whose data files are test inputs: data beside the test
/// code, the shared contract and Elizabeth Anne Alexander's demo policy.
/// Other `data` directories hold vendored files such as the CA bundle.
const INPUT_DIRS: [&str; 3] = ["contracts", "policy", "tests"];
const DATA_EXTENSIONS: [&str; 3] = ["csv", "json", "txt"];

/// Kinds with a check digit. BSB, email and phone have none, so a match
/// cannot separate a real value from a fabricated one.
const CHECKED: [&str; 4] = ["tfn", "abn", "acn", "medicare"];

/// Each finding's line, kind and digits.
type Findings = BTreeSet<(usize, String, String)>;

/// An allowlist entry's path, kind and digits.
type Entry = (String, String, String);

fn is_fixture(path: &str) -> bool {
    let mut dirs: Vec<_> = path.split('/').collect();
    let name = dirs.pop().unwrap_or_default();
    let data = name.rsplit_once('.').is_some_and(|(_, e)| DATA_EXTENSIONS.contains(&e));
    path != ALLOWLIST
        && (dirs.iter().any(|d| FIXTURE_DIRS.contains(d))
            || dirs.windows(2).any(|w| w == ["tests", "data"])
            || data && dirs.iter().any(|d| INPUT_DIRS.contains(d)))
}

#[test]
fn every_file_under_tests_data_is_screened_whatever_its_extension() {
    for path in [
        "packages/p/tests/data/notes.md",
        "packages/p/tests/data/sub/record",
        "tools/tests/data/case.yaml",
    ] {
        assert!(is_fixture(path), "{path} must be screened");
    }
    assert!(is_fixture("contracts/c/schema.csv"));
    for path in [
        "packages/p/tests/screen.rs",
        "packages/p/src/data/table.json",
        ALLOWLIST,
    ] {
        assert!(!is_fixture(path), "{path} must not be screened");
    }
}

/// Each checksum-valid identifier evatt detects.
fn identifiers(text: &str) -> patterns::Result<Findings> {
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

fn findings(path: &str, bytes: &[u8]) -> Result<Findings, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("{path}: not UTF-8 text, so it cannot be screened: {error}"))?;
    identifiers(text).map_err(|error| format!("{path}: {error}"))
}

/// Each entry's path, kind and digits with its line number. A malformed line
/// is reported by number alone, so a CI log never repeats its digits.
fn allowlist(text: &str) -> (HashMap<Entry, usize>, Vec<String>) {
    let mut entries = HashMap::new();
    let mut malformed = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.splitn(4, ' ').collect::<Vec<_>>()[..] {
            [path, kind, digits, reason]
                if CHECKED.contains(&kind)
                    && !digits.is_empty()
                    && digits.bytes().all(|b| b.is_ascii_digit())
                    && !reason.trim().is_empty() =>
            {
                entries.insert((path.into(), kind.into(), digits.into()), index + 1);
            }
            _ => malformed.push(format!("{ALLOWLIST}:{}: expected 'path kind digits reason'", index + 1)),
        }
    }
    (entries, malformed)
}

/// Every failure: findings no entry names for their file, files that could
/// not be screened, and malformed or stale entries.
fn verdict(results: Vec<(String, Result<Findings, String>)>, allowlist_text: &str) -> Vec<String> {
    let (entries, mut failures) = allowlist(allowlist_text);
    let mut used = HashSet::new();
    for (path, result) in results {
        match result {
            Err(error) => failures.push(error),
            Ok(found) => {
                for (line, kind, digits) in found {
                    let key = (path.clone(), kind, digits);
                    if entries.contains_key(&key) {
                        used.insert(key);
                    } else {
                        failures.push(format!("{path}:{line}: checksum-valid {}", key.1));
                    }
                }
            }
        }
    }
    let mut stale: Vec<_> = entries
        .iter()
        .filter(|(key, _)| !used.contains(*key))
        .map(|(_, &n)| n)
        .collect();
    stale.sort_unstable();
    failures.extend(
        stale
            .iter()
            .map(|n| format!("{ALLOWLIST}:{n}: entry matches no finding")),
    );
    failures
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
    // One thread per file: in a debug build the largest recordings take most of a minute each.
    let results = std::thread::scope(|scope| {
        let workers: Vec<_> = paths
            .iter()
            .map(|path| {
                let root = &root;
                scope.spawn(move || {
                    let bytes = std::fs::read(root.join(path)).map_err(|error| format!("{path}: unreadable: {error}"));
                    (path.clone(), bytes.and_then(|bytes| findings(path, &bytes)))
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().expect("screen thread")).collect()
    });
    let failures = verdict(results, include_str!("synthetic-identifiers.txt"));
    assert!(
        failures.is_empty(),
        "{} failures in {} screened files. Replace a real value, or list a synthetic one for its file in \
         {ALLOWLIST}:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n")
    );
}

fn check(path: &str, bytes: &[u8], allowlist_text: &str) -> Vec<String> {
    verdict(vec![(path.into(), findings(path, bytes))], allowlist_text)
}

#[test]
fn a_planted_checksum_valid_tfn_fails_and_an_invalid_one_passes() {
    let planted = b"Client: Sample Holdings Pty Ltd\nTFN: 123 456 782\n";
    assert_eq!(check("p.md", planted, ""), ["p.md:2: checksum-valid tfn"]);
    assert_eq!(
        check("p.csv", b"id,tfn,code\n1,123456782,7\n", ""),
        ["p.csv:2: checksum-valid tfn"]
    );
    // A value that wraps onto a list-like line is triage for redact, and still a finding here.
    assert_eq!(
        check("p.md", b"TFN 123 456\n782. Next\n", ""),
        ["p.md:1: checksum-valid tfn"]
    );
    // So is a wrapped Medicare number with its individual reference number.
    assert_eq!(
        check("p.md", b"Medicare: 2123 4567\n0. 1/1\n", ""),
        ["p.md:1: checksum-valid medicare"]
    );
    assert!(check("p.md", b"TFN: 123 456 789\n", "").is_empty());
    assert!(check("p.csv", b"id,tfn,code\n1,123456789,7\n", "").is_empty());
    assert_eq!(check("p.csv", b"TFN \xff", "").len(), 1);
}

#[test]
fn an_entry_allows_its_value_in_its_own_file_only_and_must_match() {
    let planted = b"TFN: 123 456 782\n";
    let entry = "p.md tfn 123456782 documented test vector\n";
    assert!(check("p.md", planted, entry).is_empty());
    let stale = format!("{ALLOWLIST}:1: entry matches no finding");
    // The same value in another file is a finding, and the unused entry is stale.
    assert_eq!(
        check("q.md", planted, entry),
        ["q.md:1: checksum-valid tfn".to_string(), stale.clone()]
    );
    assert_eq!(check("p.md", b"no identifiers\n", entry), [stale]);
    let malformed = check("p.md", planted, "# header\np.md tfn 123456782\n");
    assert_eq!(
        malformed,
        [
            format!("{ALLOWLIST}:2: expected 'path kind digits reason'"),
            "p.md:1: checksum-valid tfn".to_string(),
        ]
    );
    assert!(!malformed[0].contains("123456782"));
}
