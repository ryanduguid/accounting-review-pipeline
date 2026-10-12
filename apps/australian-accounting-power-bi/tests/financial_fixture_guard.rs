//! Native guard regression using fabricated CSVs and a disposable script copy.

#![cfg(windows)]

mod common;

use std::fs;
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::app;

#[test]
fn unknown_account_is_refused_before_model_connection() {
    let target = std::env::temp_dir().join(format!("powerbi-guard-{}", std::process::id()));
    let _ = fs::remove_dir_all(&target);
    fs::create_dir_all(target.join("tools")).unwrap();
    fs::create_dir_all(target.join("samples")).unwrap();
    let script = target.join("tools/test_financial_filters.ps1");
    fs::copy(app().join("tools/test_financial_filters.ps1"), &script).unwrap();
    fs::write(
        target.join("samples/sample-chart-of-accounts.csv"),
        "AccountCode,Class,SubClass\n100,Asset,Current Assets\n",
    )
    .unwrap();
    fs::write(
        target.join("samples/sample-general-ledger.csv"),
        "EntityID,PostingDate,AccountCode,Debit,Credit\nFAB001,2026-07-01,999,1.00,0.00\n",
    )
    .unwrap();
    let child = Command::new("powershell")
        .args(["-NoProfile", "-File"])
        .arg(&script)
        .args(["-Server", "localhost:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Windows PowerShell");
    let outcome = within(child, Duration::from_secs(30));
    fs::remove_dir_all(&target).unwrap();
    let (status, stderr) = outcome.expect("the guard must finish within 30 seconds, as the Python suite required");
    assert!(!status.success());
    assert!(stderr.contains("is missing from the chart of accounts"), "{stderr}");
}

/// The child's exit status and stderr, or `None` after killing it at `limit`.
///
/// Like `subprocess.run(..., timeout=...)` with captured output, the limit covers both
/// the child's exit and the end of its pipes, which a descendant can hold open after the
/// child itself has exited.
fn within(mut child: Child, limit: Duration) -> Option<(ExitStatus, String)> {
    let deadline = Instant::now() + limit;
    let (sender, receiver) = mpsc::channel();
    let mut stderr = child.stderr.take().expect("piped stderr");
    thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        let _ = sender.send(text);
    });
    let (stdout_done, stdout_closed) = mpsc::channel();
    let mut stdout = child.stdout.take().expect("piped stdout");
    thread::spawn(move || {
        let _ = io::copy(&mut stdout, &mut io::sink());
        let _ = stdout_done.send(());
    });
    let status = loop {
        if let Some(status) = child.try_wait().expect("child status") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let remaining = || deadline.saturating_duration_since(Instant::now());
    let stderr = status.and_then(|_| receiver.recv_timeout(remaining()).ok());
    let stdout = stderr
        .as_ref()
        .and_then(|_| stdout_closed.recv_timeout(remaining()).ok());
    if let (Some(status), Some(stderr), Some(())) = (status, stderr, stdout) {
        return Some((status, stderr));
    }
    // taskkill /T also ends descendants still attached to the child; then reap the child.
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID"])
        .arg(child.id().to_string())
        .output();
    let _ = child.kill();
    let _ = child.wait();
    None
}

#[test]
fn the_deadline_stops_a_hung_guard() {
    let child = Command::new("powershell")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 20"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Windows PowerShell");
    let started = Instant::now();
    assert!(within(child, Duration::from_secs(1)).is_none());
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// A descendant that inherits stderr and outlives the child must not hold the test
/// past its deadline.
#[test]
fn the_deadline_covers_a_descendant_holding_stderr() {
    let child = Command::new("cmd")
        .args([
            "/C",
            "start",
            "/B",
            "powershell",
            "-NoProfile",
            "-Command",
            "Start-Sleep -Seconds 20",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cmd");
    let started = Instant::now();
    assert!(within(child, Duration::from_secs(2)).is_none());
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// The same for a descendant holding only stdout, its stderr sent elsewhere.
#[test]
fn the_deadline_covers_a_descendant_holding_stdout() {
    let child = Command::new("cmd")
        .args([
            "/C",
            "start",
            "/B",
            "powershell",
            "-NoProfile",
            "-Command",
            "Start-Sleep -Seconds 20",
            "2>NUL",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cmd");
    let started = Instant::now();
    assert!(within(child, Duration::from_secs(2)).is_none());
    assert!(started.elapsed() < Duration::from_secs(10));
}
