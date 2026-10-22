//! Offline readiness controls and verified, transactional pack files.
pub mod cli;
pub mod comparison;
#[cfg(test)]
mod contract_tests;
mod decimal;
pub mod documents;
pub mod engine;
#[cfg(test)]
mod evaluation_tests;
mod isodate;
pub mod loader;
pub mod models;
#[cfg(feature = "probe")]
pub mod probe;
pub mod profiles;
pub mod publication;
mod pycsv;
mod pyjson;
mod pypath;
mod pytext;
pub mod report;
pub mod stdio;
mod tables;
#[cfg(test)]
mod test_support;
pub mod viewer;

pub type Result<T> = std::result::Result<T, String>;

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
