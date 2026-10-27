//! Offline monthly close controls. An acknowledgement never changes a status.

pub mod calculation_evidence;
pub mod classification;
pub mod cli;
pub mod comparison;
pub mod decimal;
pub mod drivers;
pub mod engine;
pub mod equity;
#[cfg(feature = "tools")]
#[doc(hidden)]
pub mod fuzzing;
pub mod isodate;
pub mod loader;
pub mod models;
#[cfg(feature = "probe")]
pub mod probe;
pub mod publication;
pub mod pycsv;
pub mod pyjson;
pub mod pypath;
pub mod pytext;
pub mod queries;
pub mod queue;
pub mod reconciliation;
pub mod reconciliation_report;
pub mod report;
pub mod schedules;
#[allow(unused_macros)]
pub mod stdio;
mod tables;
pub mod viewer;

pub type Result<T> = std::result::Result<T, String>;

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(any(test, feature = "probe"))]
mod test_support;
