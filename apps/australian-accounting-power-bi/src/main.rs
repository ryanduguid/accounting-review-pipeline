// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! `generate-fixtures`: rewrites the six fabricated sample CSVs beside this crate.
//!
//! Like `tools/generate_fixtures.py` before it, it takes no options and ignores any
//! arguments it is given.

use std::path::PathBuf;
use std::process::ExitCode;

use australian_accounting_power_bi::fixtures;

fn main() -> ExitCode {
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples");
    let code = fixtures::command_line(&samples, &mut std::io::stdout().lock(), &mut std::io::stderr().lock());
    ExitCode::from(code)
}
