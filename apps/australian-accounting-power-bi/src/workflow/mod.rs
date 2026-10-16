//! Pinned, offline fabricated review workflow and portable evidence.

mod archive;
mod cli;
mod export;
mod files;
mod html;
mod json;
mod ledger;
#[cfg(feature = "probe")]
#[doc(hidden)]
pub mod probe;
mod producer;
mod projection;
mod pycsv;
mod run;

pub use cli::entry as command_line;
pub use export::{export, verify as verify_export};
pub use html::render as render_html;
pub use projection::projection;
pub use run::{Verified, build, verify};
mod schema;

type Result<T> = std::result::Result<T, String>;

const MAX_EXPORT_BYTES: usize = 2 * 1024 * 1024;
const MAX_EXPORT_MEMBER: usize = 512 * 1024;
const MAX_EXPORT_FILES: usize = 64;
const BOUNDARY: &str =
    "Fabricated demonstration. Verification and acknowledgement do not approve accounting or close a period.";
const CLOSE_FILES: [&str; 4] = [
    "close-review-pack.json",
    "close-summary.md",
    "exceptions.csv",
    "client-queries.csv",
];
const READY_FILES: [&str; 3] = ["readiness-pack.json", "readiness-summary.md", "findings.csv"];
const PRODUCERS: [(&str, &str, &str, &str); 2] = [
    (
        "close-control",
        "monthly-close-control-plane",
        "closecontrol",
        "closecontrol.cli:main",
    ),
    (
        "review-ready",
        "review-ready-gate",
        "reviewready",
        "reviewready.cli:main",
    ),
];

fn digest(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}
