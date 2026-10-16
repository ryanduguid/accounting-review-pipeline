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

fn require(condition: bool, message: &str) -> Result<()> {
    if condition { Ok(()) } else { Err(message.into()) }
}

fn digest(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn canonical_hash(value: &Value) -> Result<Value> {
    let response = probe::evaluate(&json!({"op":"json","text":value.to_string()}));
    require(response["ok"] == true, "Fixture manifest canonicalisation failed")?;
    Ok(response["value"].clone())
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
        require(
            output.status.success(),
            &format!(
                "Native delegate compilation: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )?;
        for command in ["close-control", "review-ready"] {
            io(fs::copy(
                native_bin.join(format!("{command}{}", std::env::consts::EXE_SUFFIX)),
                bin.join(format!("actual-{command}{}", std::env::consts::EXE_SUFFIX)),
            ))?;
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
        for name in [
            "calls",
            "displaced",
            "replacement-done",
            "replacement-unsupported",
            "snapshot-checked",
        ] {
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
        reseal(&run, &mut receipt, &digest(&io(fs::read(&self.delegate))?))?;
        workflow::verify(&run)?;
        Ok(run)
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
        require(!packs.is_empty(), "Producer view was never invoked")?;
        let captured = packs[0].parent().ok_or("Captured parent missing")?.to_owned();
        require(
            captured != source && packs.iter().all(|path| path.parent() == Some(captured.as_path())),
            "Display did not use one captured snapshot",
        )?;
        require(
            !captured.exists(),
            "Export snapshot directory survived completion/refusal",
        )?;
        Ok(captured)
    }
}

fn reseal(run: &Path, receipt: &mut Value, hash: &str) -> Result<()> {
    for command in ["close-control", "review-ready"] {
        let manifest = &mut receipt["producer_manifests"][command];
        manifest["launcher_sha256"] = hash.into();
        let manifest_hash = canonical_hash(manifest)?;
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
    let error = workflow::render_html(&run, &fixture.bin, None).unwrap_err();
    require(
        error.contains("hash differs"),
        &format!("Expected pinned hash refusal: {error}"),
    )?;
    require(!fixture.bin.join("calls").exists(), "Hash mismatch executed a producer")?;
    let run = fixture.prepare(source, "wrong-version", "wrong-version")?;
    let error = workflow::render_html(&run, &fixture.bin, None).unwrap_err();
    require(
        error.contains("identity differs"),
        &format!("Expected pinned version refusal: {error}"),
    )?;
    require(
        !fixture.calls()?.lines().any(|line| line.ends_with(" view")),
        "Identity mismatch executed producer view",
    )?;
    let run = fixture.prepare(source, "producer-refusal", "refuse")?;
    let error = workflow::render_html(&run, &fixture.bin, None).unwrap_err();
    require(
        error.contains("verification refused"),
        &format!("Expected producer view refusal: {error}"),
    )?;
    require(
        fixture.calls()?.contains("close-control view"),
        "Refusal did not execute the actual invocation",
    )
}

fn identity_replacement(fixture: &Fixture, source: &Path) -> Result<()> {
    let run = fixture.prepare(source, "identity-replacement", "replace")?;
    let result = workflow::render_html(&run, &fixture.bin, None);
    if fixture.bin.join("replacement-unsupported").exists() {
        let reason = io(fs::read_to_string(fixture.bin.join("replacement-unsupported")))?;
        require(
            cfg!(windows) && reason == "PermissionDenied",
            &format!("Unexpected identity replacement failure: {reason}"),
        )?;
        result?;
        eprintln!(
            "Adversarial identity replacement: Windows denied renaming its running executable; case unavailable."
        );
        return Ok(());
    }
    require(
        fixture.bin.join("replacement-done").exists(),
        "Identity replacement did not occur",
    )?;
    let error = result.expect_err("Changed producer identity must refuse display");
    require(
        error.contains("changed during invocation"),
        &format!("Expected postinvocation identity refusal: {error}"),
    )
}

fn build_identity_replacement(fixture: &Fixture, source: &Path) -> Result<()> {
    fixture.prepare(source, "build-replacement-seed", "replace-invoke")?;
    let run = fixture.root.join("build-replacement");
    let result = workflow::build(&run, &fixture.bin, 9, None);
    if fixture.bin.join("replacement-unsupported").exists() {
        require(
            cfg!(windows) && io(fs::read_to_string(fixture.bin.join("replacement-unsupported")))? == "PermissionDenied",
            "Unexpected build replacement failure",
        )?;
        result?;
        eprintln!(
            "Adversarial build identity replacement: Windows denied renaming its running executable; case unavailable."
        );
    } else {
        require(
            fixture.bin.join("replacement-done").exists(),
            "Build identity replacement did not occur",
        )?;
        let error = result.expect_err("Changed producer identity must refuse build");
        require(
            error.contains("changed during invocation"),
            &format!("Expected build identity refusal: {error}"),
        )?;
    }
    Ok(())
}

fn snapshot_export(fixture: &Fixture, source: &Path) -> Result<()> {
    let run = fixture.prepare(source, "snapshot-isolation", "snapshot")?;
    let before = workflow::verify(&run)?;
    let output = fixture.root.join("snapshot.zip");
    workflow::export(&run, &fixture.bin, &output)?;
    fixture.captured(&run)?;
    require(
        fixture.bin.join("snapshot-checked").exists(),
        "Captured bytes were not checked during rendering",
    )?;
    require(
        workflow::verify(&run)? == before,
        "Transient source mutation was not restored",
    )?;
    let result = workflow::verify_export(&output)?;
    require(
        result["period"] == before.receipt["period"],
        "Captured export period changed",
    )?;
    let run = fixture.prepare(source, "export-refusal", "refuse")?;
    let output = fixture.root.join("refused.zip");
    require(
        workflow::export(&run, &fixture.bin, &output).is_err(),
        "Refused producer created an export",
    )?;
    require(!output.exists(), "Refused export left an output")?;
    fixture.captured(&run)?;
    Ok(())
}

fn unlisted_during_render(fixture: &Fixture, source: &Path) -> Result<()> {
    let run = fixture.prepare(source, "unlisted-during-render", "extra")?;
    let before = workflow::verify(&run)?;
    let output = fixture.root.join("unlisted.zip");
    let error = workflow::export(&run, &fixture.bin, &output).unwrap_err();
    require(
        error.contains("unlisted"),
        &format!("Expected final physical inventory refusal: {error}"),
    )?;
    require(
        run.join("unlisted.txt").is_file(),
        "Rendering never added the adversarial file",
    )?;
    require(
        !output.exists(),
        "Export published after an unlisted source file appeared",
    )?;
    require(
        workflow::verify(&run)? == before,
        "Extra-file case altered sealed source bytes",
    )?;
    fixture.captured(&run)?;
    Ok(())
}

pub fn run(root: &Path, source_run: &Path, native_bin: &Path) -> Result<()> {
    let fixture = Fixture::new(root, native_bin)?;
    pinned_refusals(&fixture, source_run)?;
    identity_replacement(&fixture, source_run)?;
    build_identity_replacement(&fixture, source_run)?;
    snapshot_export(&fixture, source_run)?;
    unlisted_during_render(&fixture, source_run)?;
    println!(
        "Native adversarial producer pinning, export snapshot isolation, cleanup and late inventory checks passed."
    );
    Ok(())
}
