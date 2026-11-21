// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

fn main() {
    let args = std::env::args_os().skip(1).collect();
    std::process::exit(review_ready_gate::cli::command_thread(move || {
        review_ready_gate::cli::main(args)
    }));
}
