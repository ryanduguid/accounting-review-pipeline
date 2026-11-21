// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

#[cfg(test)]
use std::path::PathBuf;

#[path = "../tests/support/temp.rs"]
mod temp;
pub use temp::TempDir;
#[cfg(test)]
pub fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}
