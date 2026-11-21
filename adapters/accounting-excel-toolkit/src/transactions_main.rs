// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::{
    io::{self, Write},
    process::ExitCode,
};

fn main() -> ExitCode {
    let (code, out, err) = accounting_excel_toolkit::transactions::cli::run(
        &std::env::args_os().skip(1).collect::<Vec<_>>(),
    );
    if io::stdout().lock().write_all(out.as_bytes()).is_err()
        || io::stderr().lock().write_all(err.as_bytes()).is_err()
    {
        return ExitCode::FAILURE;
    }
    ExitCode::from(code)
}
