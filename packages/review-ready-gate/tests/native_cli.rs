// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/differential_cli.rs"]
mod differential_cli;

use review_ready_gate::probe::Json;
use std::{fs, path::PathBuf, process::Command};
#[path = "support/temp.rs"]
mod temp;
use temp::TempDir;

fn gate_command(example: &str, output: &std::path::Path) -> Command {
    fixture_command(
        "bas",
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join(example),
        output,
    )
}

fn fixture_command(profile: &str, pack: &std::path::Path, output: &std::path::Path) -> Command {
    let mut command = child_profile::command(env!("CARGO_BIN_EXE_review-ready"));
    command
        .args(["gate", "--profile", profile, "--pack"])
        .arg(pack)
        .arg("--output")
        .arg(output)
        .env_remove("GIT_WORK_TREE");
    command
}

#[test]
fn malformed_decimal_tolerances_refuse_before_reading_or_writing_a_pack() {
    let tmp = TempDir::new();
    for (value, representation) in [
        ("1\u{a0}2", "'1\\xa02'"),
        ("1e+", "'1e+'"),
        (
            "1e12345678901234567890123456",
            "'1e12345678901234567890123456'",
        ),
    ] {
        let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
            .args(["gate", "--tieout-tolerance", value])
            .current_dir(&tmp.0)
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1), "{value:?}");
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8(result.stderr)
            .unwrap()
            .replace("\r\n", "\n");
        assert!(stderr.starts_with("usage: review-ready gate"), "{stderr}");
        assert!(stderr.ends_with(&format!(
            "review-ready gate: error: argument --tieout-tolerance: not a decimal: {representation}\n"
        )), "{stderr}");
        assert_eq!(fs::read_dir(&tmp.0).unwrap().count(), 0);
    }
}

#[test]
fn checkout_markers_and_configured_work_trees_refuse_before_input_reads() {
    let tmp = TempDir::new();
    for marker in [".git", ".hg", ".svn", ".bzr"] {
        for directory_marker in [false, true] {
            let checkout = tmp.join(format!("{}-{directory_marker}", &marker[1..]));
            fs::create_dir(&checkout).unwrap();
            if directory_marker {
                fs::create_dir(checkout.join(marker)).unwrap();
            } else {
                fs::write(checkout.join(marker), b"marker").unwrap();
            }
            let output = checkout.join("absent/nested/output");
            let result = gate_command("does-not-exist", &output).output().unwrap();
            assert_eq!(result.status.code(), Some(1));
            assert!(result.stdout.is_empty());
            assert!(String::from_utf8_lossy(&result.stderr).contains("version-control checkout"));
            assert!(!output.exists());
        }
    }
    let tracked = tmp.join("tracked");
    fs::create_dir(&tracked).unwrap();
    let output = tracked.join("output");
    for value in [tracked.clone(), PathBuf::from("tracked")] {
        let result = gate_command("does-not-exist", &output)
            .current_dir(&tmp.0)
            .env("GIT_WORK_TREE", value)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&result.stderr).contains("GIT_WORK_TREE"));
        assert!(!output.exists());
    }
}
#[test]
fn checkout_aliases_and_symlink_loops_are_refused_before_input_reads() {
    let tmp = TempDir::new();
    let checkout = tmp.join("checkout");
    fs::create_dir(&checkout).unwrap();
    fs::create_dir(checkout.join(".git")).unwrap();
    let alias = tmp.join("alias");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&checkout, &alias).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&checkout, &alias).unwrap();
    let output = alias.join("nested/output");
    let result = gate_command("does-not-exist", &output).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("version-control checkout"));
    assert!(!output.exists());
    let looping = tmp.join("loop");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&looping, &looping).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&looping, &looping).unwrap();
    let result = gate_command("does-not-exist", &looping.join("output"))
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot be examined"));
}
#[cfg(unix)]
#[test]
fn unreadable_checkout_metadata_is_refused_before_input_reads() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new();
    let parent = tmp.join("unreadable");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o0)).unwrap();
    let result = gate_command("does-not-exist", &parent.join("output")).output();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let result = result.unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot be examined"));
    assert!(!parent.join("output").exists());
}
fn snapshot(
    path: &std::path::Path,
) -> std::collections::BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    fn visit(
        root: &std::path::Path,
        path: &std::path::Path,
        files: &mut std::collections::BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                visit(root, &entry.path(), files);
            } else if kind.is_file() {
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    (
                        fs::read(entry.path()).unwrap(),
                        entry.metadata().unwrap().modified().unwrap(),
                    ),
                );
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    visit(path, path, &mut files);
    files
}
#[test]
fn display_commands_preserve_payload_and_publication_bytes_and_mtimes() {
    let tmp = TempDir::new();
    let output = tmp.join("pack");
    assert!(
        gate_command("bas-ready", &output)
            .output()
            .unwrap()
            .status
            .success()
    );
    let before = snapshot(&output);
    for args in [
        vec!["view", "--pack-dir"],
        vec!["compare", "--previous-pack-dir"],
    ] {
        let mut command = child_profile::command(env!("CARGO_BIN_EXE_review-ready"));
        command.args(&args).arg(&output);
        if args[0] == "compare" {
            command.arg("--current-pack-dir").arg(&output);
        }
        assert!(command.output().unwrap().status.success());
        assert_eq!(snapshot(&output), before);
    }
    fs::write(output.join("readiness-summary.md"), b"edited").unwrap();
    let before = snapshot(&output);
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["view", "--pack-dir"])
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .starts_with("review-ready view: verification failed:")
    );
    assert_eq!(snapshot(&output), before);
    let empty = tmp.join("empty");
    fs::create_dir(&empty).unwrap();
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["view", "--pack-dir"])
        .arg(&empty)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(fs::read_dir(&empty).unwrap().next().is_none());
}
fn refused_view(path: &std::path::Path) {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    let mut child = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["view", "--pack-dir"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(1));
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("viewer did not refuse the special payload before reading it");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one refusal case per special file type"
)]
fn viewer_refuses_directory_hardlink_symlink_and_fifo_payloads_before_reading() {
    for name in [
        "readiness-pack.json",
        "readiness-summary.md",
        "findings.csv",
    ] {
        for kind in ["directory", "hardlink", "symlink"] {
            let tmp = TempDir::new();
            let output = tmp.join("pack");
            assert!(
                gate_command("bas-ready", &output)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            let payload = output.join(name);
            let target = tmp.join("retained");
            fs::copy(&payload, &target).unwrap();
            let original = fs::read(&target).unwrap();
            fs::remove_file(&payload).unwrap();
            match kind {
                "directory" => {
                    fs::create_dir(&payload).unwrap();
                }
                "hardlink" => {
                    fs::hard_link(&target, &payload).unwrap();
                }
                "symlink" => {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&target, &payload).unwrap();
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_file(&target, &payload).unwrap();
                }
                _ => unreachable!(),
            }
            refused_view(&output);
            assert_eq!(fs::read(&target).unwrap(), original);
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let tmp = TempDir::new();
            let output = tmp.join("pack");
            assert!(
                gate_command("bas-ready", &output)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            let payload = output.join(name);
            fs::remove_file(&payload).unwrap();
            let path = std::ffi::CString::new(payload.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
            refused_view(&output);
        }
    }
}
#[test]
fn closed_stdout_returns_failure_after_preserving_a_published_pack() {
    let tmp = TempDir::new();
    let output = tmp.join("pack");
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["gate", "--profile", "bas", "--pack"])
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/bas-ready"))
        .arg("--output")
        .arg(&output)
        .stdout(writer)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let before = [
        "readiness-pack.json",
        "readiness-summary.md",
        "findings.csv",
    ]
    .map(|name| fs::read(output.join(name)).unwrap());
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let shown = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["view", "--pack-dir"])
        .arg(&output)
        .stdout(writer)
        .output()
        .unwrap();
    assert_eq!(shown.status.code(), Some(1));
    let after = [
        "readiness-pack.json",
        "readiness-summary.md",
        "findings.csv",
    ]
    .map(|name| fs::read(output.join(name)).unwrap());
    assert_eq!(before, after);
}
#[cfg(unix)]
#[test]
fn failed_stdout_device_returns_failure() {
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .arg("--help")
        .stdout(fs::File::options().write(true).open("/dev/full").unwrap())
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
}

