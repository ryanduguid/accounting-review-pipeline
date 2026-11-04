// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Joined conformance for the independently released review commands.
//! Monthly Close Controls and the Workpaper Review Gate must agree on the
//! blocks they share, and both must keep the root Xero trial-balance contract's
//! accept/reject boundary. Each answers through its own conformance probe (the
//! `probe` feature), in process; the exporter's side of the contract is its
//! own `tests/root_contract.rs`.
#![cfg(test)]

mod loader_dates;
mod shared_blocks;
mod xero_contract;

use monthly_close_control_plane::probe as close_probe;
use monthly_close_control_plane::pyjson::{self, Json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Monthly Close Controls' probe answer, as its probe command prints it.
fn monthly(request: &Json) -> Json {
    pyjson::loads(&close_probe::encode(&close_probe::evaluate(request))).unwrap()
}

/// The Workpaper Review Gate's probe answer, as its probe command prints it.
fn ready(request: &Json) -> Json {
    let answer = review_ready_gate::probe::evaluate(&pyjson::dumps(request, false)).unwrap();
    pyjson::loads(&answer).unwrap()
}

fn is_error(answer: &Json) -> bool {
    answer.get("error").is_some()
}

fn text(value: &Json) -> String {
    value.as_pystr().and_then(|s| s.to_utf8()).unwrap()
}

fn path_text(path: &Path) -> Json {
    Json::str(&path.to_string_lossy())
}

/// A request naming a file and carrying its bytes, as the probes read them.
fn file_request(op: &str, extra: Option<(&str, &str)>, path: &Path) -> Json {
    let hex: String = std::fs::read(path)
        .unwrap()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut request = Json::object([
        ("op", Json::str(op)),
        ("path", path_text(path)),
        ("hex", Json::str(&hex)),
    ]);
    if let Some((key, value)) = extra {
        request.set(key, Json::str(value));
    }
    request
}

fn trial_balance(path: &Path) -> Json {
    file_request("csv", Some(("kind", "trial_balance")), path)
}

/// A scratch folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "joined-conformance-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).unwrap();
        Scratch(path)
    }

    /// The folder as `Path.resolve()` gives it: Windows' verbatim prefix dropped.
    fn resolved(&self) -> PathBuf {
        let real = std::fs::canonicalize(&self.0).unwrap();
        let text = real.to_string_lossy();
        PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
