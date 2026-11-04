// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Safety pins on the Excel acceptance runner, `tools/native_excel_acceptance.ps1`.
//!
//! The runner needs desktop Excel and stays PowerShell. These checks read it as text,
//! and 2 run it under Windows PowerShell 5.1 only as far as the refusals it must reach
//! before Excel starts.

mod common;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Source, regex, root};

const RUNNER: &str = "tools/native_excel_acceptance.ps1";

/// How the runner finds its repository, in the order it must run: the empty-string
/// guard, the script's own path, its directory, the default root, the resolved root and
/// the first path built from it.
const ROOT_BINDING: &[&str] = &[
    "[string]::IsNullOrWhiteSpace($RepositoryRoot)",
    "$scriptPath = $MyInvocation.MyCommand.Path",
    "$scriptDirectory = Split-Path -Parent $scriptPath",
    "$RepositoryRoot = Split-Path -Parent $scriptDirectory",
    "$repository = (Resolve-Path -LiteralPath $RepositoryRoot -ErrorAction Stop).ProviderPath",
    "Join-Path $repository",
];

/// The fabricated inputs, registered checks, child-process contract and timing the
/// runner must keep.
const RUNNER_PINS: &[&str] = &[
    "samples\\sample-xero-trial-balance.csv",
    "samples\\sample-xero-trial-balance-columns.csv",
    "[guid]::NewGuid().ToString('D')",
    "[IO.Path]::GetTempPath()",
    "samples\\sample-payday-super-report.csv",
    "PaydaySuper_Report",
    "ZZ_CoreChecks",
    "ZZ_PaydayBaseChecks",
    "ZZ_PaydayExtraCheck",
    "ZZ_PaydayEmployeeNoteCheck",
    "ZZ_PaydayNoProvenanceMaterialisationCheck",
    "ZZ_PaydayDuplicateProvenanceCheck",
    "ZZ_PaydayNonTerminalProvenanceCheck",
    "ZZ_PaydayEmptyProvenanceCheck",
    "ZZ_PaydayShortRowCheck",
    "ZZ_PaydayNoContributionsCheck",
    "ZZ_PaydayUnterminatedQuoteCheck",
    "ZZ_PaydayUnterminatedExtraQuoteCheck",
    "ZZ_PaydayQuotedMultilineCheck",
    "ZZ_PaydayScale500Check",
    "ZZ_PaydayScale5000Check",
    "ZZ_PaydayScale10000Check",
    "$paydayCheckQueries",
    "ExpectedRows = 7",
    "$querySpecifications = @(\n        if ($CheckSet -eq 'Core')",
    "List.Sum(PaydaySuper_Report($mPaydayBadAmount)[sg_amount])",
    "Value.Metadata(PaydaySuper_Report($mPaydayNoProvenance))[PaydaySuperProvenance]",
    "Table.RowCount(PaydaySuper_Report($mPaydayNoProvenance))",
    "Table.RowCount(PaydaySuper_Report($mPaydayShortRow))",
    "attempt[Error][Detail]",
    "Text.Contains(detail, \"CSV record 3\")",
    "$expectedChildCount = if ($childSet -eq 'Core') { 78 } else { 27 }",
    "if ($childRows.Count -ne $expectedChildCount)",
    "if ($rowCount -ne 105)",
    "foreach ($scaleRows in @(500, 5000, 10000))",
    "[Diagnostics.Stopwatch]::StartNew()",
    "ScaleRows = $scaleRows",
    "$payload.Timings",
    "[ValidateSet('All', 'Core', 'Payday')]",
    "-CheckSet $childSet",
    "-ResultPath $resultFile",
    "ConvertFrom-Json -ErrorAction Stop",
    "native acceptance child exited",
    "returned a malformed check row",
    "Core and Payday children ran against different Excel",
];

/// The COM references the `finally` block releases, in reverse order of creation.
const RELEASE_STEPS: &[&str] = &[
    "Release-ComReference $dataBodyRange",
    "Release-ComReference $queryTable",
    "Release-ComReference $listObject",
    "Release-ComReference $listObjects",
    "Release-ComReference $targetRange",
    "Release-ComReference $currentQuery",
    "for ($index = $createdQueries.Count - 1; $index -ge 0; $index--)",
    "Release-ComReference $queries",
    "Release-ComReference $worksheet",
    "Release-ComReference $worksheets",
    "Release-ComReference $workbook",
    "Release-ComReference $workbooks",
    "Release-ComReference $excel",
];

fn runner() -> Source {
    Source::adapter(RUNNER)
}

