//! evatt's standalone implementation. CPython 3.14 and Unicode 16 are frozen.
pub mod pytext;
#[rustfmt::skip]
pub mod tables;
pub mod patterns;
pub mod pyjson;
#[rustfmt::skip]
mod regexes;

#[cfg(feature = "probe")]
pub fn probe_patterns() -> impl Iterator<Item = (&'static str, &'static fancy_regex::Regex)> {
    regexes::PATTERNS
        .iter()
        .map(|&(name, _)| (name, patterns::pattern(name)))
}
mod argtheme;
#[cfg(feature = "probe")]
pub mod benchmark;
pub mod cli;
mod commands;
pub mod disclosure;
pub mod email;
pub mod entities;
pub mod files;
pub mod oserror;
mod persist;
pub mod pypath;
pub mod redact;
pub mod stdio;
mod termsize;
mod textwrap;
pub mod verify;