#[cfg(feature = "probe")]
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one Unicode path scenario with its checks"
)]
fn native_intake_preserves_private_use_unicode_arguments_and_paths() {
    let tmp = TempDir::new();
    let source = tmp.join("original-\u{10f800}.txt");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/document-intake/invoice.txt"),
        &source,
    )
    .unwrap();
    let output = tmp.join("bundle-\u{10f800}");
    let entity = "Demo\u{10f7ff}\u{10f800}";
    let version = "manual\u{10f7ff}\u{10f800}";
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["intake", "--synthetic", "--source"])
        .arg(&source)
        .arg("--text")
        .arg(&source)
        .args([
            "--text-origin",
            "manual",
            "--text-version",
            version,
            "--entity",
            entity,
            "--period-end",
            "2026-03-31",
            "--output",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = review_ready_gate::probe::decode(
        &fs::read_to_string(output.join("document-intake.json")).unwrap(),
    )
    .unwrap();
    assert!(manifest.at("entity_ref").is_str(entity));
    assert!(manifest.at("text_version").is_str(version));
    let before = snapshot(&output);
    let shown = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["view-document", "--bundle"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    assert_eq!(snapshot(&output), before);
}

#[test]
fn deeply_nested_json_is_a_named_input_error_in_the_real_process() {
    for depth in [500, 1001, 10000] {
        let tmp = TempDir::new();
        let pack = tmp.join("pack");
        fs::create_dir(&pack).unwrap();
        let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/bas-ready");
        for name in [
            "trial_balance.csv",
            "activity_statement.csv",
            "gst_control_gl.csv",
            "open_items.csv",
        ] {
            fs::copy(examples.join(name), pack.join(name)).unwrap();
        }
        fs::write(
            pack.join("self_review.json"),
            format!("{}null{}", "[".repeat(depth), "]".repeat(depth)),
        )
        .unwrap();
        let output = tmp.join("output");
        let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
            .args(["gate", "--profile", "bas", "--pack"])
            .arg(&pack)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(1),
            "depth {depth}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8_lossy(&result.stderr).starts_with("review-ready: input error:"));
        assert!(!output.exists());
    }
}

fn payloads(path: &std::path::Path) -> [Option<Vec<u8>>; 3] {
    review_ready_gate::report::PACK_NAMES.map(|name| fs::read(path.join(name)).ok())
}
#[test]
fn publication_probe_refuses_wrong_argument_counts_before_creating_output() {
    let tmp = TempDir::new();
    for args in [
        vec![],
        vec!["output", "never"],
        vec!["output", "never", "new", "extra"],
    ] {
        let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
            .args(args)
            .current_dir(&tmp.0)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        assert_eq!(
            String::from_utf8(result.stderr)
                .unwrap()
                .replace("\r\n", "\n"),
            "usage: review-ready-publication-probe OUTPUT CHECKPOINT PREFIX\n"
        );
        assert_eq!(fs::read_dir(&tmp.0).unwrap().count(), 0);
    }
}
#[test]
fn publication_probe_refuses_a_checkout_without_creating_output() {
    let tmp = TempDir::new();
    let checkout = tmp.join("checkout");
    fs::create_dir(&checkout).unwrap();
    fs::write(checkout.join(".git"), b"fabricated checkout marker").unwrap();
    let output = checkout.join("pack");
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
        .arg(&output)
        .args(["never", "new"])
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("version-control checkout"));
    assert!(!output.exists());
    assert_eq!(fs::read_dir(&checkout).unwrap().count(), 1);
    assert_eq!(
        fs::read(checkout.join(".git")).unwrap(),
        b"fabricated checkout marker"
    );
}
#[test]
fn publication_probe_preserves_an_output_path_occupied_by_a_file() {
    let tmp = TempDir::new();
    let output = tmp.join("occupied");
    fs::write(&output, b"retain this file").unwrap();
    let expected = fs::create_dir_all(&output).unwrap_err().to_string();
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
        .arg(&output)
        .args(["never", "new"])
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert_eq!(String::from_utf8_lossy(&result.stderr).trim_end(), expected);
    assert_eq!(fs::read(&output).unwrap(), b"retain this file");
    assert_eq!(fs::read_dir(&tmp.0).unwrap().count(), 1);
}
#[cfg(feature = "probe")]
fn seed(path: &std::path::Path) {
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
        .arg(path)
        .args(["never", "old"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[cfg(feature = "probe")]
struct Child(std::process::Child);
#[cfg(feature = "probe")]
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[cfg(feature = "probe")]
fn paused(path: &std::path::Path, point: &str) -> Child {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let mut child = Child(
        child_profile::command(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
            .arg(path)
            .arg(point)
            .arg("new")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    let line = receiver.recv_timeout(std::time::Duration::from_secs(10));
    if line.is_err() {
        let _ = child.0.kill();
        let _ = child.0.wait();
    }
    reader.join().unwrap();
    assert_eq!(line.unwrap().unwrap().trim(), format!("checkpoint {point}"));
    child
}

fn unrecorded_transaction(control: &std::path::Path) -> PathBuf {
    let mut transactions: Vec<_> = fs::read_dir(control)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("tx-")
        })
        .collect();
    assert_eq!(transactions.len(), 1);
    transactions.pop().unwrap()
}

fn resume_publication(mut child: Child) -> (std::process::ExitStatus, String) {
    use std::io::Write;
    use std::time::{Duration, Instant};
    child.0.stdin.take().unwrap().write_all(b"\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "publication did not resume");
        std::thread::sleep(Duration::from_millis(10));
    };
    let stderr = std::io::read_to_string(child.0.stderr.take().unwrap()).unwrap();
    (status, stderr)
}

#[test]
fn obstructed_state_write_preserves_the_old_generation_and_unrecorded_evidence() {
    for retained_transaction in [false, true] {
        let tmp = TempDir::new();
        let output = tmp.join("output");
        seed(&output);
        let old = payloads(&output);
        let revision = review_ready_gate::publication::snapshot_revision(&output).unwrap();
        let control = output.join(review_ready_gate::publication::CONTROL_NAME);
        let old_state = fs::read(control.join("state.json")).unwrap();
        let child = paused(&output, "transaction-created");
        let transaction = unrecorded_transaction(&control);
        let pending_state = control.join("state.next");
        fs::write(&pending_state, b"retain pending state").unwrap();
        let expected = if retained_transaction {
            fs::write(transaction.join("unknown"), b"retain transaction evidence").unwrap();
            "unrecorded transaction remains; retain control for review".to_string()
        } else {
            fs::remove_dir(&transaction).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&pending_state)
                .unwrap_err()
                .to_string()
        };
        let (status, stderr) = resume_publication(child);
        assert_eq!(status.code(), Some(1));
        assert_eq!(stderr.trim_end(), expected);
        assert_eq!(payloads(&output), old);
        assert_eq!(fs::read(control.join("state.json")).unwrap(), old_state);
        assert_eq!(
            review_ready_gate::publication::snapshot_revision(&output).unwrap(),
            revision
        );
        assert_eq!(fs::read(&pending_state).unwrap(), b"retain pending state");
        if retained_transaction {
            assert_eq!(
                fs::read(transaction.join("unknown")).unwrap(),
                b"retain transaction evidence"
            );
        } else {
            assert!(!transaction.exists());
        }
    }
}

#[cfg(feature = "probe")]
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one crash point per publication stage"
)]
fn abrupt_death_keeps_precommit_evidence_and_preserves_a_settled_commit() {
    for point in [
        "in-progress",
        "prepared",
        "install-0",
        "install-1",
        "install-2",
        "committed",
    ] {
        let tmp = TempDir::new();
        let output = tmp.join("output");
        seed(&output);
        let mut child = paused(&output, point);
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        if point == "committed" {
            assert!(
                review_ready_gate::publication::snapshot_revision(&output)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                payloads(&output),
                std::array::from_fn(|i| Some(format!("new-{i}").into_bytes()))
            );
            let retry =
                child_profile::command(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
                    .arg(&output)
                    .args(["never", "next"])
                    .output()
                    .unwrap();
            assert!(retry.status.success());
        } else {
            let before = payloads(&output);
            assert!(review_ready_gate::publication::snapshot_revision(&output).is_err());
            let retry =
                child_profile::command(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
                    .arg(&output)
                    .args(["never", "next"])
                    .output()
                    .unwrap();
            assert_eq!(retry.status.code(), Some(1));
            assert!(String::from_utf8_lossy(&retry.stderr).contains("unresolved pack publication"));
            assert_eq!(payloads(&output), before);
            let control = output.join(review_ready_gate::publication::CONTROL_NAME);
            assert!(
                fs::read_dir(&control).unwrap().any(|p| p
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("tx-"))
            );
        }
    }
}
#[cfg(feature = "probe")]
#[test]
fn second_process_cannot_acquire_writer_admission() {
    let tmp = TempDir::new();
    let output = tmp.join("output");
    seed(&output);
    let _child = paused(&output, "prepared");
    let before = payloads(&output);
    let other = child_profile::command(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
        .arg(&output)
        .args(["never", "other"])
        .output()
        .unwrap();
    assert_eq!(other.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&other.stderr).contains("another pack writer holds admission"));
    assert_eq!(payloads(&output), before);
}
#[cfg(feature = "probe")]
#[test]
fn a_missing_signal_expires_the_checkpoint_and_rolls_back_the_publication() {
    use std::io::Write;
    use std::time::{Duration, Instant};

    let tmp = TempDir::new();
    let output = tmp.join("output");
    seed(&output);
    let old = payloads(&output);
    let mut child = paused(&output, "prepared");
    child
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(b"cancel\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "signal checkpoint did not expire"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(1));
    let stderr = std::io::read_to_string(child.0.stderr.take().unwrap()).unwrap();
    assert_eq!(stderr.trim(), "signal checkpoint timed out");
    assert_eq!(payloads(&output), old);
    assert!(
        review_ready_gate::publication::snapshot_revision(&output)
            .unwrap()
            .is_some()
    );
    let retry = child_profile::command(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
        .arg(&output)
        .args(["never", "next"])
        .output()
        .unwrap();
    assert!(retry.status.success(), "{retry:?}");
    assert_eq!(
        payloads(&output),
        std::array::from_fn(|i| Some(format!("next-{i}").into_bytes()))
    );
}
#[cfg(all(unix, feature = "probe"))]
#[test]
fn managed_sigterm_finishes_rollback_and_keeps_postcommit_generation() {
    use std::io::Write;
    for point in [
        "initialised",
        "in-progress",
        "backup-0",
        "stage-1",
        "prepared",
        "install-0",
        "install-1",
        "install-2",
        "committed",
        "finished",
    ] {
        let tmp = TempDir::new();
        let output = tmp.join("output");
        seed(&output);
        let old = payloads(&output);
        let mut child = paused(&output, point);
        // SAFETY: this is the owned child paused at a writer checkpoint.
        assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGTERM) }, 0);
        child
            .0
            .stdin
            .take()
            .unwrap()
            .write_all(b"cancel\n")
            .unwrap();
        let status = child.0.wait().unwrap();
        assert_eq!(status.code(), Some(1), "{point}");
        assert!(
            review_ready_gate::publication::snapshot_revision(&output)
                .unwrap()
                .is_some()
        );
        if ["committed", "finished"].contains(&point) {
            assert_eq!(
                payloads(&output),
                std::array::from_fn(|i| Some(format!("new-{i}").into_bytes()))
            );
        } else {
            assert_eq!(payloads(&output), old, "{point}");
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one producer scenario with its assertions"
)]
fn the_gate_records_its_producer_and_view_refuses_an_edited_one() {
    let tmp = TempDir::new();
    let mut runs = Vec::new();
    for name in ["first", "second"] {
        let output = tmp.join(name);
        assert!(
            gate_command("bas-ready", &output)
                .output()
                .unwrap()
                .status
                .success()
        );
        runs.push(payloads(&output));
    }
    assert_eq!(runs[0], runs[1]);
    let output = tmp.join("first");
    let json = fs::read_to_string(output.join("readiness-pack.json")).unwrap();
    let version = format!("\"version\": \"{}\"", env!("CARGO_PKG_VERSION"));
    for member in [
        "\"producer\": {",
        "\"component\": \"review-ready-gate\"",
        "\"pack_contract\": \"2\"",
        &version,
    ] {
        assert!(json.contains(member), "{member} missing from {json}");
    }
    let line = format!(
        "- Produced by: review-ready-gate {}, commit ",
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        fs::read_to_string(output.join("readiness-summary.md"))
            .unwrap()
            .contains(&line)
    );
    let view = |path: &std::path::Path| {
        child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
            .args(["view", "--pack-dir"])
            .arg(path)
            .output()
            .unwrap()
    };
    let shown = view(&output);
    assert!(shown.status.success(), "{shown:?}");
    assert!(String::from_utf8_lossy(&shown.stdout).contains(&line));
    for (index, (before, after, diagnostic)) in [
        (
            version.as_str(),
            "\"version\": \"9.9.9\"",
            "verification failed",
        ),
        (
            "\"pack_contract\": \"2\"",
            "\"pack_contract\": \"3\"",
            "producer must hold exactly",
        ),
        (
            "\"component\": \"review-ready-gate\"",
            "\"component\": \"monthly-close-control-plane\"",
            "producer must hold exactly",
        ),
        (
            "\"producer\": {",
            "\"producer\": {\"built_at\": \"2026-10-28\",",
            "producer must hold exactly",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let edited = tmp.join(format!("edited-{index}"));
        fs::create_dir(&edited).unwrap();
        for name in ["readiness-summary.md", "findings.csv"] {
            fs::copy(output.join(name), edited.join(name)).unwrap();
        }
        assert!(json.contains(before));
        fs::write(
            edited.join("readiness-pack.json"),
            json.replacen(before, after, 1),
        )
        .unwrap();
        let shown = view(&edited);
        assert_eq!(shown.status.code(), Some(1), "{shown:?}");
        assert!(shown.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&shown.stderr).contains(diagnostic),
            "{shown:?}"
        );
    }
}

fn pack_json(output: &std::path::Path) -> String {
    fs::read_to_string(output.join("readiness-pack.json")).unwrap()
}

fn json_array(value: &Json) -> &[Json] {
    match value {
        Json::Array(items) => items,
        other => panic!("Expected a JSON array, got {other:?}"),
    }
}

fn assert_only_finding(
    output: &std::path::Path,
    status: &str,
    slot: &str,
    code: &str,
    reason: &str,
) {
    let before = snapshot(output);
    let verified = review_ready_gate::viewer::verify_pack(output).unwrap();
    assert!(verified.document.at("overall_status").is_str(status));
    let document = pack_json(output);
    assert_eq!(document.matches("\"code\":").count(), 1);
    for (name, value) in [
        ("slot", slot),
        ("code", code),
        ("status", status),
        ("reason", reason),
    ] {
        assert!(
            document.contains(&format!("\"{name}\": \"{value}\"")),
            "{document}"
        );
    }
    assert_eq!(snapshot(output), before);
}

#[test]
fn gst_rollforward_counts_each_control_account_once_across_repeated_postings() {
    for (receivable, bank, code) in [("2500.00", "127500.00", 0), ("2499.98", "127500.02", 2)] {
        let tmp = TempDir::new();
        let input = pack_copy(&tmp, "bas-ready");
        let path = input.join("trial_balance.csv");
        let original = fs::read_to_string(&path).unwrap();
        let mut changed = original
            .replace("0.00,7500.00", "0.00,10000.00")
            .replace("127500.00,0.00", &format!("{bank},0.00"));
        assert_ne!(changed, original);
        changed.push_str(&format!("2026-03-31,Cedar and Pine Consulting Pty Ltd,Assets,821,GST Receivable,8210,0.00,0.00,{receivable},0.00\n"));
        fs::write(&path, changed).unwrap();
        fs::write(input.join("gst_control_gl.csv"), "Date,AccountID,AccountName,Debit,Credit,Description\n2026-01-15,820,GST Payable,0.00,6000.00,First sales posting\n2026-01-16,820,GST Payable,0.00,4000.00,Second sales posting\n2026-02-20,821,GST Receivable,2500.00,0.00,GST on purchases\n").unwrap();
        let before = snapshot(&input);
        let output = tmp.join("output");
        let result = fixture_command("bas", &input, &output).output().unwrap();
        assert_eq!(result.status.code(), Some(code), "{result:?}");
        if code == 0 {
            let verified = review_ready_gate::viewer::verify_pack(&output).unwrap();
            assert!(verified.document.at("overall_status").is_str("READY"));
            assert!(pack_json(&output).contains("\"findings\": []"));
        } else {
            assert_only_finding(
                &output,
                "NOT_READY",
                "gst_control_gl",
                "TIEOUT_BREAK",
                "GST control accounts close at 7500.02 in the trial balance, but the GST control file moves them by 7500.00 and, with no GST_OPENING on the activity statement, they are taken to carry in nil; difference 0.02 exceeds tolerance 0.01. The file may be missing postings, or the accounts carried a balance in, such as an unpaid earlier BAS or the cents the ATO drops from 1A and 1B, which GST_OPENING declares.",
            );
        }
        assert_eq!(snapshot(&input), before);
    }
}

#[test]
fn statement_mapping_reuses_accounts_on_detail_lines_and_a_subtotal() {
    for (subtotal, code) in [("205000.00", 0), ("205000.02", 2)] {
        let tmp = TempDir::new();
        let input = pack_copy(&tmp, "year-end-ready");
        fs::write(input.join("tie_out_matrix.csv"), format!("StatementLine,StatementAmount,WorkpaperRef,SourceFile,Status\nCash,120000.00,WP-100,trial_balance.csv,TIED\nTrade debtors,85000.00,WP-110,trial_balance.csv,TIED\nTrade creditors,65000.00,WP-200,trial_balance.csv,TIED\nLoss for the year,40000.00,WP-300,trial_balance.csv,TIED\nRetained earnings,180000.00,WP-900,trial_balance.csv,TIED\nCurrent assets,{subtotal},WP-120,trial_balance.csv,TIED\n")).unwrap();
        fs::write(input.join("fs_mapping.csv"), "StatementLine,AccountID,NormalBalance\nCash,100,DEBIT\nTrade debtors,110,DEBIT\nTrade creditors,200,CREDIT\nLoss for the year,300,DEBIT\nLoss for the year,400,DEBIT\nLoss for the year,500,DEBIT\nRetained earnings,900,CREDIT\nCurrent assets,100,DEBIT\nCurrent assets,110,DEBIT\n").unwrap();
        let before = snapshot(&input);
        let output = tmp.join("output");
        let result = fixture_command("year_end", &input, &output)
            .args(["--require-control", "fs_mapping"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(code), "{result:?}");
        if code == 0 {
            let verified = review_ready_gate::viewer::verify_pack(&output).unwrap();
            assert!(verified.document.at("overall_status").is_str("READY"));
            let document = pack_json(&output);
            assert!(document.contains("\"findings\": []"));
            assert!(document.contains("\"name\": \"fs_mapping\""));
            assert_eq!(document.matches("\"state\": \"ran\"").count(), 1);
        } else {
            assert_only_finding(
                &output,
                "NOT_READY",
                "fs_mapping",
                "TIEOUT_BREAK",
                "Statement line 'Current assets' shows 205000.02, but its mapped trial-balance accounts give 205000.00; difference 0.02 exceeds tolerance 0.01.",
            );
        }
        assert_eq!(snapshot(&input), before);
    }
}

#[test]
fn comparative_entity_and_period_refusals_publish_verified_blocked_reports() {
    for (before, after, reason) in [
        (
            "Varrock Ventures Pty Ltd",
            "Lumbridge Ventures Pty Ltd",
            "Current tenant 'Varrock Ventures Pty Ltd' does not match prior tenant 'Lumbridge Ventures Pty Ltd'.",
        ),
        (
            "2026-05-31",
            "2026-06-30",
            "Prior ReportDate 2026-06-30 is not earlier than current ReportDate 2026-06-30.",
        ),
        (
            "2026-05-31",
            "2026-07-31",
            "Prior ReportDate 2026-07-31 is not earlier than current ReportDate 2026-06-30.",
        ),
    ] {
        let tmp = TempDir::new();
        let input = pack_copy(&tmp, "month-end-ready");
        let path = input.join("prior_trial_balance.csv");
        let original = fs::read_to_string(&path).unwrap();
        assert_eq!(original.matches(before).count(), 7);
        fs::write(path, original.replace(before, after)).unwrap();
        let before = snapshot(&input);
        let output = tmp.join("output");
        let result = fixture_command("month_end", &input, &output)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2), "{result:?}");
        assert_only_finding(
            &output,
            "BLOCKED",
            "prior_trial_balance",
            "PERIOD_ORDER",
            reason,
        );
        assert_eq!(snapshot(&input), before);
    }
}

fn view_read_only(output: &std::path::Path) -> std::process::Output {
    let before = snapshot(output);
    let view = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["view", "--pack-dir"])
        .arg(output)
        .output()
        .unwrap();
    assert_eq!(snapshot(output), before);
    view
}

fn acknowledged_pack(tmp: &TempDir) -> PathBuf {
    let note = tmp.join("review-note.json");
    fs::write(
        &note,
        br#"{"reviewer_initials":"RD","reviewed_on":"2026-04-12","comment":"Fabricated review with | and \\ in text."}"#,
    )
    .unwrap();
    let output = tmp.join("acknowledged");
    let gated = gate_command("bas-not-ready", &output)
        .arg("--review-note")
        .arg(note)
        .output()
        .unwrap();
    assert_eq!(gated.status.code(), Some(2), "{gated:?}");
    assert!(view_read_only(&output).status.success());
    output
}

#[test]
fn viewing_an_acknowledged_pack_preserves_its_not_ready_status() {
    let tmp = TempDir::new();
    let output = acknowledged_pack(&tmp);
    let view = view_read_only(&output);
    assert!(view.status.success(), "{view:?}");
    assert!(view.stderr.is_empty());
    let text = String::from_utf8(view.stdout).unwrap();
    for expected in [
        "**Overall status: NOT_READY**",
        "- Reviewer initials: RD",
        "- Reviewed on: 2026-04-12",
        r"Fabricated review with \| and \\ in text.",
        "does not change readiness or approve a file",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    assert!(pack_json(&output).contains("\"overall_status\": \"NOT_READY\""));
}

#[test]
fn viewing_refuses_edited_acknowledgement_types_effect_and_dates() {
    let tmp = TempDir::new();
    let output = acknowledged_pack(&tmp);
    let original = pack_json(&output);
    for (before, after, diagnostic) in [
        (
            "\"reviewer_initials\": \"RD\"",
            "\"reviewer_initials\": 1",
            "acknowledgement.reviewer_initials must be a string",
        ),
        (
            "\"reviewed_on\": \"2026-04-12\"",
            "\"reviewed_on\": null",
            "acknowledgement.reviewed_on must be a string",
        ),
        (
            "Acknowledgement is evidence of human review only;",
            "Acknowledgement approves this file;",
            "acknowledgement.effect disagrees",
        ),
        ("2026-04-12", "20260412", "extended-format ISO date"),
        ("2026-04-12", "2026-02-30", "is not a date"),
    ] {
        let changed = original.replacen(before, after, 1);
        assert_ne!(changed, original, "{before}");
        fs::write(output.join("readiness-pack.json"), changed).unwrap();
        let view = view_read_only(&output);
        assert_eq!(view.status.code(), Some(1), "{view:?}");
        assert!(view.stdout.is_empty());
        assert_view_says(&view, diagnostic);
    }
}

#[test]
fn viewing_refuses_duplicate_missing_or_disagreeing_source_witnesses() {
    let tmp = TempDir::new();
    let output = tmp.join("pack");
    assert!(
        gate_command("bas-ready", &output)
            .output()
            .unwrap()
            .status
            .success()
    );
    let path = output.join("readiness-summary.md");
    let original = fs::read_to_string(&path).unwrap();
    let evidence = original
        .lines()
        .find(|line| line.starts_with("- `") && line.contains("`): `"))
        .unwrap();
    let without_evidence = original
        .lines()
        .filter(|line| !(line.starts_with("- `") && line.contains("`): `")))
        .collect::<Vec<_>>()
        .join("\n");
    let (prefix, _) = evidence.rsplit_once("`): `").unwrap();
    let wrong_digest = format!("{prefix}`): `{}`", "0".repeat(64));
    for (changed, diagnostic) in [
        (
            format!("{original}{evidence}\n"),
            "more than one evidence line",
        ),
        (without_evidence, "no source-evidence digest lines found"),
        (
            original.replacen(evidence, &wrong_digest, 1),
            "list different source digests",
        ),
        (
            original.replacen(evidence, &evidence.replace("` (`", "` (`changed-"), 1),
            "list different source digests",
        ),
    ] {
        fs::write(&path, &original).unwrap();
        assert!(view_read_only(&output).status.success());
        assert_ne!(changed, original);
        fs::write(&path, changed).unwrap();
        let view = view_read_only(&output);
        assert_eq!(view.status.code(), Some(1), "{view:?}");
        assert!(view.stdout.is_empty());
        assert_view_says(&view, diagnostic);
    }
}

#[test]
fn viewing_refuses_disagreeing_finding_rows_and_delimiters() {
    let tmp = TempDir::new();
    let output = tmp.join("pack");
    assert_eq!(
        gate_command("bas-not-ready", &output)
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let path = output.join("readiness-summary.md");
    let original = fs::read_to_string(&path).unwrap();
    let newline = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let row = original
        .lines()
        .find(|line| line.starts_with("| NOT_READY |") || line.starts_with("| BLOCKED |"))
        .unwrap();
    let (_, after_status) = row[1..].split_once('|').unwrap();
    for (changed, diagnostic) in [
        (
            original.replacen(&format!("| --- | --- | --- | --- | --- |{newline}"), "", 1),
            "no delimiter row",
        ),
        (
            original.replacen(&format!("{row}{newline}"), "", 1),
            "finding counts disagree",
        ),
        (
            original.replacen(row, &format!("|{after_status}"), 1),
            "cells, expected 5",
        ),
    ] {
        fs::write(&path, &original).unwrap();
        assert!(view_read_only(&output).status.success());
        assert_ne!(changed, original);
        fs::write(&path, changed).unwrap();
        let view = view_read_only(&output);
        assert_eq!(view.status.code(), Some(1), "{view:?}");
        assert!(view.stdout.is_empty());
        assert_view_says(&view, diagnostic);
    }
}

#[test]
fn viewing_refuses_a_findings_table_when_the_pack_has_no_findings() {
    let tmp = TempDir::new();
    let output = tmp.join("pack");
    let generated = gate_command("bas-ready", &output).output().unwrap();
    assert!(generated.status.success(), "{generated:?}");
    assert!(view_read_only(&output).status.success());
    let path = output.join("readiness-summary.md");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!("{original}\n| Status | Code | Slot | Repeat | Reason |\n"),
    )
    .unwrap();
    let shown = view_read_only(&output);
    assert_eq!(shown.status.code(), Some(1), "{shown:?}");
    assert!(shown.stdout.is_empty());
    assert_view_says(
        &shown,
        "tabulates findings but readiness-pack.json lists none",
    );
    fs::write(&path, original).unwrap();
    assert!(view_read_only(&output).status.success());
}

#[test]
fn viewing_refuses_ambiguous_finding_tables_and_contradictory_counts() {
    let tmp = TempDir::new();
    let output = tmp.join("pack");
    let generated = gate_command("bas-not-ready", &output).output().unwrap();
    assert_eq!(generated.status.code(), Some(2), "{generated:?}");
    assert!(view_read_only(&output).status.success());
    let path = output.join("readiness-summary.md");
    let original = fs::read_to_string(&path).unwrap();
    let header = "| Status | Code | Slot | Repeat | Reason |";
    assert_eq!(original.matches(header).count(), 1);
    let counts = original
        .lines()
        .find(|line| line.starts_with("- Findings:"))
        .unwrap();
    for (changed, diagnostic) in [
        (
            original.replacen(header, "", 1),
            "expected exactly one findings-table header, found 0",
        ),
        (
            format!("{original}\n{header}\n"),
            "expected exactly one findings-table header, found 2",
        ),
        (
            original.replacen(
                counts,
                "- Findings: 0 total; 0 blocked; 0 not ready; 0 repeats.",
                1,
            ),
            "finding counts disagree: readiness-pack.json says",
        ),
    ] {
        assert_ne!(changed, original);
        fs::write(&path, changed).unwrap();
        let shown = view_read_only(&output);
        assert_eq!(shown.status.code(), Some(1), "{shown:?}");
        assert!(shown.stdout.is_empty());
        assert_view_says(&shown, diagnostic);
        fs::write(&path, &original).unwrap();
        assert!(view_read_only(&output).status.success());
    }
}

#[test]
fn viewing_requires_exactly_one_findings_scope_line() {
    let tmp = TempDir::new();
    let output = tmp.join("pack");
    assert_eq!(
        gate_command("bas-not-ready", &output)
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let path = output.join("readiness-summary.md");
    let original = fs::read_to_string(&path).unwrap();
    let count = original
        .lines()
        .find(|line| line.starts_with("- Findings:"))
        .unwrap();
    for changed in [
        original.replacen(count, "", 1),
        format!("{original}{count}\n"),
    ] {
        fs::write(&path, &original).unwrap();
        assert!(view_read_only(&output).status.success());
        assert_ne!(changed, original);
        fs::write(&path, changed).unwrap();
        let view = view_read_only(&output);
        assert_eq!(view.status.code(), Some(1), "{view:?}");
        assert!(view.stdout.is_empty());
        assert_view_says(&view, "expected exactly one 'Findings' scope line");
    }
}

#[test]
fn viewing_refuses_disagreeing_csv_finding_counts() {
    let tmp = TempDir::new();
    let output = tmp.join("pack");
    assert_eq!(
        gate_command("bas-not-ready", &output)
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert!(view_read_only(&output).status.success());
    let csv = output.join("findings.csv");
    let original_csv = fs::read_to_string(&csv).unwrap();
    fs::write(&csv, format!("{}\n", original_csv.lines().next().unwrap())).unwrap();
    let view = view_read_only(&output);
    assert_eq!(view.status.code(), Some(1), "{view:?}");
    assert!(view.stdout.is_empty());
    assert_view_says(&view, "finding counts disagree");
}

fn gate_requiring(output: &std::path::Path, names: &[&str]) -> std::process::Output {
    let mut command = gate_command("bas-ready", output);
    for name in names {
        command.args(["--require-control", name]);
    }
    command.output().unwrap()
}

#[test]
fn a_required_control_that_did_not_run_is_not_ready() {
    let tmp = TempDir::new();
    let plain = tmp.join("plain");
    let result = gate_requiring(&plain, &[]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert!(pack_json(&plain).contains("\"overall_status\": \"READY\""));
    assert!(!pack_json(&plain).contains("required_controls"));
    assert!(
        !String::from_utf8_lossy(&result.stdout).contains("required control"),
        "{result:?}"
    );
    // tie_out_matrix is another profile's slot, so on a BAS pack it cannot run.
    for name in ["prior_findings", "tie_out_matrix"] {
        let output = tmp.join(name);
        let result = gate_requiring(&output, &[name]);
        assert_eq!(result.status.code(), Some(2), "{name}: {result:?}");
        let pack = pack_json(&output);
        assert!(pack.contains("\"overall_status\": \"NOT_READY\""), "{pack}");
        assert!(pack.contains("REQUIRED_CONTROL_SKIPPED"), "{pack}");
        assert!(pack.contains("\"state\": \"skipped\""), "{pack}");
        assert!(
            String::from_utf8_lossy(&result.stdout)
                .contains(&format!("required control {name}: skipped")),
            "{result:?}"
        );
        let view = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
            .arg("view")
            .arg("--pack-dir")
            .arg(&output)
            .output()
            .unwrap();
        assert_eq!(view.status.code(), Some(0), "{name}: {view:?}");
    }
}

#[test]
fn a_required_control_that_ran_leaves_ready_alone() {
    let tmp = TempDir::new();
    let output = tmp.join("ran");
    let result = gate_requiring(&output, &["open_items", "trial_balance", "open_items"]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let pack = pack_json(&output);
    assert!(pack.contains("\"overall_status\": \"READY\""), "{pack}");
    assert_eq!(
        pack.matches("\"name\": \"open_items\"").count(),
        1,
        "{pack}"
    );
    assert_eq!(pack.matches("\"state\": \"ran\"").count(), 2, "{pack}");
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("required control open_items: ran"),
        "{result:?}"
    );
    let summary = fs::read_to_string(output.join("readiness-summary.md")).unwrap();
    assert!(summary.contains("- Required controls: open_items (ran), trial_balance (ran)."));
}

/// A copy of a passing example pack.
fn pack_copy(tmp: &TempDir, example: &str) -> PathBuf {
    let copy = tmp.join("pack");
    fs::create_dir(&copy).unwrap();
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(example);
    for entry in fs::read_dir(example).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), copy.join(entry.file_name())).unwrap();
    }
    copy
}

fn compare_read_only(
    previous: &std::path::Path,
    current: &std::path::Path,
) -> std::process::Output {
    let before = (snapshot(previous), snapshot(current));
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["compare", "--previous-pack-dir"])
        .arg(previous)
        .arg("--current-pack-dir")
        .arg(current)
        .output()
        .unwrap();
    assert_eq!((snapshot(previous), snapshot(current)), before);
    result
}

#[test]
fn comparison_refuses_different_engagement_types_without_changing_packs() {
    let tmp = TempDir::new();
    let previous = tmp.join("previous");
    let current = tmp.join("current");
    for (profile, example, output) in [
        ("year_end", "year-end-ready", &previous),
        ("month_end", "month-end-ready", &current),
    ] {
        let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join(example);
        let result = fixture_command(profile, &input, output).output().unwrap();
        assert_eq!(result.status.code(), Some(0), "{result:?}");
    }
    assert_compare_refused(
        &previous,
        &current,
        "engagement type",
        "year_end",
        "month_end",
    );
}

#[test]
fn comparison_refuses_different_periods_without_changing_packs() {
    let tmp = TempDir::new();
    let input = pack_copy(&tmp, "bas-ready");
    let previous = tmp.join("previous-bas");
    let current = tmp.join("current-bas");
    let result = fixture_command("bas", &input, &previous).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    for name in ["trial_balance.csv", "self_review.json"] {
        let path = input.join(name);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(
            path,
            text.replace("2026-03-31", "2026-04-30")
                .replace("2026-04-10", "2026-05-10"),
        )
        .unwrap();
    }
    let before = snapshot(&input);
    let result = fixture_command("bas", &input, &current).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_compare_refused(
        &previous,
        &current,
        "period end",
        "2026-03-31",
        "2026-04-30",
    );
    assert_eq!(snapshot(&input), before);
}

#[test]
fn comparison_marks_changed_gst_discrepancies_without_changing_either_not_ready_status() {
    let outputs = TempDir::new();
    let previous = outputs.join("previous");
    let current = outputs.join("current");
    for (balance, bank, output) in [
        ("7500.02", "127500.02", &previous),
        ("7500.03", "127500.03", &current),
    ] {
        let tmp = TempDir::new();
        let input = pack_copy(&tmp, "bas-ready");
        let path = input.join("trial_balance.csv");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(
            path,
            text.replace("7500.00", balance).replace("127500.00", bank),
        )
        .unwrap();
        let before = snapshot(&input);
        let result = fixture_command("bas", &input, output).output().unwrap();
        assert_eq!(result.status.code(), Some(2), "{result:?}");
        assert_eq!(snapshot(&input), before);
        assert!(review_ready_gate::viewer::verify_pack(output).is_ok());
    }
    let result = compare_read_only(&previous, &current);
    assert!(result.status.success(), "{result:?}");
    let document =
        review_ready_gate::probe::decode(&String::from_utf8(result.stdout).unwrap()).unwrap();
    assert!(json_array(document.at("scope_changes")).is_empty());
    for side in ["previous", "current"] {
        assert!(document.at(side).at("overall_status").is_str("NOT_READY"));
    }
    let findings = json_array(document.at("findings"));
    assert_eq!(findings.len(), 1);
    assert!(findings[0].at("code").is_str("TIEOUT_BREAK"));
    assert!(findings[0].at("slot").is_str("gst_control_gl"));
    assert!(findings[0].at("change").is_str("CHANGED"));
    for (side, closing, difference) in [
        ("previous", "7500.02", "0.02"),
        ("current", "7500.03", "0.03"),
    ] {
        let rows = json_array(findings[0].at(side));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].at("status").is_str("NOT_READY"));
        let reason = format!(
            "GST control accounts close at {closing} in the trial balance, but the GST control file moves them by 7500.00 and, with no GST_OPENING on the activity statement, they are taken to carry in nil; difference {difference} exceeds tolerance 0.01. The file may be missing postings, or the accounts carried a balance in, such as an unpaid earlier BAS or the cents the ATO drops from 1A and 1B, which GST_OPENING declares."
        );
        assert!(rows[0].at("reason").is_str(&reason), "{:?}", rows[0]);
    }
}

