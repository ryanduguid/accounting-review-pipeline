// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use std::path::Path;

pub fn create(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_dir(target, link);
    match result {
        Ok(()) => true,
        Err(error) if cfg!(windows) && error.raw_os_error() == Some(1314) => {
            eprintln!("directory-link check skipped: Windows symlink privilege is unavailable");
            false
        }
        Err(error) => panic!("directory-link fixture: {error}"),
    }
}
