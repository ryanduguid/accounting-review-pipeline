// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: FSL-1.1-ALv2

fn main() {
    std::process::exit(review_ready_gate::cli::command_thread(
        review_ready_gate::publication::process_probe,
    ));
}