fn assert_compare_refused(
    previous: &std::path::Path,
    current: &std::path::Path,
    member: &str,
    old: &str,
    new: &str,
) {
    let result = compare_read_only(previous, current);
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(result.stdout.is_empty());
    assert_eq!(
        String::from_utf8(result.stderr).unwrap().trim(),
        format!(
            "review-ready compare: verification failed: compare needs two runs of the same {member}; the previous pack has '{old}' and the current pack has '{new}'"
        )
    );
}

fn intake_command(
    source: &std::path::Path,
    text: &std::path::Path,
    output: &std::path::Path,
) -> Command {
    let mut command = child_profile::command(env!("CARGO_BIN_EXE_review-ready"));
    command
        .args(["intake", "--synthetic", "--source"])
        .arg(source)
        .arg("--text")
        .arg(text)
        .args([
            "--text-origin",
            "manual",
            "--text-version",
            "fabricated-v1",
            "--entity",
            "Cedar and Pine Consulting Pty Ltd",
            "--period-end",
            "2026-03-31",
            "--output",
        ])
        .arg(output)
        .env_remove("GIT_WORK_TREE");
    command
}

#[test]
fn document_intake_enforces_the_exact_extracted_text_byte_limit() {
    let tmp = TempDir::new();
    let source = tmp.join("original.txt");
    let text = tmp.join("text.txt");
    let mut invoice = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/document-intake/invoice.txt"),
    )
    .unwrap();
    fs::write(&source, &invoice).unwrap();
    for size in [0, 1_048_577] {
        let mut bytes = "é".repeat(size / 2).into_bytes();
        bytes.resize(size, b' ');
        fs::write(&text, bytes).unwrap();
        let before = snapshot(&tmp.0);
        let output = tmp.join(format!("refused-{size}"));
        let result = intake_command(&source, &text, &output).output().unwrap();
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        assert!(result.stdout.is_empty());
        assert_eq!(
            String::from_utf8(result.stderr).unwrap().trim(),
            "review-ready intake: text.txt must contain between 1 and 1048576 bytes."
        );
        assert!(!output.exists());
        assert_eq!(snapshot(&tmp.0), before);
    }
    invoice.extend_from_slice("é".repeat((1_048_576 - invoice.len()) / 2).as_bytes());
    invoice.resize(1_048_576, b' ');
    fs::write(&source, &invoice).unwrap();
    fs::write(&text, &invoice).unwrap();
    let before = snapshot(&tmp.0);
    let output = tmp.join("accepted");
    let result = intake_command(&source, &text, &output).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(fs::read(output.join("original.txt")).unwrap(), invoice);
    assert_eq!(fs::read(output.join("extracted.txt")).unwrap(), invoice);
    for (name, bytes_and_mtime) in before {
        assert_eq!(snapshot(&tmp.0).get(&name), Some(&bytes_and_mtime));
    }
}

