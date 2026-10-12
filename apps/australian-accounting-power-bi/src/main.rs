//! `generate-fixtures`: rewrites the six fabricated sample CSVs beside this crate.
//!
//! Like `tools/generate_fixtures.py` before it, it takes no options and ignores any
//! arguments it is given.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use australian_accounting_power_bi::fixtures::{self, Error};

/// Python's text-mode stdout ends lines with CRLF on Windows.
const NEWLINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

fn main() -> ExitCode {
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples");
    match fixtures::generate(&samples) {
        Ok(()) => {
            let line = format!(
                "Successfully generated all synthetic fixtures in {}{NEWLINE}",
                samples.display()
            );
            match std::io::stdout().write_all(line.as_bytes()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            }
        }
        Err(error) => {
            let message = match error {
                Error::Calculation(message) => format!("ValueError: {message}"),
                Error::Io(error) => format!("OSError: {error}"),
            };
            eprint!("{message}{NEWLINE}");
            ExitCode::FAILURE
        }
    }
}
