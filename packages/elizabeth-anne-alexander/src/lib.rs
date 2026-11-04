//! Xero Ledger Review Gate: a fixed-policy, synthetic-data review boundary.
//!
//! The modules prefixed `py` reproduce the CPython behaviour the released
//! Python package depended on, so outputs, digests and refusals stay identical.

pub mod argtheme;
pub mod cli;
pub mod decimal;
pub mod gateway;
pub mod isodate;
pub mod persist;
pub mod pycsv;
pub mod pyjson;
pub mod pypath;
pub mod pytext;
mod tables;
pub mod termsize;
pub mod textwrap;
pub mod util;

#[cfg(test)]
mod tests;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
