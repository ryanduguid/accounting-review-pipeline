// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Real binary and real local git; all documents and maps are fabricated.
#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

#[cfg(windows)]
#[path = "../../../tests/support/windows_console.rs"]
mod windows_console;

use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

const MAP: &str = r#"{"schema_version":1,"entries":[{"value":"Jane Roe","placeholder":"PERSON_01","kind":"person","added":"2026-10-13"}]}"#;
const GIT_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];
/// The system temp directory, resolved as pytest's `tmp_path` is. Windows
/// runners spell TEMP with an 8.3 short name (`RUNNER~1`), which git does not
/// match against the resolved working directory in `GIT_CEILING_DIRECTORIES`.
fn resolved_temp_dir() -> PathBuf {
    let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    match temp.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => temp,
    }
}

struct Repo {
    root: PathBuf,
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl Repo {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = resolved_temp_dir().join(format!(
            "evatt-rust-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let repo = Self { root };
        repo.git(&["init", "-q"]);
        repo.put(".gitignore", b"entities.json\n*.triage.md\n*.disclosure.json\n");
        repo.put("entities.json", MAP.as_bytes());
        repo.put("in.md", b"Jane Roe\nTFN: 123456782\n");
        repo
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    fn put(&self, name: &str, data: &[u8]) {
        std::fs::write(self.path(name), data).unwrap();
    }
    fn get(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.path(name)).unwrap()
    }
    fn configured(&self, mut c: Command) -> Command {
        c.current_dir(&self.root)
            .env("GIT_CONFIG_GLOBAL", self.path("absent-global"))
            .env("GIT_CONFIG_SYSTEM", self.path("absent-system"))
            .env("GIT_CEILING_DIRECTORIES", self.root.parent().unwrap())
            .env("PYTHON_COLORS", "0");
        for name in GIT_ENV {
            c.env_remove(name);
        }
        c
    }
    fn git(&self, args: &[&str]) {
        assert!(
            self.configured(Command::new("git"))
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut c = self.configured(child_profile::command(env!("CARGO_BIN_EXE_evatt")));
        c.args(args);
        c
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn normal(&self, name: &'static str) -> Vec<&'static str> {
        let mut args = vec![name, "--in", "in.md", "--map", "entities.json"];
        if name != "verify" {
            args.extend(["--out", "out.md"]);
        }
        args
    }
    fn disclosure(&self, name: &str) -> Output {
        self.run(&[
            name,
            "--in",
            "in.md",
            "--map",
            "entities.json",
            "--destination",
            "model:sample",
            "--decision-ref",
            "sample:42",
            if name == "disclosure-record" {
                "--out"
            } else {
                "--record"
            },
            "out.disclosure.json",
        ])
    }
}
fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}
fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// Compare raw stream output with CPython's native text-mode line endings.
fn native_output(text: &str) -> String {
    if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    }
}

#[test]
fn command_help_wraps_without_requiring_or_changing_workpapers() {
    let repo = Repo::new();
    let original = repo.get("in.md");
    for command in [
        None,
        Some("redact"),
        Some("restore"),
        Some("verify"),
        Some("disclosure-record"),
        Some("disclosure-check"),
    ] {
        let mut args = command.into_iter().collect::<Vec<_>>();
        if command.is_some() {
            args.extend(["--in", "absent-input.md", "--map", "absent-map.json"]);
        }
        args.push("--help");
        let mut rendered = Vec::new();
        for width in ["20", "80"] {
            let output = repo
                .command(&args)
                .env("COLUMNS", width)
                .env("LINES", "24")
                .output()
                .unwrap();
            assert_eq!(code(&output), 0, "{args:?}: {}", stderr(&output));
            assert!(output.stderr.is_empty());
            let help = String::from_utf8(output.stdout).unwrap();
            assert!(help.starts_with("usage: evatt"), "{help}");
            assert!(help.contains("--help"));
            if let Some(command) = command {
                assert!(help.contains(command));
                assert!(help.contains("--in") && help.contains("--map"));
            } else {
                for command in ["redact", "restore", "verify", "disclosure-record", "disclosure-check"] {
                    assert!(help.contains(command));
                }
            }
            rendered.push(help);
        }
        assert_ne!(rendered[0], rendered[1], "{args:?} did not respect COLUMNS");
    }
    assert_eq!(repo.get("in.md"), original);
    assert_eq!(repo.get("entities.json"), MAP.as_bytes());
    assert!(!repo.path("out.md").exists());
    assert!(!repo.path("out.md.manifest.json").exists());
}

#[cfg(windows)]
#[test]
fn real_console_uses_native_width_and_colours_on_both_streams() {
    windows_console::check(
        "real_console_uses_native_width_and_colours_on_both_streams",
        env!("CARGO_BIN_EXE_evatt"),
        include_str!("fixtures/cli/main-help80.txt"),
        include_str!("fixtures/cli/main-required80.txt"),
        1,
    );
}

#[test]
fn help_colour_respects_python_and_terminal_environment_precedence() {
    let repo = Repo::new();
    let cases: &[(&[(&str, &str)], bool)] = &[
        (&[("PYTHON_COLORS", "1"), ("NO_COLOR", "1"), ("TERM", "dumb")], true),
        (&[("PYTHON_COLORS", "0"), ("FORCE_COLOR", "1")], false),
        (&[("NO_COLOR", "1"), ("FORCE_COLOR", "1")], false),
        (&[("NO_COLOR", ""), ("FORCE_COLOR", "1"), ("TERM", "dumb")], true),
        (&[("FORCE_COLOR", ""), ("TERM", "dumb")], false),
        (&[("TERM", "xterm")], false),
    ];
    for (variables, expected_colour) in cases {
        let mut command = repo.command(&["--help"]);
        for name in ["PYTHON_COLORS", "NO_COLOR", "FORCE_COLOR", "TERM"] {
            command.env_remove(name);
        }
        for (name, value) in *variables {
            command.env(name, value);
        }
        let output = command.output().unwrap();
        assert_eq!(code(&output), 0);
        assert!(output.stderr.is_empty());
        assert_eq!(
            output.stdout.windows(2).any(|pair| pair == b"\x1b["),
            *expected_colour,
            "{variables:?}"
        );
    }
}

#[test]
fn abbreviated_and_explicit_arguments_preserve_redaction_and_usage_errors() {
    let repo = Repo::new();
    for arguments in [
        ["redact", "--i", "in.md", "--m", "entities.json", "--o", "out.md"].as_slice(),
        ["redact", "--in=in.md", "--map=entities.json", "--out=out.md"].as_slice(),
    ] {
        let output = repo.run(arguments);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        assert_eq!(repo.get("out.md"), b"PERSON_01\nTFN: TFN_01\n");
        assert!(repo.path("out.md.manifest.json").is_file());
    }
    for arguments in [
        ["--vers=x"].as_slice(),
        ["--hel=x"].as_slice(),
        ["-h=x"].as_slice(),
        ["--", "redact"].as_slice(),
        ["redact", "--help=x"].as_slice(),
    ] {
        let output = repo.run(arguments);
        assert_eq!(code(&output), 1, "{arguments:?}");
        assert!(output.stdout.is_empty());
        assert!(stderr(&output).contains("error:"), "{arguments:?}: {}", stderr(&output));
    }
    let attached_help = repo.run(&["-hfoo"]);
    assert_eq!(code(&attached_help), 0);
    assert!(attached_help.stderr.is_empty());
    assert_eq!(attached_help.stdout, repo.run(&["--help"]).stdout);
    assert_eq!(repo.get("in.md"), b"Jane Roe\nTFN: 123456782\n");
    assert_eq!(repo.get("entities.json"), MAP.as_bytes());
}

#[test]
fn clean_halt_and_round_trip_keep_manifest_and_leftovers() {
    let repo = Repo::new();
    assert_eq!(code(&repo.run(&repo.normal("redact"))), 0);
    assert_eq!(repo.get("out.md"), b"PERSON_01\nTFN: TFN_01\n");
    assert!(repo.path("out.md.manifest.json").is_file());
    repo.put("in.md", &repo.get("out.md"));
    assert_eq!(code(&repo.run(&repo.normal("verify"))), 0);
    assert_eq!(code(&repo.run(&repo.normal("restore"))), 0);
    assert_eq!(repo.get("out.md"), b"Jane Roe\nTFN: TFN_01\n");
    repo.put("in.md", b"jane@example.com\nJohn Smith\n");
    assert_eq!(code(&repo.run(&repo.normal("redact"))), 2);
    assert!(!repo.path("out.md").exists());
    assert!(!repo.path("out.md.manifest.json").exists());
    let triage = String::from_utf8(repo.get("out.md.triage.md")).unwrap();
    assert!(triage.contains("**John Smith** (name, line 2)"));
    assert!(!triage.contains("jane@example.com"));
    repo.put("in.md", b"Jane Roe\r\n");
    assert_eq!(code(&repo.run(&repo.normal("redact"))), 0);
    assert_eq!(repo.get("out.md"), b"PERSON_01\r\n");
    assert!(!repo.path("out.md.triage.md").exists());
}

#[test]
fn filesystem_failures_and_unguarded_triage_clear_only_required_stale_files() {
    let repo = Repo::new();
    repo.put(".gitignore", b"entities.json\n");
    assert_eq!(code(&repo.run(&repo.normal("redact"))), 0);
    repo.put("in.md", b"John Smith");
    let output = repo.run(&repo.normal("redact"));
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("the triage file must never be committed"));
    assert!(!repo.path("out.md").exists());
    assert!(!repo.path("out.md.manifest.json").exists());
    assert!(!repo.path("out.md.triage.md").exists());
    std::fs::create_dir(repo.path("out.md")).unwrap();
    assert_eq!(code(&repo.run(&repo.normal("redact"))), 1);
    assert!(!repo.path("out.md.triage.md").exists());
    std::fs::remove_dir(repo.path("out.md")).unwrap();
    repo.put("in.md", b"Jane Roe");
    std::fs::create_dir(repo.path("out.md.manifest.json")).unwrap();
    assert_eq!(code(&repo.run(&repo.normal("redact"))), 1);
    assert!(!repo.path("out.md").exists());
    repo.put("blocked", b"not a directory");
    assert_eq!(
        code(&repo.run(&[
            "restore",
            "--in",
            "in.md",
            "--map",
            "entities.json",
            "--out",
            "blocked/out.md"
        ])),
        1
    );
}

#[test]
fn git_precedence_tracking_excludes_and_six_overrides_are_preserved() {
    let repo = Repo::new();
    repo.put(".gitignore", b"entities.json\n!entities.json\n");
    assert_eq!(code(&repo.run(&repo.normal("verify"))), 1);
    repo.put(".gitignore", b"*.triage.md\n");
    repo.put(".git/info/exclude", b"entities.json\n");
    assert_eq!(code(&repo.run(&repo.normal("redact"))), 0);
    let mut command = repo.command(&repo.normal("verify"));
    for name in GIT_ENV {
        command.env(name, repo.path("absent"));
    }
    assert_eq!(code(&command.output().unwrap()), 2);
    repo.git(&["add", "-f", "entities.json"]);
    let output = repo.run(&repo.normal("verify"));
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("git already tracks it"));
}

#[test]
fn collisions_include_hardlinks_and_derived_paths_before_any_mutation() {
    for name in ["entities.json", "in.md"] {
        let repo = Repo::new();
        std::fs::hard_link(repo.path(name), repo.path("out.md")).unwrap();
        let before = repo.get(name);
        assert_eq!(code(&repo.run(&repo.normal("redact"))), 1);
        assert_eq!(repo.get(name), before);
    }
    for name in ["out.md.manifest.json", "out.md.triage.md"] {
        let repo = Repo::new();
        repo.put("out.md", b"fabricated previous output");
        std::fs::hard_link(repo.path("out.md"), repo.path(name)).unwrap();
        assert_eq!(code(&repo.run(&repo.normal("redact"))), 1);
        assert_eq!(repo.get("out.md"), b"fabricated previous output");
    }
}

#[test]
fn invalid_utf8_input_and_maps_preserve_every_existing_artifact() {
    let cases: &[(&str, &[u8], &str)] = &[
        ("in.md", b"\xff", "byte 0xff in position 0: invalid start byte"),
        (
            "in.md",
            b"\xe2\x28\xa1",
            "byte 0xe2 in position 0: invalid continuation byte",
        ),
        ("in.md", b"\xe2", "byte 0xe2 in position 0: unexpected end of data"),
        ("in.md", b"\xe2\x82", "bytes in position 0-1: unexpected end of data"),
        (
            "entities.json",
            b"\xe2\x82",
            "bytes in position 0-1: unexpected end of data",
        ),
    ];
    for &(path, bytes, diagnostic) in cases {
        let repo = Repo::new();
        for name in ["out.md", "out.md.manifest.json", "out.md.triage.md"] {
            repo.put(name, name.as_bytes());
        }
        repo.put(path, bytes);
        let before = [
            "in.md",
            "entities.json",
            "out.md",
            "out.md.manifest.json",
            "out.md.triage.md",
        ]
        .map(|name| (name, repo.get(name)));
        let output = repo.run(&repo.normal("redact"));
        assert_eq!(code(&output), 1, "{path}: {diagnostic}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            stderr(&output),
            native_output(&format!("error: 'utf-8' codec can't decode {diagnostic}\n"))
        );
        for (name, expected) in before {
            assert_eq!(repo.get(name), expected, "{name}");
        }
    }
}

#[test]
fn malformed_entity_dates_and_kinds_preserve_input_and_existing_output() {
    for (old, value, diagnostic) in [
        (
            "\"2026-10-13\"",
            "null",
            "entity added date must be YYYY-MM-DD, got None",
        ),
        (
            "\"2026-10-13\"",
            "\"2026-1x-13\"",
            "entity added date must be YYYY-MM-DD, got '2026-1x-13'",
        ),
        (
            "\"2026-10-13\"",
            "{\"sample\":[null,false,1]}",
            "entity added date must be YYYY-MM-DD, got {'sample': [None, False, 1]}",
        ),
        (
            "\"person\"",
            "{\"sample\":[null,false,1]}",
            "unknown entity kind {'sample': [None, False, 1]}",
        ),
    ] {
        let repo = Repo::new();
        let map = MAP.replace(old, value);
        repo.put("entities.json", map.as_bytes());
        repo.put("out.md", b"fabricated previous output\r\n");
        let input = repo.get("in.md");
        let output = repo.run(&repo.normal("restore"));
        assert_eq!(code(&output), 1);
        assert!(output.stdout.is_empty());
        assert_eq!(stderr(&output), native_output(&format!("error: {diagnostic}\n")));
        assert_eq!(repo.get("in.md"), input);
        assert_eq!(repo.get("entities.json"), map.as_bytes());
        assert_eq!(repo.get("out.md"), b"fabricated previous output\r\n");
    }
}

#[test]
fn restore_creates_missing_output_parents_and_preserves_original_line_endings() {
    let repo = Repo::new();
    repo.put("in.md", b"PERSON_01\r\n");
    let relative = PathBuf::from("new").join("deep").join("restored.md");
    let output = repo.run(&[
        "restore",
        "--in",
        "in.md",
        "--map",
        "entities.json",
        "--out",
        relative.to_str().unwrap(),
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        native_output(&format!("wrote {}\n", relative.display()))
    );
    assert_eq!(std::fs::read(repo.root.join(&relative)).unwrap(), b"Jane Roe\r\n");
    assert_eq!(repo.get("in.md"), b"PERSON_01\r\n");
    assert_eq!(repo.get("entities.json"), MAP.as_bytes());
    assert!(!repo.root.join(relative.with_extension("md.manifest.json")).exists());
    assert!(!repo.root.join(relative.with_extension("md.triage.md")).exists());
}

#[test]
fn disclosure_findings_refuse_before_touching_an_existing_record() {
    let repo = Repo::new();
    repo.put("out.disclosure.json", b"fabricated previous record\n");
    let input = repo.get("in.md");
    let output = repo.disclosure("disclosure-record");
    assert_eq!(code(&output), 2);
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        native_output("refused: the disclosure payload has verifier findings\n")
    );
    assert_eq!(repo.get("out.disclosure.json"), b"fabricated previous record\n");
    assert_eq!(repo.get("in.md"), input);
    assert_eq!(repo.get("entities.json"), MAP.as_bytes());
}

#[test]
fn disclosure_records_cannot_replace_the_payload_map_or_redaction_artifacts() {
    for path in ["in.md", "entities.json", "in.md.manifest.json", "in.md.triage.md"] {
        let repo = Repo::new();
        let mut ignored = repo.get(".gitignore");
        ignored.extend_from_slice(b"in.md\nin.md.manifest.json\n");
        repo.put(".gitignore", &ignored);
        repo.git(&["check-ignore", "-q", path]);
        repo.put("in.md", b"PERSON_01\n");
        for name in ["in.md.manifest.json", "in.md.triage.md"] {
            repo.put(name, name.as_bytes());
        }
        let before =
            ["in.md", "entities.json", "in.md.manifest.json", "in.md.triage.md"].map(|name| (name, repo.get(name)));
        let output = repo.run(&[
            "disclosure-record",
            "--in",
            "in.md",
            "--map",
            "entities.json",
            "--destination",
            "model:sample",
            "--decision-ref",
            "sample:42",
            "--out",
            path,
        ]);
        assert_eq!(code(&output), 1, "{path}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            stderr(&output),
            native_output("error: invalid disclosure input or record path; use local ignored, untracked records\n")
        );
        for (name, expected) in before {
            assert_eq!(repo.get(name), expected, "{name}");
        }
    }
}

#[test]
fn verify_reports_the_original_line_of_numbers_crossing_list_markers() {
    for (input, expected) in [
        (
            "tfn: 1234567\n82. next",
            "number: line 1\n1 finding(s); this file is not ready to send\n",
        ),
        (
            "tfn: 1234567\n82. next\ntfn: 1234567\n82. next",
            "number: line 1\nnumber: line 3\n2 finding(s); this file is not ready to send\n",
        ),
    ] {
        let repo = Repo::new();
        repo.put("in.md", input.as_bytes());
        let output = repo.run(&repo.normal("verify"));
        assert_eq!(code(&output), 2);
        assert!(output.stderr.is_empty());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), native_output(expected));
        assert_eq!(repo.get("in.md"), input.as_bytes());
        assert_eq!(repo.get("entities.json"), MAP.as_bytes());
        assert!(!repo.path("out.md").exists());
    }
}

