// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use super::{Result, Settings, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub struct Temporary(pub PathBuf);

impl Temporary {
    pub fn new() -> Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        // Disposable comparison directory outside the checkout, not a security decision.
        // nosemgrep: rust.lang.security.temp-dir.temp-dir
        let path = std::env::temp_dir().join(format!("power-bi-differential-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Commands<'a> {
    settings: &'a Settings,
    binary: PathBuf,
}

impl Commands<'_> {
    fn call(&self, native: bool, args: &[&std::ffi::OsStr], expected: i32) -> Result<Output> {
        let mut command = if native {
            Command::new(&self.binary)
        } else {
            let mut command = Command::new(&self.settings.python);
            command
                .args(["-I", "-B"])
                .arg(self.settings.oracle.join("tools/review_workflow.py"));
            command
        };
        let output = command.args(args).output().map_err(|error| error.to_string())?;
        if output.status.code() != Some(expected) {
            return Err(format!(
                "{} {args:?}: expected {expected}, got {:?}: {}",
                if native { "Rust" } else { "Python" },
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(output)
    }

    /// Runs one command through both implementations and returns the native output.
    fn pair(&self, command: &str, run: &Path, extra: &[&std::ffi::OsStr], expected: i32) -> Result<Output> {
        let mut args = vec![
            command.as_ref(),
            "--run".as_ref(),
            run.as_os_str(),
            "--bin-dir".as_ref(),
            self.settings.bin.as_os_str(),
        ];
        args.extend_from_slice(extra);
        let python = self.call(false, &args, expected)?;
        let native = self.call(true, &args, expected)?;
        same_output(command, &python, &native, expected == 0)?;
        Ok(native)
    }
}

/// Both implementations print the same; only the wording of a refusal may differ.
fn same_output(command: &str, python: &Output, native: &Output, stderr: bool) -> Result<()> {
    if python.stdout != native.stdout || (stderr && python.stderr != native.stderr) {
        return Err(format!("{command}: CLI output differs."));
    }
    Ok(())
}

/// A refusal counts only when the native workflow gives the reason the case targets.
fn refused_for(native: &Output, reason: &str) -> Result<()> {
    let stderr = String::from_utf8_lossy(&native.stderr);
    if !stderr.contains(reason) {
        return Err(format!("Expected the refusal {reason:?}, got: {stderr}"));
    }
    Ok(())
}

pub fn run(settings: &Settings, root: &Path) -> Result<[PathBuf; 2]> {
    // This comparison binary's sibling is the workflow under test.
    // nosemgrep: rust.lang.security.current-exe.current-exe
    let binary = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name(format!("review-workflow{}", std::env::consts::EXE_SUFFIX));
    let commands = Commands { settings, binary };
    let runs = [root.join("python-8"), root.join("python-9")];
    for (index, month) in ["8", "9"].into_iter().enumerate() {
        let native = root.join(format!("rust-{month}"));
        build(&commands, &runs[index], &native, month, None)?;
        compare_runs(&runs[index], &native)?;
        artifacts(&commands, root, &runs[index], month)?;
    }
    let note = root.join("review-note.json");
    fs::write(&note, REVIEW_NOTE).map_err(|error| error.to_string())?;
    let noted = [root.join("python-note"), root.join("rust-note")];
    build(&commands, &noted[0], &noted[1], "9", Some(&note))?;
    compare_runs(&noted[0], &noted[1])?;
    let old = runs[0].as_os_str();
    let mut outputs = Vec::new();
    for native in [false, true] {
        let path = root.join(format!("comparison-{native}.html"));
        let args = [
            "html".as_ref(),
            "--run".as_ref(),
            runs[1].as_os_str(),
            "--bin-dir".as_ref(),
            settings.bin.as_os_str(),
            "--previous".as_ref(),
            old,
            "--output".as_ref(),
            path.as_os_str(),
        ];
        commands.call(native, &args, 0)?;
        outputs.push(path);
    }
    same_file(&outputs[0], &outputs[1])?;
    semantic_refusals(&commands, root, &runs[1])?;
    unsealed_refusals(&commands, root, &runs[1])?;
    export_refusal(&commands, root, &runs[1])?;
    empty_review_refusal(&commands, root, &runs[1])?;
    argument_statuses(&commands, &runs[1])?;
    archive_refusals(&commands, root)?;
    super::adversarial::run(root, &runs, &settings.bin)?;
    Ok(runs)
}

fn argument_statuses(commands: &Commands<'_>, run: &Path) -> Result<()> {
    for text in ["8", "+08", "0_9", "８", "۹", "999999999999999999999"] {
        commands.pair("verify", run, &["--month".as_ref(), text.as_ref()], 0)?;
    }
    for text in ["bad", "8_", "_8", "8.0", "8__0", "\u{1c}8"] {
        commands.pair("verify", run, &["--month".as_ref(), text.as_ref()], 2)?;
    }
    for native in [false, true] {
        commands.call(native, &[], 2)?;
        commands.call(native, &["--", "--help"].map(std::ffi::OsStr::new), 2)?;
        commands.call(native, &["verify", "--run", "-unknown"].map(std::ffi::OsStr::new), 2)?;
    }
    Ok(())
}

fn archive_refusals(commands: &Commands<'_>, root: &Path) -> Result<()> {
    let original = bytes(&root.join("python-9.zip"))?;
    for (index, offset) in [0, 8, 100, original.len() - 1].into_iter().enumerate() {
        let mut changed = original.clone();
        changed[offset] ^= 1;
        let path = root.join(format!("altered-{index}.zip"));
        fs::write(&path, changed).map_err(|error| error.to_string())?;
        commands.pair("verify-export", &path, &[], 1)?;
    }
    for (index, changed) in [
        original[..original.len() - 1].to_vec(),
        [original.as_slice(), b"trailing"].concat(),
    ]
    .into_iter()
    .enumerate()
    {
        let path = root.join(format!("outside-profile-{index}.zip"));
        fs::write(&path, changed).map_err(|error| error.to_string())?;
        commands.pair("verify-export", &path, &[], 1)?;
    }
    Ok(())
}

fn build(commands: &Commands<'_>, python: &Path, native: &Path, month: &str, note: Option<&Path>) -> Result<()> {
    let mut outputs = Vec::new();
    for (is_native, run) in [(false, python), (true, native)] {
        let mut args = vec![
            "build".as_ref(),
            "--run".as_ref(),
            run.as_os_str(),
            "--bin-dir".as_ref(),
            commands.settings.bin.as_os_str(),
            "--month".as_ref(),
            month.as_ref(),
        ];
        if let Some(note) = note {
            args.extend(["--review-note".as_ref(), note.as_os_str()]);
        }
        outputs.push(commands.call(is_native, &args, 0)?);
        commands.pair("verify", run, &[], 0)?;
    }
    same_output("build", &outputs[0], &outputs[1], true)
}

fn artifacts(commands: &Commands<'_>, root: &Path, run: &Path, month: &str) -> Result<()> {
    for (command, suffix) in [("fixtures", "csv"), ("html", "html"), ("export", "zip")] {
        let paths = [
            root.join(format!("python-{month}.{suffix}")),
            root.join(format!("rust-{month}.{suffix}")),
        ];
        let mut outputs = Vec::new();
        for (native, path) in [(false, &paths[0]), (true, &paths[1])] {
            let args = [
                command.as_ref(),
                "--run".as_ref(),
                run.as_os_str(),
                "--bin-dir".as_ref(),
                commands.settings.bin.as_os_str(),
                "--output".as_ref(),
                path.as_os_str(),
            ];
            outputs.push(commands.call(native, &args, 0)?);
        }
        same_output(command, &outputs[0], &outputs[1], true)?;
        if command == "fixtures" {
            for name in [
                "sample-review-run.csv",
                "sample-review-exceptions.csv",
                "sample-review-evidence.csv",
            ] {
                same_file(&paths[0].join(name), &paths[1].join(name))?;
            }
        } else {
            same_file(&paths[0], &paths[1])?;
        }
        if command == "export" {
            let month = ["--month".as_ref(), "99999999999999999999".as_ref()];
            commands.pair("verify-export", &paths[0], &month, 0)?;
        }
    }
    Ok(())
}

fn bytes(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|error| error.to_string())
}
fn document(path: &Path) -> Result<Value> {
    serde_json::from_slice(&bytes(path)?).map_err(|error| error.to_string())
}
fn digest(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn same_file(left: &Path, right: &Path) -> Result<()> {
    if bytes(left)? != bytes(right)? {
        return Err(format!("File bytes differ: {left:?} / {right:?}"));
    }
    Ok(())
}

fn checked_receipt(run: &Path) -> Result<Value> {
    let data = bytes(&run.join("receipt.json"))?;
    if bytes(&run.join("receipt.sha256"))? != digest(&data).as_bytes() {
        return Err("Receipt digest differs.".into());
    }
    let receipt: Value = serde_json::from_slice(&data).map_err(|error| error.to_string())?;
    for (name, hash) in receipt["files"].as_object().unwrap() {
        if digest(&bytes(&run.join(name))?) != hash.as_str().unwrap() {
            return Err(format!("Sealed hash differs: {name}"));
        }
    }
    Ok(receipt)
}

fn compare_runs(python: &Path, native: &Path) -> Result<()> {
    let (mut left, mut right) = (checked_receipt(python)?, checked_receipt(native)?);
    if left["files"]
        .as_object()
        .unwrap()
        .keys()
        .ne(right["files"].as_object().unwrap().keys())
    {
        return Err("Run inventory differs.".into());
    }
    for name in left["files"].as_object().unwrap().keys() {
        let (a, b) = (bytes(&python.join(name))?, bytes(&native.join(name))?);
        if [
            "validation/close.stdout.txt",
            "validation/drivers.stdout.txt",
            "validation/readiness.stdout.txt",
        ]
        .contains(&name.as_str())
        {
            if normalise(&a, python)? != normalise(&b, native)? {
                return Err(format!("Run bytes differ: {name}"));
            }
        } else if a != b {
            return Err(format!("Run bytes differ: {name}"));
        }
    }
    for receipt in [&mut left, &mut right] {
        receipt.as_object_mut().unwrap().remove("files");
        for invocation in receipt["invocations"].as_array_mut().unwrap() {
            for key in ["started", "finished"] {
                invocation.as_object_mut().unwrap().remove(key);
            }
        }
    }
    if left != right {
        return Err("Receipt differs beyond independently checked files and invocation timestamps.".into());
    }
    Ok(())
}

fn normalise(data: &[u8], root: &Path) -> Result<String> {
    let text = std::str::from_utf8(data).map_err(|error| error.to_string())?;
    let resolved = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let mut roots = vec![
        root.to_string_lossy().into_owned(),
        resolved.to_string_lossy().trim_start_matches("\\\\?\\").to_owned(),
    ];
    roots.sort_by_key(|value| std::cmp::Reverse(value.len()));
    let mut text = text.to_owned();
    for root in roots {
        for spelling in [root.clone(), root.replace('\\', "/"), root.replace('\\', "\\\\")] {
            text = text.replace(&spelling, "<RUN>");
        }
    }
    Ok(text)
}

fn copy_run(source: &Path, target: &Path) -> Result<Value> {
    let receipt = checked_receipt(source)?;
    for name in receipt["files"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .chain(["receipt.json", "receipt.sha256"])
    {
        let path = target.join(name);
        fs::create_dir_all(path.parent().unwrap()).map_err(|error| error.to_string())?;
        fs::write(path, bytes(&source.join(name))?).map_err(|error| error.to_string())?;
    }
    Ok(receipt)
}

fn reseal(run: &Path, receipt: &mut Value) -> Result<()> {
    for (name, hash) in receipt["files"].as_object_mut().unwrap() {
        *hash = digest(&bytes(&run.join(name))?).into();
    }
    let data = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    fs::write(run.join("receipt.sha256"), digest(&data)).map_err(|error| error.to_string())?;
    fs::write(run.join("receipt.json"), data).map_err(|error| error.to_string())
}

const REVIEW_NOTE: &str =
    r#"{"reviewer_initials":"RD","reviewed_on":"2024-10-08","comment":"Fabricated demonstration note."}"#;
const DRIVERS: &str = "drivers/variance-drivers.json";
const CLOSE_PACK: &str = "close/close-review-pack.json";

/// Rewrites one sealed JSON document in place.
fn edit(run: &Path, name: &str, change: impl FnOnce(&mut Value)) -> Result<()> {
    let mut value = document(&run.join(name))?;
    change(&mut value);
    fs::write(run.join(name), serde_json::to_vec(&value).unwrap()).map_err(|error| error.to_string())
}

fn append(run: &Path, name: &str) -> Result<()> {
    let mut data = bytes(&run.join(name))?;
    data.push(b'\n');
    fs::write(run.join(name), data).map_err(|error| error.to_string())
}

/// The first account that has journal rows behind its movement.
fn driven(drivers: &mut Value) -> &mut Value {
    drivers["accounts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|account| !account["drivers"].as_array().unwrap().is_empty())
        .unwrap()
}

fn zeros() -> Value {
    "0".repeat(64).into()
}

type Tamper = fn(&Path, &mut Value) -> Result<()>;

/// Each case changes a copy of a sealed run. The copy is then resealed, so only
/// the workflow's own reading of the content can refuse it, for the reason named.
const TAMPERS: [(&str, Tamper); 17] = [
    ("Driver journal population differs from the source.", |run, _| {
        edit(run, DRIVERS, |drivers| {
            driven(drivers)["drivers"].as_array_mut().unwrap().pop();
        })
    }),
    ("Driver journal population differs from the source.", |run, _| {
        edit(run, DRIVERS, |drivers| {
            let rows = driven(drivers)["drivers"].as_array_mut().unwrap();
            rows.push(rows[0].clone());
        })
    }),
    ("Driver totals differ from the source.", |run, _| {
        edit(run, DRIVERS, |drivers| {
            driven(drivers)["transactions_total"] = "999.00".into()
        })
    }),
    ("Driver journal detail differs from the source.", |run, _| {
        edit(run, DRIVERS, |drivers| {
            driven(drivers)["drivers"][0]["Description"] = "altered".into()
        })
    }),
    ("Required validation invocation did not succeed.", |_, receipt| {
        receipt["invocations"].as_array_mut().unwrap().swap(1, 2);
        Ok(())
    }),
    ("Required validation invocation did not succeed.", |_, receipt| {
        receipt["invocations"][1]["exit"] = 2.into();
        Ok(())
    }),
    ("Receipt invocation producer identity differs.", |_, receipt| {
        receipt["invocations"][0]["producer_manifest_sha256"] = zeros();
        Ok(())
    }),
    ("Case identity differs.", |_, receipt| {
        receipt["case"]["tenant"] = "Foreign".into();
        Ok(())
    }),
    ("Required review artefacts are missing.", |_, receipt| {
        receipt["files"]
            .as_object_mut()
            .unwrap()
            .remove("drivers/variance-drivers.csv");
        Ok(())
    }),
    ("Receipt path is outside the whitelist.", |run, receipt| {
        fs::create_dir(run.join("other")).map_err(|error| error.to_string())?;
        fs::write(run.join("other/extra.txt"), b"fabricated").map_err(|error| error.to_string())?;
        receipt["files"]["other/extra.txt"] = zeros();
        Ok(())
    }),
    ("Ledger and review sources do not tie.", |run, _| {
        append(run, "inputs/trial_balance.csv")
    }),
    ("Fixture provenance differs.", |run, receipt| {
        let sources = receipt["case"]["source_sha256"].as_object().unwrap();
        append(run, &format!("inputs/{}", sources.keys().next().unwrap()))
    }),
    ("Close source binding differs.", |run, _| {
        edit(run, CLOSE_PACK, |pack| {
            pack["source_sha256"]["current_trial_balance"] = zeros()
        })
    }),
    ("Driver source binding differs.", |run, _| {
        edit(run, DRIVERS, |drivers| {
            drivers["source_sha256"]["transactions"] = zeros()
        })
    }),
    ("Driver source binding differs.", |run, _| {
        edit(run, DRIVERS, |drivers| {
            drivers["source_sha256"]["pack:close-summary.md"] = zeros()
        })
    }),
    ("Driver pack: ", |run, _| {
        fs::write(run.join(DRIVERS), b"{").map_err(|error| error.to_string())
    }),
    ("Readiness pack: ", |run, _| {
        fs::write(run.join("readiness/readiness-pack.json"), b"{").map_err(|error| error.to_string())
    }),
];

fn semantic_refusals(commands: &Commands<'_>, root: &Path, source: &Path) -> Result<()> {
    for (index, (reason, tamper)) in TAMPERS.into_iter().enumerate() {
        let run = root.join(format!("tamper-{index}"));
        let mut receipt = copy_run(source, &run)?;
        tamper(&run, &mut receipt)?;
        reseal(&run, &mut receipt)?;
        refused_for(&commands.pair("verify", &run, &[], 1)?, reason)?;
    }
    Ok(())
}

/// A change made after sealing is refused by the seal itself.
fn unsealed_refusals(commands: &Commands<'_>, root: &Path, source: &Path) -> Result<()> {
    for (index, (name, reason)) in [
        ("receipt.sha256", "Receipt digest differs."),
        (
            "close/close-summary.md",
            "Sealed artefact changed: close/close-summary.md",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let run = root.join(format!("unsealed-{index}"));
        copy_run(source, &run)?;
        append(&run, name)?;
        refused_for(&commands.pair("verify", &run, &[], 1)?, reason)?;
    }
    Ok(())
}

/// A resealed run whose close pack raises nothing still verifies, but has no review to project.
fn empty_review_refusal(commands: &Commands<'_>, root: &Path, source: &Path) -> Result<()> {
    let run = root.join("no-findings");
    let mut receipt = copy_run(source, &run)?;
    edit(&run, CLOSE_PACK, |pack| pack["exceptions"] = Value::Array(Vec::new()))?;
    let pack = digest(&bytes(&run.join(CLOSE_PACK))?);
    edit(&run, DRIVERS, |drivers| {
        drivers["source_sha256"]["pack:close-review-pack.json"] = pack.into();
    })?;
    reseal(&run, &mut receipt)?;
    commands.pair("verify", &run, &[], 0)?;
    let output = root.join("no-findings-fixtures");
    let args = ["--output".as_ref(), output.as_os_str()];
    let native = commands.pair("fixtures", &run, &args, 1)?;
    refused_for(&native, "The fixed review case requires at least one finding.")
}

/// A run that verifies but seals fewer files than the fixed case cannot be exported.
fn export_refusal(commands: &Commands<'_>, root: &Path, source: &Path) -> Result<()> {
    let run = root.join("partial-seal");
    let mut receipt = copy_run(source, &run)?;
    receipt["files"]
        .as_object_mut()
        .unwrap()
        .remove("validation/drivers.stderr.txt");
    reseal(&run, &mut receipt)?;
    commands.pair("verify", &run, &[], 0)?;
    let output = root.join("partial-seal.zip");
    let args = ["--output".as_ref(), output.as_os_str()];
    let native = commands.pair("export", &run, &args, 1)?;
    refused_for(&native, "Export requires the exact fixed-case inventory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CASE: &str = "inputs/case.json";
    const LOG: &str = "validation/close.stdout.txt";

    fn output(stdout: &str, stderr: &str) -> Output {
        Output {
            status: std::process::ExitStatus::default(),
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    /// A sealed run holding only the named files; its log names its own folder.
    fn sealed(root: &Path, name: &str, files: &[(&str, &str)], period: &str) -> PathBuf {
        let run = root.join(name);
        let mut hashes = serde_json::Map::new();
        for (file, text) in files {
            let path = run.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text.replace("<RUN>", &run.to_string_lossy())).unwrap();
            hashes.insert((*file).into(), Value::Null);
        }
        let mut receipt = json!({"period":period,"files":hashes,"invocations":[{"command":"close-control","started":name,"finished":name}]});
        reseal(&run, &mut receipt).unwrap();
        run
    }

    #[test]
    fn printed_output_must_match_and_a_refusal_must_give_its_reason() {
        let (python, native) = (output("same", "Python wording"), output("same", "Rust wording"));
        same_output("verify", &python, &native, false).unwrap();
        let differing = same_output("verify", &python, &native, true).unwrap_err();
        assert_eq!(differing, "verify: CLI output differs.");
        assert!(same_output("verify", &python, &output("other", "Python wording"), false).is_err());
        refused_for(&native, "Rust wording").unwrap();
        let unexpected = refused_for(&native, "Receipt digest differs.").unwrap_err();
        assert!(unexpected.ends_with("got: Rust wording"), "{unexpected}");
    }

    #[test]
    fn an_unexpected_exit_status_names_the_implementation_and_what_it_returned() {
        let settings = Settings {
            python: "unused".into(),
            oracle: "unused".into(),
            bin: "unused".into(),
            seed: 16,
        };
        // The rehearsal already needs rustc to compile its fabricated producer.
        let commands = Commands {
            settings: &settings,
            binary: "rustc".into(),
        };
        let version = ["--version".as_ref()];
        assert!(commands.call(true, &version, 0).unwrap().stdout.starts_with(b"rustc "));
        let error = commands.call(true, &version, 1).unwrap_err();
        assert!(
            error.starts_with("Rust [\"--version\"]: expected 1, got Some(0)"),
            "{error}"
        );
    }

    #[test]
    fn fabricated_runs_show_each_difference_the_comparison_reports() {
        let workspace = Temporary::new().unwrap();
        let root = &workspace.0;
        let usual = [(CASE, "fabricated"), (LOG, "Wrote <RUN>")];
        let left = sealed(root, "left", &usual, "2024-09-30");
        let right = sealed(root, "right", &usual, "2024-09-30");
        compare_runs(&left, &right).unwrap();
        same_file(&left.join(CASE), &right.join(CASE)).unwrap();
        let receipts = same_file(&left.join("receipt.json"), &right.join("receipt.json")).unwrap_err();
        assert!(receipts.starts_with("File bytes differ: "), "{receipts}");
        for (name, files, period, expected) in [
            ("fewer", &usual[..1], "2024-09-30", "Run inventory differs."),
            (
                "other-case",
                &[(CASE, "altered"), usual[1]][..],
                "2024-09-30",
                "Run bytes differ: inputs/case.json",
            ),
            (
                "other-log",
                &[usual[0], (LOG, "Rewrote <RUN>")][..],
                "2024-09-30",
                "Run bytes differ: validation/close.stdout.txt",
            ),
            (
                "other-period",
                &usual[..],
                "2024-08-31",
                "Receipt differs beyond independently checked files and invocation timestamps.",
            ),
        ] {
            let other = sealed(root, name, files, period);
            assert_eq!(compare_runs(&left, &other).unwrap_err(), expected);
        }
        fs::write(right.join(CASE), "changed after sealing").unwrap();
        assert_eq!(
            checked_receipt(&right).unwrap_err(),
            "Sealed hash differs: inputs/case.json"
        );
        fs::write(right.join("receipt.sha256"), "0").unwrap();
        assert_eq!(checked_receipt(&right).unwrap_err(), "Receipt digest differs.");
    }
}
