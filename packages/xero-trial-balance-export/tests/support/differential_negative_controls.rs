// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

#[path = "../../../../tests/support/child_profile.rs"]
mod child_profile;
mod scratch;

use scratch::Scratch;
use std::{
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
use xero_trial_balance_export::pyjson::{self, Json};

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

fn fixture(binaries: &Path, probe: &Path, exporter: &Path) -> Scratch {
    let scratch = Scratch::new();
    for (name, source) in [
        ("xero-tb-probe", probe.to_path_buf()),
        ("export-tb", exporter.to_path_buf()),
        ("xero-tb-auth", binary(binaries, "xero-tb-auth")),
    ] {
        fs::copy(source, binary(&scratch.0, name)).expect("copy genuine fixture executable");
    }
    scratch
}

fn read_log(path: &Path) -> String {
    assert!(
        fs::metadata(path).unwrap().len() <= 1024 * 1024,
        "oversized diagnostic log"
    );
    fs::read_to_string(path)
        .expect("UTF-8 diagnostic log")
        .replace("\r\n", "\n")
}

fn run(
    python: &Path,
    baseline: &Path,
    binaries: &Path,
    suite: &Scratch,
    probe_name: &str,
) -> Captured {
    let stdout = suite.0.join("stdout.log");
    let stderr = suite.0.join("stderr.log");
    let mut command = child_profile::command(binary(binaries, "xero-tb-differential"));
    command
        .arg(python)
        .arg(baseline)
        .arg(binary(&suite.0, probe_name))
        .args(["1", "0"])
        .current_dir(&suite.0)
        .stdin(Stdio::null())
        .stdout(File::create(&stdout).unwrap())
        .stderr(File::create(&stderr).unwrap());
    for key in ["XERO_CLIENT_ID", "XERO_CLIENT_SECRET", "XERO_TOKEN_FILE"] {
        command.env_remove(key);
    }
    let mut child = Running(command.spawn().expect("start real differential"));
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.0.try_wait().expect("observe differential") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "differential negative control deadline"
        );
        thread::sleep(Duration::from_millis(10));
    };
    Captured {
        status,
        stdout: read_log(&stdout),
        stderr: read_log(&stderr),
    }
}

fn check_rejection(output: &Captured, value_differences: usize, cli_differences: usize) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}\n{}",
        output.stdout,
        output.stderr
    );
    assert!(
        output.stdout.contains(&format!(
            "seed 1: 689 cases, {value_differences} differences; baseline "
        )),
        "stdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    assert!(
        output
            .stdout
            .lines()
            .any(|line| { line == format!("CLI: 216 cases, {cli_differences} differences") }),
        "stdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    assert_eq!(
        output.stdout.lines().last(),
        Some("Token lock: Python and Rust exclude each other and release successfully")
    );
    assert_eq!(
        output.stderr,
        "Error: \"unexplained behaviour differences\"\n"
    );
    println!(
        "Expected rejection ({value_differences} value differences, {cli_differences} CLI differences)\n{}Expected stderr: {}",
        output.stdout,
        output.stderr.trim_end()
    );
}

fn mispackaged_suite(python: &Path, baseline: &Path, binaries: &Path) {
    let probe = binary(binaries, "xero-tb-probe");
    let suite = fixture(binaries, &probe, &probe);
    let output = run(python, baseline, binaries, &suite, "xero-tb-probe");
    check_rejection(&output, 0, 108);
    assert_eq!(
        output
            .stdout
            .lines()
            .filter(|line| { line.starts_with("CLI export-tb columns=") })
            .count(),
        20
    );
    assert_eq!(
        output
            .stdout
            .lines()
            .filter(|line| { *line == "Rust (0, \"\", \"\")" })
            .count(),
        20
    );
    assert!(!output.stdout.lines().any(|line| line.starts_with('{')));
    println!("Mispackaged genuine exporter rejected: 689 value cases and 216 CLI cases");
}

#[cfg(target_os = "linux")]
fn echoing_probe(python: &Path, baseline: &Path, binaries: &Path) {
    let cat = Path::new("/bin/cat")
        .canonicalize()
        .expect("real coreutils cat");
    let suite = fixture(binaries, &cat, &binary(binaries, "export-tb"));
    // Multicall coreutils selects cat from the executable name.
    fs::rename(binary(&suite.0, "xero-tb-probe"), binary(&suite.0, "cat"))
        .expect("preserve the real cat invocation name");
    let output = run(python, baseline, binaries, &suite, "cat");
    check_rejection(&output, 689, 0);
    let diagnostics: Vec<_> = output
        .stdout
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| pyjson::loads(line).expect("real JSON mismatch diagnostic"))
        .collect();
    assert_eq!(diagnostics.len(), 20);
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        assert!(pyjson::py_eq(
            diagnostic.at("index"),
            &Json::int(index as i64),
            true
        ));
        assert!(pyjson::py_eq(
            diagnostic.at("rust"),
            diagnostic.at("request"),
            true
        ));
        assert!(!pyjson::py_eq(
            diagnostic.at("rust"),
            diagnostic.at("python"),
            true
        ));
    }
    println!("Real echoing probe rejected: 689 value cases and 216 CLI cases");
}

fn main() {
    let args: Vec<_> = env::args_os().collect();
    assert_eq!(
        args.len(),
        4,
        "usage: differential-negative-controls PYTHON BASELINE BIN_DIR"
    );
    // Preserve the virtual environment entrypoint and its installed dependencies.
    let python = PathBuf::from(&args[1]);
    assert!(
        python.is_absolute() && python.is_file(),
        "absolute Python entrypoint"
    );
    let baseline = Path::new(&args[2])
        .canonicalize()
        .expect("prepared baseline");
    let binaries = Path::new(&args[3])
        .canonicalize()
        .expect("prepared binaries");
    mispackaged_suite(&python, &baseline, &binaries);
    #[cfg(target_os = "linux")]
    echoing_probe(&python, &baseline, &binaries);
}