#[test]
fn disclosure_is_private_exclusive_versioned_and_bound_to_unchanged_bytes() {
    let repo = Repo::new();
    repo.put("in.md", b"PERSON_01\r\n");
    assert_eq!(code(&repo.disclosure("disclosure-record")), 0);
    let record = repo.get("out.disclosure.json");
    assert!(
        String::from_utf8(record.clone())
            .unwrap()
            .contains("\"tool_version\": \"1.0.0\"")
    );
    assert_eq!(code(&repo.disclosure("disclosure-check")), 0);
    assert_eq!(code(&repo.disclosure("disclosure-record")), 1);
    assert_eq!(repo.get("out.disclosure.json"), record);
    repo.put("in.md", b"PERSON_01\n");
    assert_eq!(code(&repo.disclosure("disclosure-check")), 2);
    repo.put("in.md", b"PERSON_01\r\n");
    let old = String::from_utf8(record)
        .unwrap()
        .replace("\"tool_version\": \"1.0.0\"", "\"tool_version\": \"0.2.0\"");
    repo.put("out.disclosure.json", old.as_bytes());
    assert_eq!(code(&repo.disclosure("disclosure-check")), 2);
    repo.put("out.disclosure.json", b"{\"Sample Sensitive Sentinel\":NaN}");
    let output = repo.disclosure("disclosure-check");
    assert_eq!(code(&output), 1);
    assert!(!stderr(&output).contains("Sensitive"));
    for argv in [
        vec!["--dest", "Sample Sensitive Sentinel"],
        vec!["disclosure-reocrd", "--decision", "Sample Sensitive Sentinel"],
        vec!["disclosure-record", "--d", "Sample Sensitive Sentinel"],
    ] {
        let output = repo.run(&argv);
        assert_eq!(code(&output), 1);
        assert!(stderr(&output).contains("invalid disclosure arguments"));
        assert!(!stderr(&output).contains("Sensitive"));
    }
}

