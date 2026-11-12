// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Public workflow checks requiring an actual, harmless native producer process.

use australian_accounting_power_bi::workflow::{self, probe};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, String>;

fn io<T>(result: std::io::Result<T>) -> Result<T> {
    result.map_err(|error| error.to_string())
}

/// The workflow must refuse, and for the reason the case targets.
fn refused<T: std::fmt::Debug>(result: Result<T>, reason: &str) {
    let error = result.expect_err("The workflow accepted a case it must refuse");
    assert!(error.contains(reason), "Expected the refusal {reason:?}, got: {error}");
}

fn digest(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn canonical_hash(value: &Value) -> Value {
    let response = probe::evaluate(&json!({"op":"json","text":value.to_string()}));
    assert_eq!(response["ok"], true, "Fixture manifest canonicalisation failed");
    response["value"].clone()
}

fn place(tree: &Path, name: &str, data: &[u8]) -> Result<()> {
    let path = tree.join(name);
    io(fs::create_dir_all(path.parent().unwrap()))?;
    io(fs::write(path, data))
}

struct Fixture {
    root: PathBuf,
    bin: PathBuf,
    delegate: PathBuf,
}

impl Fixture {
    fn new(root: &Path, native_bin: &Path) -> Result<Self> {
        let root = root.join("adversarial");
        let bin = root.join("bin");
        io(fs::create_dir(&root))?;
        io(fs::create_dir(&bin))?;
        let delegate = root.join(format!("delegate{}", std::env::consts::EXE_SUFFIX));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/workflow_differential/native_delegate.rs");
        let output = io(Command::new("rustc")
            .args(["--edition", "2024", "-O", "-D", "warnings"])
            .arg(source)
            .arg("-o")
            .arg(&delegate)
            .output())?;
        let errors = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "Native delegate compilation: {errors}");
        for command in ["close-control", "review-ready"] {
            let name = format!("{command}{}", std::env::consts::EXE_SUFFIX);
            io(fs::copy(native_bin.join(&name), bin.join(format!("actual-{name}"))))?;
        }
        Ok(Self { root, bin, delegate })
    }

    fn executable(&self, command: &str) -> PathBuf {
        self.bin.join(format!("{command}{}", std::env::consts::EXE_SUFFIX))
    }

    fn prepare(&self, source: &Path, name: &str, mode: &str) -> Result<PathBuf> {
        for command in ["close-control", "review-ready"] {
            io(fs::copy(&self.delegate, self.executable(command)))?;
        }
        for name in ["calls", "displaced", "overlay", "replacement-done", "snapshot-checked"] {
            let path = self.bin.join(name);
            if path.exists() {
                io(fs::remove_file(path))?;
            }
        }
        let run = self.root.join(name);
        let mut receipt = workflow::verify(source)?.receipt;
        copy_run(source, &run, &receipt)?;
        let names: Vec<_> = receipt["files"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .chain(["receipt.json", "receipt.sha256"])
            .collect();
        io(fs::write(self.bin.join("sealed-files"), names.join("\n")))?;
        io(fs::write(self.bin.join("source-run"), run.to_string_lossy().as_bytes()))?;
        io(fs::write(self.bin.join("mode"), mode))?;
        for command in ["close-control", "review-ready"] {
            let version = receipt["producer_manifests"][command]["native"]["version"]
                .as_str()
                .ok_or("Receipt producer version missing")?;
            io(fs::write(self.bin.join(format!("version-{command}")), version))?;
        }
        reseal(&run, &mut receipt, &digest(&io(fs::read(&self.delegate))?))?;
        workflow::verify(&run)?;
        Ok(run)
    }

    /// Names the tree an `overlay-` mode copies over `target` and returns where to build it.
    fn overlay(&self, name: &str, target: &Path) -> Result<PathBuf> {
        let tree = self.root.join(name);
        let plan = format!("{}\n{}", tree.to_string_lossy(), target.to_string_lossy());
        io(fs::write(self.bin.join("overlay"), plan))?;
        Ok(tree)
    }

    fn calls(&self) -> Result<String> {
        io(fs::read_to_string(self.bin.join("calls")))
    }

    fn captured(&self, source: &Path) -> Result<PathBuf> {
        let calls = self.calls()?;
        let packs: Vec<_> = calls
            .lines()
            .filter_map(|line| line.strip_prefix("pack "))
            .map(PathBuf::from)
            .collect();
        assert!(!packs.is_empty(), "Producer view was never invoked");
        let captured = packs[0].parent().ok_or("Captured parent missing")?.to_owned();
        assert!(
            captured != source && packs.iter().all(|path| path.parent() == Some(captured.as_path())),
            "Display did not use one captured snapshot"
        );
        assert!(
            !captured.exists(),
            "Export snapshot directory survived completion/refusal"
        );
        Ok(captured)
    }
}

fn reseal(run: &Path, receipt: &mut Value, hash: &str) -> Result<()> {
    for command in ["close-control", "review-ready"] {
        let manifest = &mut receipt["producer_manifests"][command];
        manifest["launcher_sha256"] = hash.into();
        let manifest_hash = canonical_hash(manifest);
        for invocation in receipt["invocations"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .filter(|item| item["command"] == command)
        {
            invocation["executable_sha256"] = hash.into();
            invocation["producer_manifest_sha256"] = manifest_hash.clone();
        }
    }
    let data = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    io(fs::write(run.join("receipt.json"), &data))?;
    io(fs::write(run.join("receipt.sha256"), digest(&data)))
}

fn copy_run(source: &Path, target: &Path, receipt: &Value) -> Result<()> {
    for name in receipt["files"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .chain(["receipt.json", "receipt.sha256"])
    {
        let destination = target.join(name);
        io(fs::create_dir_all(destination.parent().unwrap()))?;
        io(fs::copy(source.join(name), destination))?;
    }
    Ok(())
}

fn pinned_refusals(fixture: &Fixture, source: &Path) -> Result<()> {
    let run = fixture.prepare(source, "hash-mismatch", "")?;
    let executable = fixture.executable("close-control");
    let mut changed = io(fs::read(&executable))?;
    changed.extend_from_slice(b"fabricated changed identity");
    io(fs::write(executable, changed))?;
    refused(workflow::render_html(&run, &fixture.bin, None), "hash differs");
    assert!(!fixture.bin.join("calls").exists(), "Hash mismatch executed a producer");
    let run = fixture.prepare(source, "wrong-version", "wrong-version")?;
    refused(workflow::render_html(&run, &fixture.bin, None), "identity differs");
    let viewed = fixture.calls()?.lines().any(|line| line.ends_with(" view"));
    assert!(!viewed, "Identity mismatch executed producer view");
    let run = fixture.prepare(source, "producer-refusal", "refuse")?;
    refused(workflow::render_html(&run, &fixture.bin, None), "verification refused");
    let viewed = fixture.calls()?.contains("close-control view");
    assert!(viewed, "Refusal did not execute the actual invocation");
    Ok(())
}

/// A producer that replaces its own executable whilst running is refused, on display and on build.
fn identity_replacement(fixture: &Fixture, source: &Path) -> Result<()> {
    let run = fixture.prepare(source, "identity-replacement", "replace")?;
    let result = workflow::render_html(&run, &fixture.bin, None);
    let replaced = fixture.bin.join("replacement-done");
    assert!(replaced.exists(), "Identity replacement did not occur");
    refused(result, "changed during invocation");
    fixture.prepare(source, "build-replacement-seed", "replace-invoke")?;
    let result = workflow::build(&fixture.root.join("build-replacement"), &fixture.bin, 9, None);
    assert!(replaced.exists(), "Build identity replacement did not occur");
    refused(result, "changed during invocation");
    Ok(())
}

/// A build stops when a producer changes, refuses, or its outputs change under it.
fn build_refusals(fixture: &Fixture, source: &Path) -> Result<()> {
    let build = |name: &str| workflow::build(&fixture.root.join(name), &fixture.bin, 9, None);
    fixture.prepare(source, "sibling-seed", "replace-sibling")?;
    refused(build("sibling-build"), "installed identity differs from the receipt");
    fixture.prepare(source, "refused-seed", "refuse")?;
    refused(
        build("refused-build"),
        "review-ready readiness-view refused the case (exit 1)",
    );
    fixture.prepare(source, "close-view-seed", "overlay-view")?;
    let tree = fixture.overlay("close-view-tree", &fixture.root.join("close-view-build"))?;
    place(&tree, "close/close-summary.md", b"fabricated late change")?;
    refused(build("close-view-build"), "Close outputs changed during verification");
    fixture.prepare(source, "provenance-seed", "overlay-drivers")?;
    let tree = fixture.overlay("provenance-tree", &fixture.root.join("provenance-build"))?;
    let pack = io(fs::read(source.join("close/close-review-pack.json")))?;
    let mut pack: Value = serde_json::from_slice(&pack).map_err(|error| error.to_string())?;
    pack["source_sha256"]["current_trial_balance"] = "0".repeat(64).into();
    place(&tree, "close/close-review-pack.json", pack.to_string().as_bytes())?;
    refused(
        build("provenance-build"),
        "Close provenance or context differs from the case",
    );
    Ok(())
}

/// A comparison needs the same producer, a comparison it can read and a previous run that stays put.
fn comparison_refusals(fixture: &Fixture, [previous, current]: &[PathBuf; 2]) -> Result<()> {
    let display = |run: &Path, old: &Path| workflow::render_html(run, &fixture.bin, Some(old));
    let run = fixture.prepare(current, "comparison-identity", "")?;
    refused(
        display(&run, previous),
        "Comparison producer installed identities differ",
    );
    let old = fixture.prepare(previous, "comparison-garbage-previous", "")?;
    let run = fixture.prepare(current, "comparison-garbage", "garbage-compare")?;
    refused(display(&run, &old), "Comparison: ");
    let old = fixture.prepare(previous, "comparison-swap-previous", "")?;
    let run = fixture.prepare(current, "comparison-swap", "overlay-compare")?;
    let tree = fixture.overlay("comparison-swap-tree", &old)?;
    copy_run(&run, &tree, &workflow::verify(&run)?.receipt)?;
    refused(display(&run, &old), "Previous run changed during display");
    Ok(())
}

/// A run that changes whilst a producer reads it is neither displayed nor exported.
fn changed_whilst_read(fixture: &Fixture, [previous, current]: &[PathBuf; 2]) -> Result<()> {
    let run = fixture.prepare(current, "display-swap", "overlay-view")?;
    let tree = fixture.overlay("display-swap-tree", &run)?;
    copy_run(previous, &tree, &workflow::verify(previous)?.receipt)?;
    refused(
        workflow::render_html(&run, &fixture.bin, None),
        "Current run changed during display",
    );
    let run = fixture.prepare(current, "export-reencoded", "overlay-view")?;
    let tree = fixture.overlay("export-reencoded-tree", &run)?;
    let receipt = workflow::verify(&run)?.receipt.to_string().into_bytes();
    place(&tree, "receipt.json", &receipt)?;
    place(&tree, "receipt.sha256", digest(&receipt).as_bytes())?;
    let output = fixture.root.join("reencoded.zip");
    refused(
        workflow::export(&run, &fixture.bin, &output),
        "Run changed during export",
    );
    assert!(
        !output.exists(),
        "Export published a run that changed whilst it was read"
    );
    Ok(())
}

fn snapshot_export(fixture: &Fixture, source: &Path) -> Result<()> {
    let run = fixture.prepare(source, "snapshot-isolation", "snapshot")?;
    let before = workflow::verify(&run)?;
    let output = fixture.root.join("snapshot.zip");
    workflow::export(&run, &fixture.bin, &output)?;
    fixture.captured(&run)?;
    let checked = fixture.bin.join("snapshot-checked").exists();
    assert!(checked, "Captured bytes were not checked during rendering");
    let after = workflow::verify(&run)?;
    assert!(after == before, "Transient source mutation was not restored");
    let result = workflow::verify_export(&output)?;
    assert_eq!(
        result["period"], before.receipt["period"],
        "Captured export period changed"
    );
    let run = fixture.prepare(source, "export-refusal", "refuse")?;
    let output = fixture.root.join("refused.zip");
    let refusal = workflow::export(&run, &fixture.bin, &output);
    assert!(refusal.is_err(), "Refused producer created an export");
    assert!(!output.exists(), "Refused export left an output");
    fixture.captured(&run)?;
    Ok(())
}

fn unlisted_during_render(fixture: &Fixture, source: &Path) -> Result<()> {
    let run = fixture.prepare(source, "unlisted-during-render", "extra")?;
    let before = workflow::verify(&run)?;
    let output = fixture.root.join("unlisted.zip");
    refused(workflow::export(&run, &fixture.bin, &output), "unlisted");
    let added = run.join("unlisted.txt").is_file();
    assert!(added, "Rendering never added the adversarial file");
    assert!(
        !output.exists(),
        "Export published after an unlisted source file appeared"
    );
    let after = workflow::verify(&run)?;
    assert!(after == before, "Extra-file case altered sealed source bytes");
    fixture.captured(&run)?;
    Ok(())
}

pub fn run(root: &Path, runs: &[PathBuf; 2], native_bin: &Path) -> Result<()> {
    let fixture = Fixture::new(root, native_bin)?;
    let current = &runs[1];
    pinned_refusals(&fixture, current)?;
    identity_replacement(&fixture, current)?;
    build_refusals(&fixture, current)?;
    comparison_refusals(&fixture, runs)?;
    changed_whilst_read(&fixture, runs)?;
    snapshot_export(&fixture, current)?;
    unlisted_during_render(&fixture, current)?;
    println!(
        "Native adversarial producer pinning, export snapshot isolation, cleanup and late inventory checks passed."
    );
    Ok(())
}
