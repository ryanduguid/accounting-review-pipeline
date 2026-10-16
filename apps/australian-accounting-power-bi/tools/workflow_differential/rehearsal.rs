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

    fn pair(&self, command: &str, run: &Path, extra: &[&std::ffi::OsStr], expected: i32) -> Result<()> {
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
        if python.stdout != native.stdout || (expected == 0 && python.stderr != native.stderr) {
            return Err(format!("{command}: CLI output differs."));
        }
        Ok(())
    }
}

pub fn run(settings: &Settings, root: &Path) -> Result<[PathBuf; 2]> {
    let binary = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name(format!("review-workflow{}", std::env::consts::EXE_SUFFIX));
    let commands = Commands { settings, binary };
    let runs = [root.join("python-8"), root.join("python-9")];
    for (index, month) in ["8", "9"].into_iter().enumerate() {
        let native = root.join(format!("rust-{month}"));
        build(&commands, &runs[index], &native, month)?;
        compare_runs(&runs[index], &native)?;
        artifacts(&commands, root, &runs[index], month)?;
    }
    let old = runs[0].as_os_str();
    let mut outputs = Vec::new();
    for native in [false, true] {
        let path = root.join(format!("comparison-{native}.html"));
        commands.call(
            native,
            &[
                "html".as_ref(),
                "--run".as_ref(),
                runs[1].as_os_str(),
                "--bin-dir".as_ref(),
                settings.bin.as_os_str(),
                "--previous".as_ref(),
                old,
                "--output".as_ref(),
                path.as_os_str(),
            ],
            0,
        )?;
        outputs.push(path);
    }
    same_file(&outputs[0], &outputs[1])?;
    semantic_refusals(&commands, root, &runs[1])?;
    argument_statuses(&commands, &runs[1])?;
    archive_refusals(&commands, root)?;
    super::adversarial::run(root, &runs[1], &settings.bin)?;
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

fn build(commands: &Commands<'_>, python: &Path, native: &Path, month: &str) -> Result<()> {
    let mut outputs = Vec::new();
    for (is_native, run) in [(false, python), (true, native)] {
        let args = [
            "build".as_ref(),
            "--run".as_ref(),
            run.as_os_str(),
            "--bin-dir".as_ref(),
            commands.settings.bin.as_os_str(),
            "--month".as_ref(),
            month.as_ref(),
        ];
        outputs.push(commands.call(is_native, &args, 0)?);
        commands.pair("verify", run, &[], 0)?;
    }
    if outputs[0].stdout != outputs[1].stdout || outputs[0].stderr != outputs[1].stderr {
        return Err("Build CLI output differs.".into());
    }
    Ok(())
}

fn artifacts(commands: &Commands<'_>, root: &Path, run: &Path, month: &str) -> Result<()> {
    for (command, suffix) in [("fixtures", "csv"), ("html", "html"), ("export", "zip")] {
        let paths = [
            root.join(format!("python-{month}.{suffix}")),
            root.join(format!("rust-{month}.{suffix}")),
        ];
        let mut outputs = Vec::new();
        for (native, path) in [(false, &paths[0]), (true, &paths[1])] {
            outputs.push(commands.call(
                native,
                &[
                    command.as_ref(),
                    "--run".as_ref(),
                    run.as_os_str(),
                    "--bin-dir".as_ref(),
                    commands.settings.bin.as_os_str(),
                    "--output".as_ref(),
                    path.as_os_str(),
                ],
                0,
            )?);
        }
        if outputs[0].stdout != outputs[1].stdout || outputs[0].stderr != outputs[1].stderr {
            return Err(format!("{command}: CLI output differs."));
        }
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
            commands.pair(
                "verify-export",
                &paths[0],
                &["--month".as_ref(), "99999999999999999999".as_ref()],
                0,
            )?;
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

fn semantic_refusals(commands: &Commands<'_>, root: &Path, source: &Path) -> Result<()> {
    for index in 0..8 {
        let run = root.join(format!("tamper-{index}"));
        let mut receipt = copy_run(source, &run)?;
        if index < 4 {
            let path = run.join("drivers/variance-drivers.json");
            let mut drivers = document(&path)?;
            let account = drivers["accounts"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|account| !account["drivers"].as_array().unwrap().is_empty())
                .unwrap();
            match index {
                0 => {
                    account["drivers"].as_array_mut().unwrap().pop();
                }
                1 => {
                    let duplicate = account["drivers"][0].clone();
                    account["drivers"].as_array_mut().unwrap().push(duplicate);
                }
                2 => account["transactions_total"] = "999.00".into(),
                _ => account["drivers"][0]["Description"] = "altered".into(),
            }
            fs::write(path, serde_json::to_vec(&drivers).unwrap()).map_err(|error| error.to_string())?;
        } else {
            match index {
                4 => receipt["invocations"].as_array_mut().unwrap().swap(1, 2),
                5 => receipt["invocations"][1]["exit"] = 2.into(),
                6 => receipt["invocations"][0]["producer_manifest_sha256"] = "0".repeat(64).into(),
                _ => receipt["case"]["tenant"] = "Foreign".into(),
            }
        }
        reseal(&run, &mut receipt)?;
        commands.pair("verify", &run, &[], 1)?;
    }
    Ok(())
}
