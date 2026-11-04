// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

#[cfg(test)]
use std::path::PathBuf;

#[path = "../tests/support/temp.rs"]
mod temp;
pub use temp::TempDir;
#[cfg(test)]
pub fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}
