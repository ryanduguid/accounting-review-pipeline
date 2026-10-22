//! CPython 3.15 comparison against the frozen pre-port workflow.

#[path = "workflow_differential/adversarial.rs"]
mod adversarial;
#[path = "workflow_differential/cases.rs"]
mod cases;
#[path = "workflow_differential/rehearsal.rs"]
mod rehearsal;

use australian_accounting_power_bi::workflow::probe;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

type Result<T> = std::result::Result<T, String>;

struct Settings {
    python: PathBuf,
    oracle: PathBuf,
    bin: PathBuf,
    seed: u64,
}

fn settings() -> Result<Settings> {
    let mut values = std::collections::BTreeMap::new();
    // Oracle paths come from the developer running the comparison; they grant nothing.
    // nosemgrep: rust.lang.security.args-os.args-os
    let mut args = std::env::args_os().skip(1);
    while let Some(name) = args.next() {
        if !["--python", "--oracle", "--bin-dir", "--seed"]
            .iter()
            .any(|key| name == *key)
        {
            return Err("Use --python PATH --oracle APP --bin-dir PATH [--seed INTEGER].".into());
        }
        values.insert(name, args.next().ok_or("Missing argument value.")?);
    }
    let path = |key| {
        values
            .get(std::ffi::OsStr::new(key))
            .map(PathBuf::from)
            .ok_or_else(|| format!("Missing {key}."))
    };
    let seed = values
        .get(std::ffi::OsStr::new("--seed"))
        .map(|value| value.to_string_lossy().parse::<u64>())
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or(16);
    Ok(Settings {
        python: path("--python")?,
        oracle: path("--oracle")?,
        bin: path("--bin-dir")?,
        seed,
    })
}

struct Oracle {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Oracle {
    fn new(settings: &Settings) -> Result<Self> {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/differential/workflow_oracle.py");
        let mut child = Command::new(&settings.python)
            .args(["-I", "-B"])
            .arg(script)
            .arg(&settings.oracle)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| error.to_string())?;
        let input = child.stdin.take().ok_or("Oracle stdin unavailable.")?;
        let output = BufReader::new(child.stdout.take().ok_or("Oracle stdout unavailable.")?);
        Ok(Self { child, input, output })
    }

    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        serde_json::to_writer(&mut self.input, request).map_err(|error| error.to_string())?;
        self.input
            .write_all(b"\n")
            .and_then(|()| self.input.flush())
            .map_err(|error| error.to_string())?;
        let mut line = String::new();
        if self.output.read_line(&mut line).map_err(|error| error.to_string())? == 0 {
            return Err("Oracle exited before its response.".into());
        }
        serde_json::from_str(&line).map_err(|error| format!("Invalid oracle response: {error}"))
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run() -> Result<()> {
    let settings = settings()?;
    let workspace = rehearsal::Temporary::new()?;
    let runs = rehearsal::run(&settings, &workspace.0)?;
    let cases = cases::generate(&runs[1], settings.seed)?;
    let mut oracle = Oracle::new(&settings)?;
    let (mut counts, mut failures) = (std::collections::BTreeMap::<String, usize>::new(), 0);
    for (index, request) in cases.iter().enumerate() {
        let expected = oracle.evaluate(request)?;
        let actual = probe::evaluate(request);
        *counts.entry(request["op"].as_str().unwrap().into()).or_default() += 1;
        if actual != expected {
            failures += 1;
            if failures <= 8 {
                eprintln!("Case {index}: {request}\nPython {expected}\nRust {actual}");
            }
        }
    }
    println!(
        "Seed {}: {} core cases {counts:?}; {failures} unexplained differences. Two-month six-command CLI, sealed files, CSV/HTML/ZIP bytes and comparison checked.",
        settings.seed,
        cases.len()
    );
    if failures != 0 {
        return Err("Differential comparison failed.".into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
