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
        observe();
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
        "Error: \"comparison expected 7024 responses per process, received 7024 from the oracle and 1 from the native probe\"\n",
        "response-count",
    );
}

#[cfg(unix)]
fn extra_response(python: &Path, baseline: &Path, suite: &TempDir, binaries: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let sibling = suite.join("review-ready-probe");
    fs::copy(
        binary(binaries, "review-ready-probe"),
        suite.join("review-ready-probe-original"),
    )
    .unwrap();
    fs::write(
        &sibling,
        r#"#!/bin/sh
"${0%/*}/review-ready-probe-original" || exit "$?"
printf '%s\n' '{}'
"#,
    )
    .unwrap();
    fs::set_permissions(&sibling, fs::Permissions::from_mode(0o700)).unwrap();
    check_refusal(
        run(python, baseline, suite, "extra-response"),
        "Error: \"comparison expected 7024 responses per process, received 7024 from the oracle and 7025 from the native probe\"\n",
        "extra-response",
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

fn check_recovery(python: &Path, baseline: &Path, suite: &TempDir) {
    let output = run(python, baseline, suite, "timeout-recovery");
    assert_eq!(output.status.code(), Some(0), "{}", output.stderr);
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert_eq!(
        output.stdout,
        "seed 1: 7024 value/file/CLI cases; 21 CLI presentation differences; 50 recorded bas and year_end coverage differences; 0 unexplained differences\n"
    );
    println!("Recovered after oracle timeout: 7024 cases, 0 unexplained differences");
}

fn oracle_timeout(python: &Path, baseline: &Path, suite: &TempDir, binaries: &Path) {
    fs::copy(
        binary(binaries, "review-ready-probe"),
        binary(&suite.0, "review-ready-probe"),
    )
    .unwrap();
    let oracle = TempDir::new();
    let package = oracle.join("reviewready");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("cli.py"), "").unwrap();
    fs::write(
        package.join("__init__.py"),
        "import os, pathlib, time\npathlib.Path(__file__).with_name('oracle.pid').write_text(str(os.getpid()), encoding='utf-8')\ntime.sleep(170)\n",
    )
    .unwrap();
    let marker = package.join("oracle.pid");
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
    extra_response(python, baseline, &suite, binaries);
    #[cfg(unix)]
    echoing_probe(python, baseline, &suite);
    oracle_timeout(python, baseline, &suite, binaries);
}