#[test]
fn surrogate_restore_leaves_the_same_empty_file_and_one_error_line() {
    let repo = Repo::new();
    repo.put("in.md", b"PERSON_01");
    repo.put("entities.json", MAP.replace("Jane Roe", r"Jane \ud800Roe").as_bytes());
    let output = repo.run(&repo.normal("restore"));
    assert_eq!(code(&output), 1);
    assert_eq!(repo.get("out.md"), b"");
    assert_eq!(stderr(&output).lines().count(), 1);
    assert!(stderr(&output).starts_with("error: "));
    assert_eq!(code(&repo.run(&repo.normal("verify"))), 0);
}

#[cfg(unix)]
#[test]
fn record_static_links_missing_dotdot_and_fifo_are_refused() {
    use std::os::unix::fs::symlink;
    let repo = Repo::new();
    repo.put("in.md", b"PERSON_01");
    std::fs::create_dir(repo.path("records")).unwrap();
    symlink("records", repo.path("linked")).unwrap();
    for path in ["linked/out.disclosure.json", "missing/../linked/../out.disclosure.json"] {
        let output = repo.run(&[
            "disclosure-record",
            "--in",
            "in.md",
            "--map",
            "entities.json",
            "--destination",
            "model:sample",
            "--decision-ref",
            "sample:42",
            "--out",
            path,
        ]);
        assert_eq!(code(&output), 1);
        assert!(!repo.path("missing").exists());
    }
    assert_eq!(code(&repo.disclosure("disclosure-record")), 0);
    std::fs::hard_link(repo.path("out.disclosure.json"), repo.path("alias.disclosure.json")).unwrap();
    assert_eq!(code(&repo.disclosure("disclosure-check")), 1);
    std::fs::remove_file(repo.path("alias.disclosure.json")).unwrap();
    std::fs::remove_file(repo.path("out.disclosure.json")).unwrap();
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(repo.path("out.disclosure.json").as_os_str().as_bytes()).unwrap();
    // SAFETY: the path is NUL terminated and refers only to this fabricated test directory.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert_eq!(code(&repo.disclosure("disclosure-check")), 1);
}

