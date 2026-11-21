// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

fn main() {
    std::thread::Builder::new()
        .name("xero-probe".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(xero_trial_balance_export::compat_probe::run)
        .expect("probe worker")
        .join()
        .expect("probe completion");
}
