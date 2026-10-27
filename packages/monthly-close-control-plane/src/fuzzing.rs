//! The loader calls both fuzz harnesses make: the `close-control-fuzz`
//! mutation tool and the cargo-fuzz `close_loaders` target.

use crate::loader::{self, SourceSnapshot};
use std::path::Path;

/// Read untrusted bytes with every loader. Refusals are results; only a panic
/// is a defect.
pub fn exercise(bytes: &[u8]) {
    let source = SourceSnapshot::new(Path::new("fuzz.csv"), bytes.to_vec());
    let _ = loader::load_canonical_tb(&source);
    let _ = loader::load_mapping(Some(&source));
    let _ = loader::load_mapping_policy(&source);
    let _ = loader::load_balance_policy(&source);
    let _ = loader::load_subledger(Some(&source));
}
