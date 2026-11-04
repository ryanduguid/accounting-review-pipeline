// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

fn main() {
    let result = std::thread::Builder::new()
        .name("close-control-probe".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(monthly_close_control_plane::probe::main)
        .expect("probe thread")
        .join()
        .unwrap_or(1);
    std::process::exit(result);
}