#[test]
fn document_view_refuses_duplicate_malformed_and_nonobject_json_without_writing() {
    for (json, diagnostic) in [
        (
            r#"{"source":{},"source":{}}"#,
            "Duplicate document JSON member: 'source'.",
        ),
        ("{", "document-intake.json is not valid document JSON."),
        ("[]", "document-intake.json must hold a JSON object."),
    ] {
        let tmp = TempDir::new();
        fs::write(tmp.join("document-intake.json"), json).unwrap();
        let before = snapshot(&tmp.0);
        let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
            .args(["view-document", "--bundle"])
            .arg(&tmp.0)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        assert!(result.stdout.is_empty());
        assert_eq!(
            String::from_utf8(result.stderr).unwrap().trim(),
            format!("review-ready view-document: verification failed: {diagnostic}")
        );
        assert_eq!(snapshot(&tmp.0), before);
    }
}

#[test]
fn an_occupied_payload_directory_refuses_publication_and_allows_a_clean_retry() {
    let tmp = TempDir::new();
    let input = pack_copy(&tmp, "bas-ready");
    let output = tmp.join("output");
    fs::create_dir(&output).unwrap();
    for name in ["readiness-pack.json", "readiness-summary.md"] {
        fs::write(output.join(name), format!("fabricated sentinel: {name}")).unwrap();
    }
    let obstruction = output.join("findings.csv");
    fs::create_dir(&obstruction).unwrap();
    fs::write(obstruction.join("sentinel.txt"), b"preserve me").unwrap();
    let before = (snapshot(&input), snapshot(&output));
    let directory_mtimes = (
        fs::metadata(&output).unwrap().modified().unwrap(),
        fs::metadata(&obstruction).unwrap().modified().unwrap(),
    );
    let result = fixture_command("bas", &input, &output).output().unwrap();
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(result.stdout.is_empty());
    assert_eq!(
        String::from_utf8(result.stderr).unwrap().trim(),
        "review-ready: output error: publication refuses a non-regular or linked file: findings.csv"
    );
    assert_eq!((snapshot(&input), snapshot(&output)), before);
    assert_eq!(
        (
            fs::metadata(&output).unwrap().modified().unwrap(),
            fs::metadata(&obstruction).unwrap().modified().unwrap()
        ),
        directory_mtimes
    );
    assert!(!output.join(".reviewready").exists());
    let retained = tmp.join("retained-findings");
    assert!(obstruction.starts_with(&tmp.0) && retained.starts_with(&tmp.0));
    fs::rename(obstruction, &retained).unwrap();
    let result = fixture_command("bas", &input, &output).output().unwrap();
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let verified = review_ready_gate::viewer::verify_pack(&output).unwrap();
    assert!(verified.document.at("overall_status").is_str("READY"));
    assert_eq!(snapshot(&input), before.0);
    assert_eq!(
        fs::read(retained.join("sentinel.txt")).unwrap(),
        b"preserve me"
    );
}