#[cfg(unix)]
#[test]
fn relative_git_path_is_pinned_before_changing_to_the_map_directory() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    std::fs::create_dir(repo.path("bin")).unwrap();
    std::fs::create_dir(repo.path("nested")).unwrap();
    repo.put("nested/entities.json", MAP.as_bytes());
    repo.put(
        "bin/git",
        b"#!/bin/sh\ncase $1 in ls-files) exit 1;; check-ignore) exit 0;; esac\nexit 3\n",
    );
    std::fs::set_permissions(repo.path("bin/git"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = repo
        .command(&["verify", "--in", "in.md", "--map", "nested/entities.json"])
        .env("PATH", "bin")
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    let missing = repo
        .command(&repo.normal("verify"))
        .env("PATH", repo.path("no-bin"))
        .output()
        .unwrap();
    assert_eq!(code(&missing), 1);
    assert!(stderr(&missing).contains("git is not on PATH"));
}

#[test]
fn wide_triage_shares_one_context_and_quotes_it_once() {
    let repo = Repo::new();
    let text = (10000000..10002000)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(";");
    repo.put("in.md", text.as_bytes());
    assert_eq!(code(&repo.run(&repo.normal("redact"))), 2);
    let triage = String::from_utf8(repo.get("out.md.triage.md")).unwrap();
    assert_eq!(triage.matches("  > ").count(), 1);
    let found = evatt::redact::residual(&text).unwrap();
    assert!(
        found
            .iter()
            .all(|u| std::sync::Arc::ptr_eq(&found[0].context, &u.context))
    );
}

#[cfg(unix)]
fn closed_pipe() -> std::process::Stdio {
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut descriptors = [0; 2];
    // SAFETY: descriptors is writable storage for both newly owned pipe ends.
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    // SAFETY: pipe succeeded; these descriptors have no other Rust owners.
    let reader = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
    // SAFETY: pipe succeeded; this descriptor has no other Rust owner.
    let writer = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
    drop(reader);
    std::process::Stdio::from(writer)
}

#[cfg(unix)]
#[test]
fn closed_pipes_keep_buffered_and_argparse_unbuffered_exit_codes() {
    let repo = Repo::new();
    for (unbuffered, expected, message) in [("0", 120, "error: cannot flush CLI diagnostic\n"), ("1", 0, "")] {
        let output = repo
            .command(&["--version"])
            .env("PYTHONUNBUFFERED", unbuffered)
            .stdout(closed_pipe())
            .output()
            .unwrap();
        assert_eq!(code(&output), expected);
        assert_eq!(stderr(&output), message);
    }
    for (unbuffered, expected) in [("0", 120), ("1", 1)] {
        let output = repo
            .command(&[])
            .env("PYTHONUNBUFFERED", unbuffered)
            .stderr(closed_pipe())
            .output()
            .unwrap();
        assert_eq!(code(&output), expected);
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn stream_settings_cpython_refuses_at_start_up_write_nothing() {
    let repo = Repo::new();
    let cases: [&[(&str, &str)]; 8] = [
        &[("PYTHONUTF8", "2"), ("PYTHONIOENCODING", "utf-8")],
        &[("PYTHONUTF8", "2"), ("PYTHONIOENCODING", "utf-8:strict")],
        &[("PYTHONUTF8", "2"), ("PYTHONIOENCODING", "cp1252")],
        &[("PYTHONIOENCODING", "bogus")],
        // CPython drops the non-ASCII letter and looks up "asci", which fails.
        &[("PYTHONIOENCODING", "asci\u{130}")],
        // Handlers not implemented here are refused before any command runs.
        &[("PYTHONIOENCODING", "ascii:xmlcharrefreplace")],
        &[("PYTHONIOENCODING", "utf-8:namereplace")],
        &[("PYTHONIOENCODING", "utf-8:bogus")],
    ];
    for env in cases {
        for args in [vec!["--version"], repo.normal("redact")] {
            let mut command = repo.command(&args);
            command.envs(env.iter().copied());
            let output = command.output().unwrap();
            assert_eq!(code(&output), 1, "{env:?} {args:?}");
            assert!(output.stdout.is_empty(), "{env:?} {args:?}");
            assert!(
                stderr(&output).starts_with("error: unsupported stream encoding"),
                "{env:?}"
            );
            assert!(!repo.path("out.md").exists() && !repo.path("out.md.manifest.json").exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn undecodable_python_variables_refuse_and_pathext_is_ignored_on_posix() {
    use std::os::unix::ffi::OsStringExt;
    let repo = Repo::new();
    let raw = |bytes: &[u8]| std::ffi::OsString::from_vec(bytes.to_vec());
    for (name, value) in [
        ("PYTHONUTF8", raw(b"\xff")),
        ("PYTHONIOENCODING", raw(b"bogus\xff")),
        ("PYTHONIOENCODING", raw(b"utf-8\xff")),
        ("PYTHONIOENCODING", raw(b"utf-8:\xff")),
    ] {
        let mut command = repo.command(&repo.normal("redact"));
        command.env(name, &value);
        let output = command.output().unwrap();
        assert_eq!(code(&output), 1, "{name}");
        assert!(!repo.path("out.md").exists(), "{name}");
    }
    let mut command = repo.command(&repo.normal("verify"));
    command.env("PATHEXT", raw(b"\xff"));
    assert_eq!(code(&command.output().unwrap()), 2);
    // CPython prints locale coercion warnings to stderr under this setting.
    let mut command = repo.command(&repo.normal("redact"));
    command.env("PYTHONCOERCECLOCALE", "warn");
    assert_eq!(code(&command.output().unwrap()), 1);
    assert!(!repo.path("out.md").exists());
}

#[cfg(windows)]
#[test]
fn legacy_windows_stream_modes_refuse_and_empty_values_count_as_unset() {
    let repo = Repo::new();
    for name in ["PYTHONLEGACYWINDOWSFSENCODING", "PYTHONLEGACYWINDOWSSTDIO"] {
        let mut command = repo.command(&repo.normal("redact"));
        command.env(name, "1");
        let output = command.output().unwrap();
        assert_eq!(code(&output), 1, "{name}");
        assert!(output.stdout.is_empty() && !repo.path("out.md").exists(), "{name}");
        let mut command = repo.command(&["--version"]);
        command.env(name, "");
        assert_eq!(code(&command.output().unwrap()), 0, "{name} empty");
    }
}

#[cfg(windows)]
#[test]
fn default_windows_stdout_escapes_a_lone_low_surrogate_like_cpython() {
    use std::os::windows::ffi::OsStringExt;
    let repo = Repo::new();
    repo.put("in.md", b"Hello world.\n");
    // "out" + U+DCE9 + ".md": CPython 3.14.8 writes b"wrote out\xe9.md\r\n", exit 0.
    let out = std::ffi::OsString::from_wide(&[111, 117, 116, 0xDCE9, 46, 109, 100]);
    let mut command = repo.configured(child_profile::command(env!("CARGO_BIN_EXE_evatt")));
    command
        .args(["redact", "--in", "in.md", "--map", "entities.json", "--out"])
        .arg(&out)
        .env_remove("PYTHONIOENCODING")
        .env_remove("PYTHONUTF8");
    let output = command.output().unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"wrote out\xe9.md\r\n");
}
