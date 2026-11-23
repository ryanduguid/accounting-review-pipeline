// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use accounting_excel_toolkit::aged::cli::{Tool, exit};
fn main() -> std::process::ExitCode {
    exit(Tool::Verifier)
}