#[test]
fn either_missing_bas_label_publishes_a_verified_not_ready_report() {
    for row in ["1A,10000.00", "1B,2500.00"] {
        let tmp = TempDir::new();
        let input = pack_copy(&tmp, "bas-ready");
        fs::write(
            input.join("activity_statement.csv"),
            format!("Label,Amount\n{row}\n"),
        )
        .unwrap();
        let before = snapshot(&input);
        let output = tmp.join("output");
        let result = fixture_command("bas", &input, &output).output().unwrap();
        assert_eq!(result.status.code(), Some(2), "{result:?}");
        assert_only_finding(
            &output,
            "NOT_READY",
            "activity_statement",
            "TIEOUT_BREAK",
            "Activity statement is missing label 1A or 1B, so net GST cannot be tied.",
        );
        let verified = review_ready_gate::viewer::verify_pack(&output).unwrap();
        assert!(
            json_array(verified.document.at("findings"))[0]
                .at("reviewer_action")
                .is_str("Return the pack. BAS readiness requires 1A and 1B.")
        );
        assert!(view_read_only(&output).status.success());
        assert_eq!(snapshot(&input), before);
    }
}

#[test]
fn an_empty_bas_directory_produces_a_usable_not_ready_report_without_source_claims() {
    let tmp = TempDir::new();
    let input = tmp.join("empty-input");
    fs::create_dir(&input).unwrap();
    let output = tmp.join("output");
    let result = fixture_command("bas", &input, &output).output().unwrap();
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let verified = review_ready_gate::viewer::verify_pack(&output).unwrap();
    let document = &verified.document;
    assert!(document.at("overall_status").is_str("NOT_READY"));
    for name in ["period_end", "preparer_initials"] {
        assert!(document.at(name).is_str(""));
    }
    assert!(matches!(document.at("source_sha256"), Json::Object(items) if items.is_empty()));
    let findings = json_array(document.at("findings"));
    assert_eq!(findings.len(), 5);
    let mut slots = findings
        .iter()
        .map(|finding| {
            assert!(finding.at("code").is_str("MISSING_ARTEFACT"));
            assert!(finding.at("status").is_str("NOT_READY"));
            finding.at("slot").as_pystr().unwrap().to_utf8().unwrap()
        })
        .collect::<Vec<_>>();
    slots.sort();
    assert_eq!(
        slots,
        [
            "activity_statement",
            "gst_control_gl",
            "open_items",
            "self_review",
            "trial_balance"
        ]
    );
    let skipped = json_array(document.at("controls_not_run"));
    assert_eq!(skipped.len(), 2);
    for slot in ["bank_rec", "prior_findings"] {
        assert!(skipped.iter().any(|row| row.at("slot").is_str(slot)));
    }
    let shown = view_read_only(&output);
    assert!(shown.status.success(), "{shown:?}");
    let summary = String::from_utf8(shown.stdout).unwrap();
    for expected in [
        "- Period end: n/a",
        "- Preparer initials: n/a",
        "No source files were present in the pack directory.",
    ] {
        assert!(summary.contains(expected), "{summary}");
    }
    assert!(fs::read_dir(input).unwrap().next().is_none());
}