#[test]
fn native_runner_is_portable_and_uses_only_fabricated_inputs() {
    let source = runner();
    let parameters = regex(r"(?ms)^param\(\s*(.*?)^\)\s*$")
        .captures(&source.text)
        .unwrap_or_else(|| panic!("{RUNNER} has no param block"));
    let block = Source::new("the runner's param block", &parameters[1]);
    // Windows PowerShell 5.1 binds an omitted [string] parameter as the empty string. Its
    // initialiser must stay empty: $PSScriptRoot is itself empty when a -File parameter
    // default is evaluated.
    block.matches(r"(?m)^\s*\[string\]\$RepositoryRoot,?\s*$");
    block.never_matches(r"\$RepositoryRoot\s*=");
    block.lacks_all(&["$PSScriptRoot", "$MyInvocation"]);

    let body = Source::new(
        "the runner after its param block",
        &source.text[parameters.get(0).unwrap().end()..],
    );
    body.has_all(ROOT_BINDING);
    body.in_order(ROOT_BINDING);
    body.in_order(&[
        "throw \"Required repository path is missing: $requiredPath\"",
        "New-Item -ItemType Directory -Path $temporaryDirectory",
        "New-Object -ComObject Excel.Application",
    ]);
    source.never_matches(r"[A-Za-z]:\\Users\\");
    source.has_all(RUNNER_PINS);
    let prefix = "sir-alexander-fitzgerald-native-";
    assert_eq!(
        source.count(prefix),
        3,
        "occurrences of {prefix:?} in {RUNNER}"
    );
    source.lacks_all(&["accounting-excel-toolkit-native-", "WScript.Shell"]);
}

#[test]
fn native_unterminated_quote_checks_pin_the_adapter_error() {
    let source = runner();
    for (needle, expected) in [
        ("Source = New-PaydayMalformedRowErrorCheckM", 2),
        ("-DetailContains 'unterminated quoted field'", 2),
    ] {
        assert_eq!(
            source.count(needle),
            expected,
            "occurrences of {needle:?} in {RUNNER}"
        );
    }
    source.has_all(&[
        "$mExpected = ConvertTo-MText (",
        "{{$mName, $mExpected, actualText, actual = true}}",
    ]);
    source.lacks("\"PaydaySuper.Report / Malformed report row / $mDetailContains\"");
    source.has_all(&[
        "Record.FieldOrDefault(errorRecord, \"Reason\", \"\")",
        "Record.FieldOrDefault(errorRecord, \"Message\", \"\")",
        "Record.FieldOrDefault(errorRecord, \"Detail\", \"\")",
        "and reason = \"PaydaySuper.Report\"",
        "and message = \"Malformed report row\"",
        "and Text.Contains(detail, $mDetailContains)",
    ]);
}

/// `shutil.which("powershell.exe")`: Windows PowerShell 5.1 on PATH. These tests run only
/// on Windows, where it ships with the system, so a missing one fails rather than passing
/// without running.
fn windows_powershell() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("powershell.exe"))
        .find(|candidate| candidate.is_file())
        .expect("Windows PowerShell 5.1 (powershell.exe) is not on PATH")
}

/// Run a copy of the runner, returning its exit code and its stdout followed by stderr.
fn run(
    powershell: &Path,
    script: &Path,
    arguments: &[&OsStr],
    cwd: Option<&Path>,
) -> (Option<i32>, String) {
    let mut command = Command::new(powershell);
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(arguments);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("cannot run {}: {error}", powershell.display()));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), text)
}

#[test]
#[cfg_attr(not(windows), ignore = "requires Windows PowerShell 5.1")]
fn native_child_modes_require_a_private_result_path_before_excel() {
    let powershell = windows_powershell();
    let (status, output) = run(
        &powershell,
        &root().join(RUNNER),
        &[
            OsStr::new("-RepositoryRoot"),
            root().as_os_str(),
            OsStr::new("-CheckSet"),
            OsStr::new("Core"),
        ],
        None,
    );
    assert_eq!(status, Some(1), "{output}");
    let output = Source::new("the runner's output", output);
    output.has("require the private ResultPath argument");
    output.lacks("Desktop Microsoft Excel could not be started");
}

