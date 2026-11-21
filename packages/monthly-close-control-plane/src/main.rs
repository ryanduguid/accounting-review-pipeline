// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

fn main() -> std::process::ExitCode {
    let args = std::env::args_os().skip(1).collect();
    let status = monthly_close_control_plane::cli::command_thread(move || {
        monthly_close_control_plane::cli::main(args)
    });
    std::process::ExitCode::from(u8::try_from(status).unwrap_or(1))
}
