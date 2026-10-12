//! Native guard regression using fabricated CSVs and a disposable script copy.

#![cfg(windows)]

mod common;

use std::fs;
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
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
fn within(mut child: Child, limit: Duration) -> Option<(ExitStatus, String)> {
    let mut stderr = child.stderr.take().expect("piped stderr");
    let reader = thread::spawn(move || {
        let mut text = String::new();
        stderr.read_to_string(&mut text).map(|_| text).unwrap_or_default()
    });
    let mut stdout = child.stdout.take().expect("piped stdout");
    thread::spawn(move || io::copy(&mut stdout, &mut io::sink()));
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("child status") {
            return Some((status, reader.join().expect("stderr reader")));
        }
        if started.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
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
