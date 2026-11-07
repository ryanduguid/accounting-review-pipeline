// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! The loader calls the cargo-fuzz `pack_loaders` target makes.

use crate::loader::{self, SourceSnapshot};
use std::path::Path;

/// Read untrusted bytes with every pack loader. Refusals are results; only a
/// panic is a defect.
pub fn exercise(bytes: &[u8]) {
    let source = SourceSnapshot::new(Path::new("fuzz.csv"), bytes.to_vec());
    let _ = loader::load_canonical_tb(&source);
    let _ = loader::load_open_items(&source);
    let _ = loader::load_activity_statement(&source);
    let _ = loader::load_gst_control(&source);
    let _ = loader::load_bank_rec(&source);
    let _ = loader::load_tie_out_matrix(&source);
    let _ = loader::load_fs_mapping(&source);
    let _ = loader::load_prior_findings(&source);
    for profile in ["bas", "month_end", "year_end"] {
        let _ = loader::load_self_review(&source, profile);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    /// Replay the checked-in seeds so a broken harness fails ordinary tests.
    #[test]
    fn seeds_replay_without_panicking() {
        let seeds = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds");
        let mut count = 0;
        for entry in fs::read_dir(seeds).unwrap() {
            super::exercise(&fs::read(entry.unwrap().path()).unwrap());
            count += 1;
        }
        assert!(count > 0);
    }
}
