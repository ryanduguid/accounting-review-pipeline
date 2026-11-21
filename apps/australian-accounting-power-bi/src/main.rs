// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

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
