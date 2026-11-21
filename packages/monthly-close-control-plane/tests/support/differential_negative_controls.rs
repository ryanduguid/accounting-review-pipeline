// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

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
    run_observing(python, baseline, suite, label, || {})
}

fn run_observing(
    python: &Path,
    baseline: &Path,
    suite: &TempDir,
    label: &str,
    mut observe: impl FnMut(),
) -> Captured {
    let stdout = suite.join(format!("{label}.stdout"));
    let stderr = suite.join(format!("{label}.stderr"));
    let mut child = Running(
        // This comparer returns normally; preserve the caller's exit-time profile.
        Command::new(binary(&suite.0, "close-control-differential"))
            .arg(python)
            .arg(baseline)
            .arg("1")
            .current_dir(&suite.0)
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .expect("start genuine monthly differential"),
    );
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        observe();
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "{label} control deadline");
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

    let sibling = suite.join("close-control-probe");
    fs::write(&sibling, "#!/bin/sh\nexec /usr/bin/wc -l\n").unwrap();
    fs::set_permissions(&sibling, fs::Permissions::from_mode(0o700)).unwrap();
    check_refusal(
        run(python, baseline, suite, "response-count"),
        "Error: \"oracle/native response count differs\"\n",
        "response-count",
    );
}

#[cfg(unix)]
fn echoing_probe(python: &Path, baseline: &Path, suite: &TempDir) {
    use std::os::unix::fs::PermissionsExt;

    let sibling = suite.join("close-control-probe");
    fs::write(&sibling, "#!/bin/sh\nexec /usr/bin/cat\n").unwrap();
    fs::set_permissions(&sibling, fs::Permissions::from_mode(0o700)).unwrap();
    let output = run(python, baseline, suite, "echoing-probe");
    assert_eq!(output.status.code(), Some(1), "{}", output.stderr);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert_eq!(
        output
            .stderr
            .lines()
            .filter(|line| line.starts_with("difference "))
            .count(),
        12
    );
    assert!(
        output
            .stderr
            .ends_with("Error: \"36635 unexplained differences in 36635 cases\"\n"),
        "{}",
        output.stderr
    );
    println!("Expected echoing-probe refusal: 36635 mismatches and 12 diagnostics");
}

#[cfg(unix)]
fn assert_oracle_stopped(pid: u32) {
    // Signal zero only queries whether the owned oracle process still exists.
    let result = unsafe { libc::kill(i32::try_from(pid).unwrap(), 0) };
    let error = std::io::Error::last_os_error();
    assert_eq!(result, -1, "oracle {pid} still exists");
    assert_eq!(error.raw_os_error(), Some(libc::ESRCH), "{error}");
}

#[cfg(windows)]
fn assert_oracle_stopped(pid: u32) {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };

    // The owned PID is queried without requesting permission to terminate it.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        let error = std::io::Error::last_os_error();
        assert_eq!(error.raw_os_error(), Some(ERROR_INVALID_PARAMETER as i32));
        return;
    }
    // The non-null process handle stays open through this zero-duration wait.
    let status = unsafe { WaitForSingleObject(process, 0) };
    let closed = unsafe { CloseHandle(process) };
    assert_ne!(closed, 0);
    assert_eq!(status, WAIT_OBJECT_0, "oracle {pid} is still running");
}

fn timeout_oracle(baseline: &Path) -> TempDir {
    let oracle = TempDir::new();
    // The genuine corpus reads these frozen examples before Python starts.
    for relative in [
        "examples/calculation-evidence-coal-lsl-levy.json",
        "examples/calculation-evidence-refused.json",
        "examples/schedules/expenses.json",
        "examples/schedules/migration.json",
        "examples/schedules/interentity.json",
    ] {
        let destination = oracle.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(baseline.join(relative), destination).unwrap();
    }
    let package = oracle.join("closecontrol");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("cli.py"), "").unwrap();
    fs::write(
        package.join("__init__.py"),
        "import os, pathlib, time\npathlib.Path(__file__).with_name('oracle.pid').write_text(str(os.getpid()), encoding='utf-8')\ntime.sleep(170)\n",
    )
    .unwrap();
    oracle
}

fn check_recovery(python: &Path, baseline: &Path, suite: &TempDir) {
    let output = run(python, baseline, suite, "timeout-recovery");
    assert_eq!(output.status.code(), Some(0), "{}", output.stderr);
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert_eq!(
        output.stdout,
        concat!(
            "13 legacy financial scenarios and 8 equity renders satisfy fixed outcome assertions\n",
            "5 legacy balance and financial-year comparisons satisfy fixed outcome assertions\n",
            "8 legacy equity saved packs verify with fixed status assertions\n",
            "seed 1: 36635 value/core/evidence/render cases, 2734 verification cases, 552 driver cases, 258 comparison/queue cases and 835 CLI cases; 0 unexplained differences\n",
        )
    );
    println!("Recovered after oracle timeout: 41014 cases, 0 unexplained differences");
}

fn oracle_timeout(python: &Path, baseline: &Path, suite: &TempDir, binaries: &Path) {
    fs::copy(
        binary(binaries, "close-control-probe"),
        binary(&suite.0, "close-control-probe"),
    )
    .unwrap();
    let oracle = timeout_oracle(baseline);
    let marker = oracle.join("closecontrol/oracle.pid");
    let mut observed = None;
    let output = run_observing(python, &oracle.0, suite, "oracle-timeout", || {
        if observed.is_none() {
            observed = fs::read_to_string(&marker)
                .ok()
                .and_then(|text| text.parse::<u32>().ok())
                .filter(|&pid| pid > 0)
                .map(|pid| (pid, Instant::now()));
        }
    });
    let (pid, started) = observed.expect("owned oracle wrote its PID marker");
    assert!(
        started.elapsed() < Duration::from_secs(150),
        "oracle timeout took {:?}; stderr: {}",
        started.elapsed(),
        output.stderr
    );
    check_refusal(
        output,
        "Error: \"comparison process exceeded its 120-second deadline\"\n",
        "oracle-timeout",
    );
    assert_oracle_stopped(pid);
    check_recovery(python, baseline, suite);
}

fn main() {
    let args: Vec<_> = env::args_os().collect();
    assert_eq!(args.len(), 4, "expected PYTHON BASELINE BIN_DIR");
    let python = Path::new(&args[1]);
    let baseline = Path::new(&args[2]);
    let binaries = Path::new(&args[3]);
    let suite = TempDir::new();
    fs::copy(
        binary(binaries, "close-control-differential"),
        binary(&suite.0, "close-control-differential"),
    )
    .unwrap();
    check_refusal(
        run(python, baseline, &suite, "missing-sibling"),
        "Error: \"build probe binaries before running the differential\"\n",
        "missing-sibling",
    );
    #[cfg(unix)]
    response_count(python, baseline, &suite);
    #[cfg(unix)]
    echoing_probe(python, baseline, &suite);
    oracle_timeout(python, baseline, &suite, binaries);
}