fn gate_on(pack: &std::path::Path, profile: &str, output: &std::path::Path, name: &str) -> String {
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .args(["gate", "--profile", profile, "--pack"])
        .arg(pack)
        .args(["--require-control", name, "--output"])
        .arg(output)
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2), "{name}: {result:?}");
    let shown = String::from_utf8_lossy(&result.stdout);
    assert!(
        shown.contains(&format!("required control {name}: failed")),
        "{result:?}"
    );
    pack_json(output)
}

#[test]
fn a_required_control_with_an_empty_file_is_failed_not_skipped() {
    let tmp = TempDir::new();
    let pack_dir = pack_copy(&tmp, "bas-ready");
    fs::write(pack_dir.join("prior_findings.csv"), b"").unwrap();
    let pack = gate_on(&pack_dir, "bas", &tmp.join("failed"), "prior_findings");
    assert!(pack.contains("REQUIRED_CONTROL_FAILED"), "{pack}");
    assert!(pack.contains("\"state\": \"failed\""), "{pack}");
}

#[test]
fn a_required_control_with_a_header_only_file_is_failed_on_every_profile() {
    for (profile, example) in [
        ("bas", "bas-ready"),
        ("month_end", "month-end-ready"),
        ("year_end", "year-end-ready"),
    ] {
        let tmp = TempDir::new();
        let pack_dir = pack_copy(&tmp, example);
        let items = pack_dir.join("open_items.csv");
        let text = fs::read_to_string(&items).unwrap();
        fs::write(&items, format!("{}\n", text.lines().next().unwrap())).unwrap();
        fs::write(
            pack_dir.join("prior_findings.csv"),
            "FindingCode,Slot,Status\n",
        )
        .unwrap();
        for name in ["open_items", "prior_findings"] {
            let output = tmp.join(name);
            let pack = gate_on(&pack_dir, profile, &output, name);
            assert!(
                pack.contains("REQUIRED_CONTROL_FAILED"),
                "{profile}: {pack}"
            );
            assert!(pack.contains("holds no rows"), "{profile}: {pack}");
            assert!(pack.contains("\"overall_status\": \"NOT_READY\""), "{pack}");
        }
    }
}

