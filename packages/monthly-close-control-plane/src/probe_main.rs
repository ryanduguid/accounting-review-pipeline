// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

fn main() -> std::process::ExitCode {
    let result = std::thread::Builder::new()
        .name("close-control-probe".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(monthly_close_control_plane::probe::main)
        .expect("probe thread")
        .join()
        .unwrap_or(1);
    if result == 0 {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
