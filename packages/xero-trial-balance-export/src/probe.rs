// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

fn main() {
    std::thread::Builder::new()
        .name("xero-probe".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(xero_trial_balance_export::compat_probe::run)
        .expect("probe worker")
        .join()
        .expect("probe completion");
}
