# Contributing

Keep this project in its narrow role: a local, deterministic readiness gate.
No contribution should give it authority to post journals, make payments, lodge
returns, lock periods, send reports, or approve a file.

## Data boundary

- Use fabricated fixtures. Keep client trial balances, subledgers, workpapers,
  readiness packs, credentials, `.env` files, tokens and screenshots from a live
  accounting system out of the repository.
- Put fabricated CSV fixtures under `examples/` and schema references
  (header-only CSVs, plus `self_review.json`) under `schemas/`. The `.gitignore`
  blocks ordinary CSV files outside those directories.
- Treat source CSV content and review notes as untrusted input. Keep the
  fail-closed validation and the spreadsheet-formula safeguards in place.

## Local verification

Use Rust stable and the committed Cargo lockfile.

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --release
```

CI runs the native checks on Linux and Windows. CodeQL does not cover this Rust
source. Python 3.15 with Unicode 17.0.0 is required only for the frozen baseline
comparison, which uses fabricated inputs.

For a behaviour change, add or update a focused test under `tests/`. Keep the
output deterministic: no wall-clock timestamps, client identifiers or hidden
state in a readiness pack.

[Native port coverage](tests/PORT_COVERAGE.md) maps the retired Python test
families to native tests and records the limit of that mapping.

Run the root joined conformance tests from a full `accounting-review-pipeline`
checkout. They read `contracts/xero-trial-balance-v1/` and compare Monthly Close
Controls with the Rust probe. The native crate's tests use its own fabricated
examples and evaluation fixtures.

## Pull requests

Explain which gate or boundary your change affects, include the test result, and
name any operational limitation that remains. Never present a review
acknowledgement as an approved file.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md), and
keep credentials, client data and exploit details out of the issue tracker.

## Releasing

Native publishing is blocked. Follow [RELEASING.md](RELEASING.md); do not tag or
upload a native release until a separately authorised publisher is configured.
