// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Native gate receipts, checked without Power BI Desktop: the real runner and receipt script
//! run beside stub checks, and the settings copy is found in fabricated settings folders.

#![cfg(windows)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use common::app;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SUITES: [&str; 6] = [
    "test_financial_filters",
    "test_benchmark_measures",
    "test_review_controls",
    "test_review_projection",
    "test_review_presentation",
    "measure_report_queries",
];
const BINDING_STUB: &str = "param($InstanceFile, $SourceManifest, $EvidencePath, $SourceRoot, $PowerBIBin)
Write-Output 'Stub binding verified.'
";
const FAILING_BINDING_STUB: &str = "param($InstanceFile, $SourceManifest, $EvidencePath, $SourceRoot, $PowerBIBin)
throw 'Stub binding failed.'
";
const PASSING_STUB: &str = "param($Server, $PowerBIBin, $Samples)
Write-Output 'Stub suite passed.'
";
const FAILING_STUB: &str = "param($Server, $PowerBIBin, $Samples)
[Console]::Error.WriteLine('Stub suite failed.')
exit 3
";

/// Records the driver's own process as Desktop and engine, its start shifted by `ShiftMs`, then
/// runs the runner under the same host, passing `SourceRoot` when given.
const RUN_DRIVER: &str = r#"param([string]$Work, [int]$ShiftMs, [string]$SourceRoot)
$ErrorActionPreference = 'Stop'
$self = Get-CimInstance Win32_Process -Filter "ProcessId = $PID"
$started = $self.CreationDate.AddMilliseconds($ShiftMs).ToString('o')
$instance = Join-Path $Work 'instance.json'
[ordered]@{ project = $Work; server = 'localhost:1'; desktop_pid = $PID; started_at = $started; engine_pid = $PID; engine_started_at = $started } |
    ConvertTo-Json | Set-Content -LiteralPath $instance -Encoding utf8