/// A private directory whose name carries spaces, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let temporary = std::env::temp_dir();
        for attempt in 0..100 {
            let path = temporary.join(format!(
                "sir alexander fitzgerald {}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("{}: {error}", path.display()),
            }
        }
        panic!("could not allocate a scratch directory");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `Path.resolve()`: the canonical path, without the `\\?\` prefix Windows adds to it.
fn resolved(path: &Path) -> PathBuf {
    let canonical =
        fs::canonicalize(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    if let Some(plain) = canonical
        .to_str()
        .and_then(|text| text.strip_prefix(r"\\?\"))
    {
        return PathBuf::from(plain);
    }
    canonical
}

fn assert_missing_powerquery(
    case: &str,
    (status, output): (Option<i32>, String),
    repository: &Path,
) {
    // Windows PowerShell 5.1 hard-wraps rendered error records at its host width even
    // when stderr is redirected. Fold those display continuations before comparing the
    // complete canonical path.
    let unwrapped = regex(r"\r?\n\s*").replace_all(&output, "");
    let expected = resolved(repository).join("powerquery");
    assert_eq!(status, Some(1), "{case}: {output}");
    assert!(
        unwrapped.contains(&format!(
            "Required repository path is missing: {}",
            expected.display()
        )),
        "{case}: {output}"
    );
    let output = Source::new(format!("the runner's output ({case})"), output);
    output.lacks_all(&[
        "Cannot bind argument to parameter 'Path'",
        "Desktop Microsoft Excel could not be started",
    ]);
}

#[test]
#[cfg_attr(not(windows), ignore = "requires Windows PowerShell 5.1")]
fn repository_root_binding_reaches_required_path_preflight() {
    let powershell = windows_powershell();
    let script = root().join(RUNNER);
    let scratch = Scratch::new();
    let temporary_root = resolved(&scratch.0);

    // Only the script exists under this default repository root. The missing powerquery
    // directory stops execution before the GUID directory, workbook and
    // Excel.Application are created.
    let default_root = temporary_root.join("default root with spaces");
    let copied_script = default_root
        .join("tools")
        .join("native_excel_acceptance.ps1");
    fs::create_dir_all(copied_script.parent().unwrap()).unwrap();
    fs::copy(&script, &copied_script).unwrap();
    assert_missing_powerquery(
        "omitted parameter",
        run(&powershell, &copied_script, &[], None),
        &default_root,
    );

    let absolute_root = temporary_root.join("absolute root with spaces");
    fs::create_dir(&absolute_root).unwrap();
    assert_missing_powerquery(
        "absolute explicit root",
        run(
            &powershell,
            &script,
            &[OsStr::new("-RepositoryRoot"), absolute_root.as_os_str()],
            None,
        ),
        &absolute_root,
    );

    let relative_root = temporary_root.join("relative root with spaces");
    fs::create_dir(&relative_root).unwrap();
    assert_missing_powerquery(
        "caller-relative explicit root",
        run(
            &powershell,
            &script,
            &[
                OsStr::new("-RepositoryRoot"),
                OsStr::new("relative root with spaces"),
            ],
            Some(&temporary_root),
        ),
        &relative_root,
    );
}

/// The README and the parser both qualify where the Xero layouts came from: dated,
/// fabricated, and not a stable export schema.
#[test]
fn xero_layout_provenance_stays_qualified() {
    for document in ["README.md", "powerquery/Xero.TrialBalance.pq"] {
        let source = Source::adapter(document);
        let prose = Source::new(
            format!("{document} with whitespace folded"),
            source.text.split_whitespace().collect::<Vec<_>>().join(" "),
        );
        prose.has_all(&[
            "20 August 2026",
            "fabricated fixtures",
            "not an official or stable Xero interactive-export schema",
            "fresh non-client export kept outside the repository",
        ]);
        source.lacks_all(&[
            "Xero ships",
            "API / report exports",
            "current UI export",
            "current UI CSV export",
        ]);
    }
}

#[test]
fn native_runner_closes_and_releases_every_com_reference_in_finally() {
    let source = runner();
    source.has_all(&[
        "$excel.AutomationSecurity = 3",
        "$workbook.SaveAs($temporaryWorkbook, 51)",
    ]);
    let finally_body = source.starting_at_last("finally {");
    finally_body.has_all(&[
        "$workbook.Close($false)",
        "$excel.Quit()",
        "Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force",
    ]);
    finally_body.has_all(RELEASE_STEPS);
    finally_body.in_order(RELEASE_STEPS);
}

/// The runner's synopsis and CONTRIBUTING both describe the Payday child's isolation, and
/// they disagreed: 20 queries across 19 files against 21 across 20. Count the registered
/// cases from the script itself rather than trusting either sentence.
#[test]
fn payday_child_counts_match_the_registered_queries_and_fixtures() {
    let source = runner();
    let captured = |pattern: &str| -> Vec<String> {
        regex(pattern)
            .captures_iter(&source.text)
            .map(|found| found[1].to_owned())
            .collect()
    };
    let scale_names = captured(r"\[pscustomobject\]@\{\s*Name\s*=\s*'(ZZ_PaydayScale\d+Check)'");
    // The 3 scale queries are appended by a foreach over their specifications; every
    // other query is a literal registry entry.
    let literal_names: Vec<_> = captured(r"Name\s*=\s*'(ZZ_Payday[A-Za-z0-9]*)'")
        .into_iter()
        .filter(|name| !scale_names.contains(name))
        .collect();
    assert_eq!(scale_names.len(), 3, "{scale_names:?}");
    let query_count = literal_names.len() + scale_names.len();
    assert_eq!(query_count, 21, "{literal_names:?} and {scale_names:?}");
    // Each query reads exactly one fabricated file through its own ConvertTo-MText path
    // variable; ZZ_PaydayNoProvenance* share one.
    let mut fixture_variables = captured(r"(?m)^\s*\$(mPayday[A-Za-z0-9]*)\s*=\s*ConvertTo-MText");
    let fixture_count = fixture_variables.len();
    fixture_variables.sort();
    fixture_variables.dedup();
    assert_eq!(
        fixture_variables.len(),
        fixture_count,
        "a path variable is assigned twice"
    );
    assert_eq!(fixture_count, 20, "{fixture_variables:?}");
    let synopsis = [
        format!("The Payday child uses {query_count} independent"),
        format!("single-source queries across {fixture_count} fabricated files"),
    ];
    source.has_all(&synopsis.each_ref().map(String::as_str));
    Source::adapter("CONTRIBUTING.md").matches(&format!(
        r"each of\s+{query_count} queries reads only one of {fixture_count} fabricated files"
    ));
}