/// A pack whose `open_items` ran and whose `prior_findings` was skipped, its
/// JSON put through `edit`, and what `view` then says.
fn viewed_after(edit: impl Fn(&str) -> String) -> std::process::Output {
    let tmp = TempDir::new();
    let output = tmp.join("edited");
    gate_requiring(&output, &["open_items", "prior_findings"]);
    let text = pack_json(&output);
    let changed = edit(&text);
    assert_ne!(changed, text);
    fs::write(output.join("readiness-pack.json"), changed).unwrap();
    let view = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .arg("view")
        .arg("--pack-dir")
        .arg(&output)
        .output()
        .unwrap();
    assert_ne!(view.status.code(), Some(0), "{view:?}");
    view
}

fn assert_view_says(view: &std::process::Output, expected: &str) {
    let error = String::from_utf8_lossy(&view.stderr);
    assert!(error.contains(expected), "{error}");
}

fn edited_member(document: &Json, name: &str, replacement: Json) -> Json {
    let Json::Object(mut members) = document.clone() else {
        panic!("Expected a JSON object");
    };
    let (_, value) = members.iter_mut().find(|(key, _)| key.is(name)).unwrap();
    *value = replacement;
    Json::Object(members)
}

fn assert_edited_pack_refused(output: &std::path::Path, document: &Json, diagnostic: &str) {
    fs::write(
        output.join("readiness-pack.json"),
        review_ready_gate::probe::encode(document),
    )
    .unwrap();
    let shown = view_read_only(output);
    assert_eq!(shown.status.code(), Some(1), "{shown:?}");
    assert!(shown.stdout.is_empty());
    assert_view_says(&shown, diagnostic);
}

#[test]
fn unsupported_saved_publication_state_preserves_payloads_and_recovery_evidence() {
    let tmp = TempDir::new();
    let output = tmp.join("output");
    assert!(
        gate_command("bas-ready", &output)
            .output()
            .unwrap()
            .status
            .success()
    );
    let state_path = output.join(".reviewready/state.json");
    let original = fs::read(&state_path).unwrap();
    let state = review_ready_gate::probe::decode(std::str::from_utf8(&original).unwrap()).unwrap();
    assert!(matches!(state.at("transaction"), Json::Null));
    let cases = [
        ("status", Json::str("unknown")),
        ("transaction", Json::str("not-a-transaction")),
        ("status", Json::str("in_progress")),
        ("previous", Json::Null),
        ("previous", Json::Array(vec![])),
        (
            "previous",
            Json::Array(vec![Json::str("bad"), Json::Null, Json::Null]),
        ),
    ];
    for (member, value) in cases {
        let changed = edited_member(&state, member, value);
        fs::write(&state_path, review_ready_gate::probe::encode(&changed)).unwrap();
        let before = snapshot(&output);
        for result in [
            view_read_only(&output),
            gate_command("bas-ready", &output).output().unwrap(),
        ] {
            assert_eq!(result.status.code(), Some(1), "{result:?}");
            assert!(result.stdout.is_empty());
            assert_view_says(&result, "publication state is unsupported");
            assert_eq!(snapshot(&output), before);
        }
        fs::write(&state_path, &original).unwrap();
    }
    assert!(view_read_only(&output).status.success());
}

#[test]
fn viewing_refuses_invalid_status_source_names_and_nonobject_findings() {
    let tmp = TempDir::new();
    let output = tmp.join("output");
    assert!(
        gate_command("bas-ready", &output)
            .output()
            .unwrap()
            .status
            .success()
    );
    let original = pack_json(&output);
    let document = review_ready_gate::probe::decode(&original).unwrap();
    for (status, representation) in [
        (Json::str("unknown"), "'unknown'"),
        (Json::Bool(true), "True"),
        (Json::Bool(false), "False"),
        (Json::Null, "None"),
    ] {
        let changed = edited_member(&document, "overall_status", status);
        assert_edited_pack_refused(&output, &changed, &format!("got {representation}"));
    }
    assert_edited_pack_refused(
        &output,
        &Json::Array(vec![]),
        "top level must be a JSON object",
    );
    let surrogate =
        review_ready_gate::probe::decode(r#"{"\ud800": {"filename":"input.csv","sha256":"bad"}}"#)
            .unwrap();
    let changed = edited_member(&document, "source_sha256", surrogate);
    assert_edited_pack_refused(&output, &changed, "source_sha256 key is not valid Unicode");
    let Json::Object(sources) = document.at("source_sha256") else {
        panic!("source witnesses")
    };
    let (label, info) = sources.first().unwrap();
    for filename in [Json::str(""), Json::Null] {
        let source = Json::Object(vec![(
            label.clone(),
            edited_member(info, "filename", filename),
        )]);
        let changed = edited_member(&document, "source_sha256", source);
        assert_edited_pack_refused(&output, &changed, ".filename must be a non-empty string");
    }
    let changed = edited_member(&document, "findings", Json::Array(vec![Json::Null]));
    assert_edited_pack_refused(&output, &changed, "findings[0] must be an object");
    fs::write(output.join("readiness-pack.json"), original).unwrap();
    assert!(view_read_only(&output).status.success());
}

#[test]
fn viewing_refuses_missing_or_empty_skipped_control_fields_without_writing() {
    let tmp = TempDir::new();
    let input = tmp.join("empty-input");
    fs::create_dir(&input).unwrap();
    let output = tmp.join("output");
    let result = fixture_command("bas", &input, &output).output().unwrap();
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let original = pack_json(&output);
    let document = review_ready_gate::viewer::verify_pack(&output)
        .unwrap()
        .document;
    let skipped = &json_array(document.at("controls_not_run"))[0];
    let Json::Object(members) = skipped else {
        panic!("Expected a skipped control object");
    };
    let missing_filename = Json::Object(
        members
            .iter()
            .filter(|(key, _)| !key.is("filename"))
            .cloned()
            .collect(),
    );
    let mut invalid = vec![(missing_filename, "does not hold the fields".to_owned())];
    for key in ["filename", "reason", "slot"] {
        for value in [Json::Null, Json::str("")] {
            invalid.push((
                edited_member(skipped, key, value),
                format!("controls_not_run[0].{key} must be a non-empty string"),
            ));
        }
    }
    for (row, diagnostic) in invalid {
        let changed = edited_member(&document, "controls_not_run", Json::Array(vec![row]));
        assert_edited_pack_refused(&output, &changed, &diagnostic);
    }
    fs::write(output.join("readiness-pack.json"), original).unwrap();
    assert!(view_read_only(&output).status.success());
    assert!(fs::read_dir(input).unwrap().next().is_none());
}

#[test]
fn viewing_refuses_malformed_required_control_lists_without_writing() {
    let tmp = TempDir::new();
    let output = tmp.join("output");
    let result = gate_requiring(&output, &["open_items", "prior_findings"]);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let original = pack_json(&output);
    let document = review_ready_gate::viewer::verify_pack(&output)
        .unwrap()
        .document;
    for required in [
        Json::Null,
        Json::Array(vec![]),
        Json::Array(vec![Json::object([("name", Json::str("open_items"))])]),
        Json::Array(vec![Json::object([
            ("name", Json::int(1)),
            ("state", Json::str("ran")),
        ])]),
        Json::Array(vec![Json::object([
            ("name", Json::str("open_items")),
            ("state", Json::int(1)),
        ])]),
        Json::Array(vec![Json::object([
            ("name", Json::str("open_items")),
            ("state", Json::str("approved")),
        ])]),
    ] {
        let changed = edited_member(&document, "required_controls", required);
        assert_edited_pack_refused(
            &output,
            &changed,
            "required_controls must be a non-empty list of names and states",
        );
    }
    fs::write(output.join("readiness-pack.json"), original).unwrap();
    assert!(view_read_only(&output).status.success());
}

const RAN: &str = "\"state\": \"ran\"";
const SKIPPED: &str = "\"state\": \"skipped\"";

#[test]
fn a_pack_whose_states_disagree_with_its_findings_fails_view() {
    let swapped = viewed_after(|text| {
        text.replace(RAN, "swap")
            .replace(SKIPPED, RAN)
            .replace("swap", SKIPPED)
    });
    assert_view_says(&swapped, "disagrees with the required control findings");
    let failed = viewed_after(|text| text.replace(SKIPPED, "\"state\": \"failed\""));
    assert_view_says(&failed, "disagrees with the required control findings");
    let all_ran = viewed_after(|text| text.replace(SKIPPED, RAN));
    assert_view_says(&all_ran, "disagrees with the required control findings");
}

/// A summary rewritten to match must not carry READY past the pack's own findings.
#[test]
fn a_ready_status_over_a_required_finding_fails_view() {
    let tmp = TempDir::new();
    let output = tmp.join("edited");
    gate_requiring(&output, &["open_items", "prior_findings"]);
    for (name, status) in [
        ("readiness-pack.json", "\"overall_status\": \"{}\""),
        ("readiness-summary.md", "**Overall status: {}**"),
    ] {
        let path = output.join(name);
        let text = fs::read_to_string(&path).unwrap();
        let (from, to) = (
            status.replace("{}", "NOT_READY"),
            status.replace("{}", "READY"),
        );
        assert!(text.contains(&from), "{name}");
        fs::write(&path, text.replace(&from, &to)).unwrap();
    }
    assert!(pack_json(&output).contains("REQUIRED_CONTROL_SKIPPED"));
    let view = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .arg("view")
        .arg("--pack-dir")
        .arg(&output)
        .output()
        .unwrap();
    assert_ne!(view.status.code(), Some(0), "{view:?}");
    assert_view_says(&view, "overall_status disagrees with the findings");
}

#[test]
fn a_pack_naming_an_unknown_or_repeated_control_fails_view() {
    let invented =
        viewed_after(|text| text.replace("\"name\": \"prior_findings\"", "\"name\": \"made_up\""));
    assert_view_says(&invented, "not a control");
    let repeated = viewed_after(|text| {
        text.replace("\"name\": \"prior_findings\"", "\"name\": \"open_items\"")
    });
    assert_view_says(&repeated, "without repeats");
}

#[test]
fn a_finding_with_no_required_controls_member_fails_view() {
    let view = viewed_after(|text| {
        let start = text.find("\"required_controls\"").unwrap();
        let end = start + text[start..].find(']').unwrap() + 1;
        let rest = text[end..].trim_start_matches([',', '\r', '\n', ' ']);
        format!("{}{rest}", &text[..start])
    });
    assert_view_says(&view, "names no required_controls");
}

#[test]
fn the_new_option_makes_the_shortest_review_note_spellings_ambiguous() {
    for spelling in ["--r", "--re"] {
        let tmp = TempDir::new();
        let result = gate_command("bas-ready", &tmp.join("out"))
            .args([spelling, "note.json"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1), "{spelling}: {result:?}");
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("ambiguous option"), "{spelling}: {error}");
    }
}

