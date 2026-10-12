//! Native guard regression using fabricated CSVs and a disposable script copy.

#![cfg(windows)]

mod common;

use std::fs;
use std::process::Command;

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
    let result = Command::new("powershell")
        .args(["-NoProfile", "-File"])
        .arg(&script)
        .args(["-Server", "localhost:1"])
        .output()
        .expect("Windows PowerShell");
    fs::remove_dir_all(&target).unwrap();
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("is missing from the chart of accounts"), "{stderr}");
}
