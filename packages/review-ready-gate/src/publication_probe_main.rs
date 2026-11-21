// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

fn main() {
    std::process::exit(review_ready_gate::cli::command_thread(
        review_ready_gate::publication::process_probe,
    ));
}
