// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

fn main() {
    let args = std::env::args_os().skip(1).collect();
    std::process::exit(review_ready_gate::cli::command_thread(move || {
        review_ready_gate::cli::main(args)
    }));
}
