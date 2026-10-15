# Contributing

Keep this project in its narrow role: a local, deterministic review-pack generator. No contribution should give it authority to post journals, make payments, lodge returns, lock periods, send reports, or approve a close.

## Data boundary

- Use fabricated fixtures. Keep client trial balances, subledgers, workpapers, review packs, credentials, `.env` files, tokens and screenshots from a live accounting system out of the repository.
- Put fabricated CSV fixtures under `examples/` and header-only schema references under `schemas/`. The `.gitignore` blocks ordinary CSV files outside those 2 directories.
- Treat source CSV content and review notes as untrusted input. Keep the fail-closed validation and the spreadsheet-formula safeguards in place.

## Local verification

Use the stable Rust toolchain pinned in `rust-toolchain.toml`, from this directory:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --release --bin close-control
```

For a behaviour change, add or update a focused Rust test. Keep the output deterministic: no wall-clock timestamps, client identifiers or hidden state in a review pack.

The `probe` feature enables the migration differential harness; Python 3.15 is
needed only to run its frozen 0.2.0 oracle. Follow `AGENTS.md` for that comparison,
the all-command binary smoke and the RustSec dependency check. Production needs
no Python interpreter. The retained cross-repository utility helpers and their
tests use the root development environment. Run joined conformance and workflow
checks from a full `accounting-review-pipeline` checkout.

## Pull requests

Explain which control or boundary your change affects, include the test result, and name any operational limitation that remains. Never present a review acknowledgement as an approved or completed close.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md), and keep credentials, client data and exploit details out of the issue tracker.
