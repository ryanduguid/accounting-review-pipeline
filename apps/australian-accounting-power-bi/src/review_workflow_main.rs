// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

fn main() -> std::process::ExitCode {
    // argv is the user's own command line; it grants nothing.
    // nosemgrep: rust.lang.security.args-os.args-os
    let arguments = std::env::args_os().skip(1).collect();
    let code = australian_accounting_power_bi::workflow::command_line(
        arguments,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::ExitCode::from(code)
}
