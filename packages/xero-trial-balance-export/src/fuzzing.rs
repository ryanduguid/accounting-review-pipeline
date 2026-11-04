// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The response path the cargo-fuzz `report_json` target drives.

use crate::{amount, export, pyjson, report};

const DATE: &str = "2026-06-30";
const TENANT: &str = r#"{"tenantId": "tenant-guid", "tenantName": "Demo Company AU"}"#;

/// Read untrusted bytes as a Trial Balance response body, as `export-tb` does
/// after the HTTP client decodes it. Refusals are results; only a panic is a
/// defect.
pub fn exercise(bytes: &[u8]) {
    let body = String::from_utf8_lossy(bytes);
    let Ok(payload) = pyjson::loads_response(&body) else {
        return;
    };
    let Ok(rows) = export::validated_rows(&payload, DATE) else {
        return;
    };
    let tenant = pyjson::loads(TENANT).expect("the fixed tenant is valid JSON");
    if let Ok((output, totals)) = report::build_rows(&rows, &tenant, DATE) {
        let _ = amount::check_balanced(&totals, Some(output.len()));
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
