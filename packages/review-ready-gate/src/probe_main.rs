// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

fn main() {
    std::process::exit(review_ready_gate::cli::command_thread(
        review_ready_gate::probe::main,
    ));
}
