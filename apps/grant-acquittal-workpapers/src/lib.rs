//! Grant acquittal workpapers: local evidence schedules for reviewed grant
//! allocations. The modules prefixed `py` reproduce the CPython behaviour the
//! Python script relied on, so outputs and messages stay identical.

pub mod amount;
pub mod cli;
pub mod isodate;
pub mod oserror;
pub mod pycsv;
pub mod pyjson;
pub mod pytext;
mod tables;
pub mod workpaper;

#[cfg(test)]
mod tests;
