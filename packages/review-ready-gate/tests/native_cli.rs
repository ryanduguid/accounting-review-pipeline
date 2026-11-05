// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use std::{fs, path::PathBuf, process::Command};
#[path = "support/temp.rs"]
mod temp;
use temp::TempDir;

fn gate_command(example: &str, output: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_review-ready"));
    command
        .args(["gate", "--profile", "bas", "--pack"])
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("examples")
                .join(example),
        )
        .arg("--output")
        .arg(output)
        .env_remove("GIT_WORK_TREE");
    command
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
        let mut command = Command::new(env!("CARGO_BIN_EXE_review-ready"));
        command.args(&args).arg(&output);
        if args[0] == "compare" {
            command.arg("--current-pack-dir").arg(&output);
        }
        assert!(command.output().unwrap().status.success());
        assert_eq!(snapshot(&output), before);
    }
    fs::write(output.join("readiness-summary.md"), b"edited").unwrap();
    let before = snapshot(&output);
    let result = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
    let result = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
    let result = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
    let shown = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
    let result = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
    let result = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
    let shown = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
        let result = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
#[cfg(feature = "probe")]
fn seed(path: &std::path::Path) {
    let result = Command::new(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
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
        Command::new(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
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
            let retry = Command::new(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
                .arg(&output)
                .args(["never", "next"])
                .output()
                .unwrap();
            assert!(retry.status.success());
        } else {
            let before = payloads(&output);
            assert!(review_ready_gate::publication::snapshot_revision(&output).is_err());
            let retry = Command::new(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
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
    let other = Command::new(env!("CARGO_BIN_EXE_review-ready-publication-probe"))
        .arg(&output)
        .args(["never", "other"])
        .output()
        .unwrap();
    assert_eq!(other.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&other.stderr).contains("another pack writer holds admission"));
    assert_eq!(payloads(&output), before);
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
        Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
    // bank_rec is another profile's slot, so on a BAS pack it cannot run.
    for name in ["prior_findings", "bank_rec"] {
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
        let view = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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

#[test]
fn a_required_control_with_an_empty_file_is_failed_not_skipped() {
    let tmp = TempDir::new();
    let pack_dir = tmp.join("pack");
    fs::create_dir(&pack_dir).unwrap();
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/bas-ready");
    for entry in fs::read_dir(example).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), pack_dir.join(entry.file_name())).unwrap();
    }
    let slot = review_ready_gate::profiles::slots_for("bas")
        .unwrap()
        .iter()
        .find(|slot| slot.name == "prior_findings")
        .unwrap();
    fs::write(pack_dir.join(slot.filename), b"").unwrap();
    let output = tmp.join("failed");
    let result = Command::new(env!("CARGO_BIN_EXE_review-ready"))
        .args(["gate", "--profile", "bas", "--pack"])
        .arg(&pack_dir)
        .args(["--require-control", "prior_findings", "--output"])
        .arg(&output)
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let pack = pack_json(&output);
    assert!(pack.contains("REQUIRED_CONTROL_FAILED"), "{pack}");
    assert!(pack.contains("\"state\": \"failed\""), "{pack}");
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("required control prior_findings: failed"),
        "{result:?}"
    );
}

#[test]
fn a_pack_edited_to_say_a_skipped_control_ran_fails_view() {
    let tmp = TempDir::new();
    let output = tmp.join("edited");
    gate_requiring(&output, &["prior_findings"]);
    let file = output.join("readiness-pack.json");
    let edited = pack_json(&output).replace("\"state\": \"skipped\"", "\"state\": \"ran\"");
    fs::write(&file, edited).unwrap();
    let view = Command::new(env!("CARGO_BIN_EXE_review-ready"))
        .arg("view")
        .arg("--pack-dir")
        .arg(&output)
        .output()
        .unwrap();
    assert_ne!(view.status.code(), Some(0), "{view:?}");
    assert!(
        String::from_utf8_lossy(&view.stderr).contains("required_controls lists 0"),
        "{view:?}"
    );
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
    let result = Command::new(env!("CARGO_BIN_EXE_review-ready"))
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
