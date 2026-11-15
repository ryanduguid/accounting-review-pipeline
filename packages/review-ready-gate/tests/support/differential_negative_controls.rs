// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

mod temp;

use std::{
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};
use temp::TempDir;

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

struct Captured {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn binary(directory: &Path, name: &str) -> PathBuf {
    directory.join(format!("{name}{}", env::consts::EXE_SUFFIX))
}

fn read_log(path: &Path) -> String {
    assert!(fs::metadata(path).unwrap().len() <= 1024 * 1024);
    fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

fn run(python: &Path, baseline: &Path, suite: &TempDir, label: &str) -> Captured {
    let stdout = suite.join(format!("{label}.stdout"));
    let stderr = suite.join(format!("{label}.stderr"));
    // This comparer returns normally, so keep CI's exit-time profile mode.
    let mut child = Running(
        Command::new(binary(&suite.0, "review-ready-differential"))
            .arg(python)
            .arg(baseline)
            .arg("1")
            .current_dir(&suite.0)
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .expect("start genuine readiness differential"),
    );
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let killed = child.0.kill();
            let reaped = child.0.wait();
            panic!(
                "{label} control deadline; killed: {killed:?}; reaped: {reaped:?}; stdout: {}; stderr: {}",
                read_log(&stdout),
                read_log(&stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    };
    Captured {
        status,
        stdout: read_log(&stdout),
        stderr: read_log(&stderr),
    }
}

fn check_refusal(output: Captured, expected: &str, label: &str) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "{label}: {}\n{}",
        output.stdout,
        output.stderr
    );
    assert!(output.stdout.is_empty(), "{label}: {}", output.stdout);
    assert_eq!(output.stderr, expected, "{label}");
    println!("Expected {label} refusal: {}", expected.trim_end());
}

#[cfg(unix)]
fn response_count(python: &Path, baseline: &Path, suite: &TempDir) {
    use std::os::unix::fs::PermissionsExt;

    let sibling = suite.join("review-ready-probe");
    // Keep wc's real basename on multicall installations and consume all input.
    fs::write(&sibling, "#!/bin/sh\nexec /usr/bin/wc -l\n").unwrap();
    fs::set_permissions(&sibling, fs::Permissions::from_mode(0o700)).unwrap();
    check_refusal(
        run(python, baseline, suite, "response-count"),
        "Error: \"oracle returned 7024 lines for 1 requests\"\n",
        "response-count",
    );
}

#[cfg(unix)]
fn echoing_probe(python: &Path, baseline: &Path, suite: &TempDir) {
    use std::os::unix::fs::PermissionsExt;

    let sibling = suite.join("review-ready-probe");
    fs::write(&sibling, "#!/bin/sh\nexec /usr/bin/cat\n").unwrap();
    fs::set_permissions(&sibling, fs::Permissions::from_mode(0o700)).unwrap();
    let output = run(python, baseline, suite, "echoing-probe");
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}\n{}",
        output.stdout,
        output.stderr
    );
    assert_eq!(
        output.stdout,
        "seed 1: 7024 value/file/CLI cases; 0 CLI presentation differences; 0 recorded bas and year_end coverage differences; 7024 unexplained differences\n",
        "stderr:\n{}",
        output.stderr
    );
    assert_eq!(
        output
            .stderr
            .lines()
            .filter(|line| line.starts_with("difference "))
            .count(),
        12
    );
    assert!(
        output.stderr.ends_with("Error: \"differential failed\"\n"),
        "{}",
        output.stderr
    );
    println!("Expected echoing-probe refusal: 7024 mismatches and 12 diagnostics");
}

fn main() {
    let args: Vec<_> = env::args_os().collect();
    assert_eq!(args.len(), 4, "expected PYTHON BASELINE BIN_DIR");
    #[cfg(unix)]
    let python = Path::new(&args[1]);
    let baseline = Path::new(&args[2]);
    let binaries = Path::new(&args[3]);
    let suite = TempDir::new();
    fs::copy(
        binary(binaries, "review-ready-differential"),
        binary(&suite.0, "review-ready-differential"),
    )
    .unwrap();
    // A missing probe must refuse before even an unavailable Python is started.
    let unavailable_python = binary(&suite.0, "unavailable-python");
    assert!(!unavailable_python.exists());
    check_refusal(
        run(&unavailable_python, baseline, &suite, "missing-sibling"),
        "Error: \"build all probe binaries with cargo build --locked --features probe --bins before running the differential\"\n",
        "missing-sibling",
    );
    #[cfg(unix)]
    response_count(python, baseline, &suite);
    #[cfg(unix)]
    echoing_probe(python, baseline, &suite);
}
