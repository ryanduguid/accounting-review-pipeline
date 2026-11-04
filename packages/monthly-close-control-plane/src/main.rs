// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

fn main() {
    let args = std::env::args_os().skip(1).collect();
    std::process::exit(monthly_close_control_plane::cli::command_thread(
        move || monthly_close_control_plane::cli::main(args),
    ));
}