#[test]
fn a_name_that_is_no_slot_is_refused_before_any_pack_is_written() {
    let tmp = TempDir::new();
    for name in ["prior-findings", "repeat_findings", ""] {
        let output = tmp.join("refused");
        let result = gate_requiring(&output, &[name]);
        assert_eq!(result.status.code(), Some(1), "{name}: {result:?}");
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("is not a control"), "{name}: {error}");
        assert!(error.contains("prior_findings"), "{error}");
        assert!(!output.join("readiness-pack.json").exists(), "{name}");
    }
}

#[test]
fn a_changed_requirement_is_a_scope_change_between_runs() {
    let tmp = TempDir::new();
    let (previous, current) = (tmp.join("previous"), tmp.join("current"));
    gate_requiring(&previous, &[]);
    gate_requiring(&current, &["open_items"]);
    let result = child_profile::command(env!("CARGO_BIN_EXE_review-ready"))
        .arg("compare")
        .arg("--previous-pack-dir")
        .arg(&previous)
        .arg("--current-pack-dir")
        .arg(&current)
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&result.stdout);
    assert!(
        report.contains("\"member\": \"required_controls\""),
        "{result:?}"
    );
}

#[test]
fn published_pack_inputs_are_refused_before_any_payload_can_be_overwritten() {
    let tmp = TempDir::new();
    let output = tmp.join("pack");
    assert!(
        gate_command("bas-ready", &output)
            .output()
            .unwrap()
            .status
            .success()
    );
    let before = snapshot(&output);
    for flag in ["--review-note", "--document"] {
        for name in review_ready_gate::report::PACK_NAMES {
            assert!(before.contains_key(&PathBuf::from(name)));
            for input in [output.join(name), output.join(".").join(name)] {
                let result = gate_command("bas-ready", &output)
                    .arg(flag)
                    .arg(&input)
                    .output()
                    .unwrap();
                assert_eq!(result.status.code(), Some(1), "{flag} {input:?}");
                assert!(result.stdout.is_empty());
                let error = String::from_utf8(result.stderr).unwrap();
                assert!(error.starts_with("review-ready: output error:"), "{error}");
                assert!(error.contains(flag), "{error}");
                assert!(
                    error.contains("shares a generated pack file name"),
                    "{error}"
                );
                assert_eq!(snapshot(&output), before, "{flag} {input:?}");
            }
        }
    }
}

#[cfg(feature = "probe")]
fn probe_requests(tmp: &TempDir, requests: &str) -> std::process::Output {
    use std::{io::Write, process::Stdio};
    let mut child = child_profile::command(env!("CARGO_BIN_EXE_review-ready-probe"))
        .current_dir(&tmp.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(requests.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[cfg(feature = "probe")]
#[test]
fn probe_profile_refusal_preserves_inputs_and_keeps_the_session_usable() {
    use review_ready_gate::probe::{self, Json};

    let tmp = TempDir::new();
    let absent = tmp.join("absent-pack");
    assert!(!absent.exists());
    let request = Json::object([
        ("op", Json::str("review_pack")),
        ("profile", Json::str("quarter_end")),
        ("pack", Json::str("absent-pack")),
        ("tolerance", Json::str("0.01")),
    ]);
    let requests = format!(
        "{}\n{{\"op\":\"money\",\"value\":\"1\"}}\n",
        probe::encode(&request)
    );
    let result = probe_requests(&tmp, &requests);
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    let response = String::from_utf8(result.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(
        response,
        concat!(
            "{\"error\":\"unknown engagement profile 'quarter_end'; expected one of: bas, month_end, year_end.\"}\n",
            "{\"ok\":\"1\"}\n",
        )
    );
    assert!(!absent.exists());
    assert!(fs::read_dir(&tmp.0).unwrap().next().is_none());
}

#[cfg(feature = "probe")]
#[test]
fn probe_trial_balance_preserves_rows_and_exact_differences() {
    use review_ready_gate::probe;
    let tmp = TempDir::new();
    let csv = concat!(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n",
        "2026-03-31,Fabricated Tenant,Assets,a,Cash,100,10,0,100,0\n",
        "2026-03-31,Fabricated Tenant,Liabilities,b,Payable,200,0,7,0,90\n",
    );
    fs::write(tmp.join("tb.csv"), csv).unwrap();
    let csv_hex: String = csv.bytes().map(|byte| format!("{byte:02x}")).collect();
    let requests = format!(
        concat!(
            "{{\"op\":\"tb_integrity\",\"path\":\"tb.csv\"}}\n",
            "{{\"op\":\"csv\",\"path\":\"tb.csv\",\"hex\":\"{}\",\"kind\":\"unknown\"}}\n",
        ),
        csv_hex
    );
    let result = probe_requests(&tmp, &requests);
    assert!(result.status.success(), "{result:?}");
    assert!(result.stderr.is_empty());
    let stdout = String::from_utf8(result.stdout).unwrap();
    let responses: Vec<_> = stdout
        .lines()
        .map(|line| probe::decode(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);
    let expected = probe::decode(r#"{"movement":"3.00","ytd":"10.00","rows":[
        {"report_date":"2026-03-31","tenant":"Fabricated Tenant","section":"Assets","account_id":"a","account_name":"Cash","account_code":"100","debit":"10","credit":"0","ytd_debit":"100","ytd_credit":"0"},
        {"report_date":"2026-03-31","tenant":"Fabricated Tenant","section":"Liabilities","account_id":"b","account_name":"Payable","account_code":"200","debit":"0","credit":"7","ytd_debit":"0","ytd_credit":"90"}
    ]}"#).unwrap();
    assert_eq!(
        probe::encode(responses[0].at("ok")),
        probe::encode(&expected)
    );
    assert!(
        responses[1]
            .at("error")
            .is_str("unsupported probe CSV kind")
    );
    assert_eq!(fs::read(tmp.join("tb.csv")).unwrap(), csv.as_bytes());
}

#[cfg(feature = "probe")]
#[test]
fn probe_request_errors_precede_a_fatal_protocol_error() {
    use review_ready_gate::probe;
    let tmp = TempDir::new();
    let requests = concat!(
        "{\"op\":\"parse_pack_json\",\"hex\":\"0\"}\n",
        "{\"op\":\"cli\",\"args\":null}\n",
        "{\"op\":\"money\",\"value\":null}\n",
        "{\"op\":\"format_money\",\"value\":\"1\"}\n",
        "{\"op\":\"format_money\",\"value\":\"invalid\"}\n",
        "{\"op\":\"unknown\"}\n",
        "[\n",
    );
    let result = probe_requests(&tmp, requests);
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).starts_with("invalid probe request:"));
    let stdout = String::from_utf8(result.stdout).unwrap();
    let responses: Vec<_> = stdout
        .lines()
        .map(|line| probe::decode(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 6);
    for (index, diagnostic) in [
        (0, "invalid probe hex"),
        (1, "probe CLI arguments must be a list"),
        (2, "Amount"),
        (4, "invalid probe decimal"),
        (5, "unsupported probe operation"),
    ] {
        assert!(
            responses[index]
                .at("error")
                .as_pystr()
                .unwrap()
                .to_utf8()
                .unwrap()
                .contains(diagnostic)
        );
    }
    assert!(responses[3].at("ok").is_str("1.00"));
    assert!(fs::read_dir(&tmp.0).unwrap().next().is_none());
}
