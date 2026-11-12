// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Fabricated native producer used only by the workflow differential rehearsal.

use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn record(directory: &Path, text: &str) -> Result<()> {
    writeln!(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("calls"))?,
        "{text}"
    )?;
    Ok(())
}

fn replace(directory: &Path, executable: &Path) -> Result<()> {
    let replacement = directory.join("replacement");
    fs::copy(executable, &replacement)?;
    OpenOptions::new()
        .append(true)
        .open(&replacement)?
        .write_all(b"fixture replacement")?;
    let displaced = directory.join("displaced");
    fs::rename(executable, &displaced)?;
    if let Err(error) = fs::rename(&replacement, executable) {
        fs::rename(displaced, executable)?;
        return Err(error.into());
    }
    fs::write(directory.join("replacement-done"), b"replaced")?;
    Ok(())
}

fn check_snapshot(directory: &Path, original: &Path, pack: &Path) -> Result<()> {
    let captured = pack.parent().ok_or("Captured run missing")?;
    if captured == original {
        return Err("Export rendered from its live source".into());
    }
    for name in fs::read_to_string(directory.join("sealed-files"))?.lines() {
        if fs::read(captured.join(name))? != fs::read(original.join(name))? {
            return Err(format!("Captured bytes differ: {name}").into());
        }
    }
    fs::write(directory.join("snapshot-checked"), b"all captured bytes checked")?;
    Ok(())
}

/// Copies a prepared tree over a run, so the run changes whilst the workflow reads it.
fn overlay(tree: &Path, target: &Path) -> Result<()> {
    for entry in fs::read_dir(tree)? {
        let entry = entry?;
        let destination = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            overlay(&entry.path(), &destination)?;
        } else {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

fn run() -> Result<i32> {
    // The fixture locates its own executable so the rehearsal can replace that file.
    // nosemgrep: rust.lang.security.current-exe.current-exe
    let executable = std::env::current_exe()?;
    let directory = executable.parent().ok_or("Fixture directory missing")?;
    let command = executable
        .file_stem()
        .ok_or("Fixture command missing")?
        .to_string_lossy();
    // Fixture arguments record the producer invocation; they grant nothing.
    // nosemgrep: rust.lang.security.args-os.args-os
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let first = args.first().map(|argument| argument.to_string_lossy()).unwrap_or_default();
    record(directory, &format!("{command} {first}"))?;
    let mode = fs::read_to_string(directory.join("mode")).unwrap_or_default();
    if first == "--version" && args.len() == 1 {
        // The rehearsal records the real producer's version, so the fixture follows each release.
        let version = match mode.as_str() {
            "wrong-version" => "2.0.0".to_owned(),
            _ => fs::read_to_string(directory.join(format!("version-{command}")))?,
        };
        println!("{command} {version}");
        return Ok(0);
    }
    if command == "close-control" && mode == "garbage-compare" && first == "compare" {
        println!("fabricated text in place of a comparison");
        return Ok(0);
    }
    if command == "review-ready" && mode == "replace-sibling" && first == "gate" {
        let sibling = executable.with_file_name(format!("close-control{}", std::env::consts::EXE_SUFFIX));
        OpenOptions::new()
            .append(true)
            .open(sibling)?
            .write_all(b"fixture replacement")?;
    }
    let code = invoke(directory, &command, &executable, &args, &mode)?;
    if command == "close-control" && mode.strip_prefix("overlay-") == Some(&*first) {
        let plan = fs::read_to_string(directory.join("overlay"))?;
        let (tree, target) = plan.split_once('\n').ok_or("Fixture overlay plan missing")?;
        overlay(Path::new(tree), Path::new(target))?;
    }
    Ok(code)
}

fn invoke(directory: &Path, command: &str, executable: &Path, args: &[OsString], mode: &str) -> Result<i32> {
    if args.first().is_none_or(|argument| argument != "view") {
        if mode == "replace-invoke" && command == "review-ready" && args.first().is_some_and(|arg| arg == "gate") {
            replace(directory, executable)?;
        }
        return delegate(directory, command, args);
    }
    let pack = args
        .windows(2)
        .find(|pair| pair[0] == "--pack-dir")
        .ok_or("Fixture pack missing")?;
    let pack = PathBuf::from(&pack[1]);
    record(directory, &format!("pack {}", pack.display()))?;
    if mode == "refuse" {
        return Ok(1);
    }
    if command == "close-control" && mode == "replace" {
        replace(directory, executable)?;
    }
    let original = fs::read_to_string(directory.join("source-run"))?;
    let original = Path::new(&original);
    if command == "close-control" && mode == "extra" {
        fs::write(original.join("unlisted.txt"), b"fabricated additional evidence")?;
    }
    let restored = if command == "close-control" && mode == "snapshot" {
        check_snapshot(directory, original, &pack)?;
        Some(Saved::mutate(original.join("close/close-summary.md"))?)
    } else {
        None
    };
    let result = delegate(directory, command, args);
    drop(restored);
    result
}

fn delegate(directory: &Path, command: &str, args: &[OsString]) -> Result<i32> {
    let output = Command::new(directory.join(format!("actual-{command}{}", std::env::consts::EXE_SUFFIX)))
        .args(args)
        .output()?;
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;
    Ok(output.status.code().unwrap_or(1))
}

struct Saved(PathBuf, Vec<u8>);

impl Saved {
    fn mutate(path: PathBuf) -> Result<Self> {
        let original = Self(path.clone(), fs::read(&path)?);
        fs::write(path, b"transient fabricated replacement")?;
        Ok(original)
    }
}

impl Drop for Saved {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, &self.1);
    }
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("Native test delegate: {error}");
            std::process::exit(1);
        }
    }
}