$root = @(if ($SourceRoot) { '-SourceRoot', $SourceRoot })
$ErrorActionPreference = 'Continue'
& $self.ExecutablePath -NoProfile -File (Join-Path $Work 'tools\test_native_project.ps1') -InstanceFile $instance `
    -SourceManifest (Join-Path $Work 'source-manifest.json') -EvidenceDirectory (Join-Path $Work 'evidence') @root 2>&1 |
    Set-Content -LiteralPath (Join-Path $Work 'runner.log') -Encoding utf8
exit $LASTEXITCODE
"#;

/// Finds the settings copy for process 4242, started at midnight UTC on 1 January 2000, in each
/// case folder. The start passes through JSON as an instance record's does: PowerShell 7 reads it
/// as a local time and Windows PowerShell as a string.
const COPY_DRIVER: &str = r#"param([string]$Tools, [string]$Cases, [string]$Out)
$ErrorActionPreference = 'Stop'
. (Join-Path $Tools 'native_receipts.ps1')
$started = ('{"started_at": "2000-01-01T11:00:00.0000000+11:00"}' | ConvertFrom-Json).started_at
$found = [ordered]@{}
foreach ($case in Get-ChildItem -LiteralPath $Cases -Directory) { $found[$case.Name] = Find-SettingsCopy 4242 $started $case.FullName }
$found | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $Out -Encoding utf8
"#;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("powerbi-receipts-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Windows PowerShell writes UTF-8 with a byte-order mark.
fn read_text(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    text.trim_start_matches('\u{feff}').to_owned()
}

struct Run {
    driver: u32,
    code: Option<i32>,
    summary: Value,
    log: String,
    failing_log: String,
}

/// How a run differs from a passing one.
#[derive(Default)]
struct Case {
    /// The check that fails: a suite writes to stderr and exits 3; the binding check throws.
    failing: Option<&'static str>,
    /// Shifts the recorded start, so the process ID no longer names the recorded process.
    shift_ms: i32,
    /// A `-SourceRoot` inside the scratch folder.
    source_root: Option<&'static str>,
}

/// Copies the real runner and receipt script into `work\tools` beside stub checks.
fn stub_tools(work: &Path, failing: Option<&str>) {
    let tools = work.join("tools");
    fs::create_dir_all(&tools).unwrap();
    for script in ["test_native_project.ps1", "native_receipts.ps1"] {
        fs::copy(app().join("tools").join(script), tools.join(script)).unwrap();
    }
    let binding = if failing == Some("verify_native_model") {
        FAILING_BINDING_STUB
    } else {
        BINDING_STUB
    };
    fs::write(tools.join("verify_native_model.ps1"), binding).unwrap();
    for suite in SUITES {
        let stub = if failing == Some(suite) {
            FAILING_STUB
        } else {
            PASSING_STUB
        };
        fs::write(tools.join(format!("{suite}.ps1")), stub).unwrap();
    }
}

/// Runs the real runner under `host` with stub checks.
fn run(host: &str, case: Case) -> Run {
    let Case {
        failing,
        shift_ms,
        source_root,
    } = case;
    let work = scratch(&format!(
        "{host}-{}-{shift_ms}-{}",
        failing.unwrap_or("passing"),
        source_root.unwrap_or("default")
    ));
    stub_tools(&work, failing);
    fs::write(work.join("driver.ps1"), RUN_DRIVER).unwrap();
    let mut command = Command::new(host);
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(work.join("driver.ps1"))
        .arg("-Work")
        .arg(&work)
        .args(["-ShiftMs", &shift_ms.to_string()]);
    if let Some(root) = source_root {
        command.arg("-SourceRoot").arg(work.join(root));
    }
    let child = command.spawn().unwrap_or_else(|error| panic!("{host}: {error}"));
    let driver = child.id();
    let code = child.wait_with_output().unwrap().status.code();
    let log = read_text(&work.join("runner.log"));
    let summary = serde_json::from_str(&read_text(&work.join("evidence/run-summary.json"))).unwrap();
    let failing_log = failing
        .filter(|name| SUITES.contains(name))
        .map(|name| read_text(&work.join(format!("evidence/{name}.log"))))
        .unwrap_or_default();
    fs::remove_dir_all(&work).unwrap();
    Run {
        driver,
        code,
        summary,
        log,
        failing_log,
    }
}

/// The driver process's executable receipts, which stand in for Desktop and the engine.
fn assert_receipts(host: &str, run: &Run) {
    let summary = &run.summary;
    for role in ["desktop", "engine"] {
        let process = &summary["processes"][role];
        assert_eq!(process["pid"], run.driver, "{host} {role}: {summary}");
        let path = Path::new(process["path"].as_str().unwrap());
        assert!(
            path.file_name().unwrap().eq_ignore_ascii_case(format!("{host}.exe")),
            "{summary}"
        );
        let digest: String = Sha256::digest(fs::read(path).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(process["sha256"], digest.as_str(), "{host} {role}");
        for field in ["file_version", "product_version"] {
            assert!(
                process[field].as_str().is_some_and(|v| !v.is_empty()),
                "{host} {role} {field}: {summary}"
            );
        }
    }
    let desktop = &summary["processes"]["desktop"];
    let family = desktop["package_family"].as_str().unwrap();
    assert!(family == "none" || family.contains('_'), "{family}");
    // The driver writes no Desktop trace, so no settings copy is claimed.
    assert!(
        desktop["settings_trace"].is_null() && desktop["user_zip"].is_null(),
        "{summary}"
    );
}

fn assert_timed(step: &Value) {
    assert!(step["started_at"].is_string() && step["ended_at"].is_string(), "{step}");
}

#[test]
fn a_passing_run_records_every_step_and_both_processes() {
    for host in ["powershell", "pwsh"] {
        let run = run(host, Case::default());
        assert_eq!(run.code, Some(0), "{host}: {}", run.log);
        let summary = &run.summary;
        assert_eq!(summary["result"], "passed", "{host}: {summary}");
        assert!(summary["error"].is_null(), "{summary}");
        assert_timed(summary);
        let steps = summary["steps"].as_array().unwrap();
        let names: Vec<&str> = steps.iter().map(|step| step["name"].as_str().unwrap()).collect();
        let mut expected = vec!["verify_native_model"];
        expected.extend(SUITES);
        expected.push("verify_native_model");
        assert_eq!(names, expected);
        for step in steps {
            assert_timed(step);
            let in_process = step["name"] == "verify_native_model";
            assert_eq!(
                step["exit_code"],
                if in_process { Value::Null } else { json!(0) },
                "{step}"
            );
        }
        assert_receipts(host, &run);
    }
}

/// Windows PowerShell stops at a child's first stderr line under 'Stop' unless the runner guards it.
#[test]
fn a_failing_suite_keeps_its_raw_exit_code_and_output() {
    let run = run(
        "powershell",
        Case {
            failing: Some("test_financial_filters"),
            ..Case::default()
        },
    );
    assert_ne!(run.code, Some(0), "{}", run.log);
    let summary = &run.summary;
    assert_eq!(summary["result"], "failed", "{summary}");
    assert_eq!(
        summary["error"],
        "test_financial_filters failed with exit 3; see its retained log."
    );
    let steps = summary["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "{summary}");
    assert_timed(&steps[1]);
    assert_eq!(steps[1]["exit_code"], 3, "{summary}");
    assert!(run.failing_log.contains("Stub suite failed."), "{}", run.failing_log);
    assert_receipts("powershell", &run);
}

#[test]
fn a_process_id_that_no_longer_names_the_recorded_process_gets_no_receipt() {
    let run = run(
        "powershell",
        Case {
            shift_ms: 1000,
            ..Case::default()
        },
    );
    assert_ne!(run.code, Some(0), "{}", run.log);
    let summary = &run.summary;
    assert_eq!(summary["result"], "failed");
    assert!(
        summary["error"]
            .as_str()
            .unwrap()
            .contains("is not the recorded instance"),
        "{summary}"
    );
    assert!(summary["processes"].is_null(), "{summary}");
    assert_eq!(summary["steps"], json!([]));
}

#[test]
fn a_failing_binding_check_is_timed_and_ends_the_run() {
    for host in ["powershell", "pwsh"] {
        let run = run(
            host,
            Case {
                failing: Some("verify_native_model"),
                ..Case::default()
            },
        );
        assert_ne!(run.code, Some(0), "{host}: {}", run.log);
        let summary = &run.summary;
        assert_eq!(summary["result"], "failed", "{host}: {summary}");
        assert_eq!(summary["error"], "Stub binding failed.", "{host}");
        let steps = summary["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1, "{host}: {summary}");
        assert_eq!(steps[0]["name"], "verify_native_model");
        assert_timed(&steps[0]);
        assert!(steps[0]["exit_code"].is_null(), "{host}: {summary}");
        assert_receipts(host, &run);
    }
}

#[test]
fn a_run_that_cannot_find_its_source_still_writes_its_summary() {
    for host in ["powershell", "pwsh"] {
        let run = run(
            host,
            Case {
                source_root: Some("missing"),
                ..Case::default()
            },
        );
        assert_ne!(run.code, Some(0), "{host}: {}", run.log);
        let summary = &run.summary;
        assert_eq!(summary["result"], "failed", "{host}: {summary}");
        assert!(
            summary["error"].as_str().unwrap().contains(r"\missing"),
            "{host}: {summary}"
        );
        assert_timed(summary);
        assert!(summary["processes"].is_null(), "{host}: {summary}");
        assert_eq!(summary["steps"], json!([]), "{host}");
    }
}

const DESKTOP: &str = r"Microsoft\Power BI Desktop";

/// Writes an empty Desktop trace under `local` for `settings`, last written at `written`.
fn trace(local: &Path, settings: &str, name: &str, written: Option<SystemTime>) {
    let folder = local.join(settings).join(r"Traces\Performance");
    fs::create_dir_all(&folder).unwrap();
    let file = fs::File::create(folder.join(name)).unwrap();
    if let Some(time) = written {
        file.set_modified(time).unwrap();
    }
}

fn packaged(family: &str) -> String {
    format!(r"Packages\{family}\LocalCache\Local\{DESKTOP}")
}

fn found_in(local: &Path, settings: &str) -> Value {
    let root = local.join(settings);
    json!({
        "settings_trace": root.join(r"Traces\Performance\PBIDesktop.4242.a.log").to_str().unwrap(),
        "user_zip": root.join("User.zip").to_str().unwrap(),
    })
}

/// Builds one folder per case under `cases` and returns the copy each should report.
fn copy_cases(cases: &Path) -> Value {
    let none = json!({ "settings_trace": null, "user_zip": null });
    let mut want = serde_json::Map::new();
    let real = cases.join("real");
    trace(&real, DESKTOP, "PBIDesktop.4242.a.log", None);
    want.insert("real".into(), found_in(&real, DESKTOP));
    // One process can leave more than one trace in the copy it uses.
    let two = cases.join("two-traces");
    trace(&two, DESKTOP, "PBIDesktop.4242.a.log", None);
    trace(&two, DESKTOP, "PBIDesktop.4242.b.log", None);
    want.insert("two-traces".into(), found_in(&two, DESKTOP));
    // Inside a package, the unpackaged path also shows the package's trace.
    let inside = cases.join("inside-package");
    trace(&inside, &packaged("Agent_1"), "PBIDesktop.4242.a.log", None);
    trace(&inside, DESKTOP, "PBIDesktop.4242.a.log", None);
    want.insert("inside-package".into(), found_in(&inside, &packaged("Agent_1")));
    // An hour before the recorded start, so an earlier process with the same ID wrote it.
    let earlier = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(946_681_200));
    let reused = cases.join("reused-id");
    trace(&reused, &packaged("Agent_1"), "PBIDesktop.4242.old.log", earlier);
    trace(&reused, DESKTOP, "PBIDesktop.4242.a.log", None);
    want.insert("reused-id".into(), found_in(&reused, DESKTOP));
    let ambiguous = cases.join("ambiguous");
    trace(&ambiguous, &packaged("Agent_1"), "PBIDesktop.4242.a.log", None);
    trace(&ambiguous, &packaged("Agent_2"), "PBIDesktop.4242.a.log", None);
    want.insert("ambiguous".into(), none.clone());
    let missing = cases.join("missing");
    trace(&missing, DESKTOP, "PBIDesktop.42420.a.log", None);
    fs::create_dir_all(missing.join(packaged("Agent_1"))).unwrap();
    want.insert("missing".into(), none);
    Value::Object(want)
}

#[test]
fn the_settings_copy_is_the_one_holding_desktops_own_trace() {
    let work = scratch("copies");
    let cases = work.join("cases");
    let want = copy_cases(&cases);
    fs::write(work.join("driver.ps1"), COPY_DRIVER).unwrap();
    for host in ["powershell", "pwsh"] {
        let out = work.join(format!("found-{host}.json"));
        let status = Command::new(host)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(work.join("driver.ps1"))
            .arg("-Tools")
            .arg(app().join("tools"))
            .arg("-Cases")
            .arg(&cases)
            .arg("-Out")
            .arg(&out)
            .status()
            .unwrap_or_else(|error| panic!("{host}: {error}"));
        assert!(status.success(), "{host}");
        let found: Value = serde_json::from_str(&read_text(&out)).unwrap();
        assert_eq!(found, want, "{host}");
    }
    fs::remove_dir_all(&work).unwrap();
}
