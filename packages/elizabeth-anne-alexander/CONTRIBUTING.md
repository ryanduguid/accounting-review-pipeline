# Contributing

This component demonstrates a fixed-policy boundary for AI-assisted trial-balance review. Treat the refusals below as the part worth protecting; they are what the design exists to show.

## What contributions must not add

- OAuth, network calls, or any live Xero connection
- model or LLM invocation
- any tool that mutates accounting data

If your change widens the model-facing payload, or lets reviewer-only evidence reach a model-facing structure, argue in the pull request why the narrower version does not work.

## Data boundary

- Use synthetic data. This component's `.gitignore` blocks `.csv` outside
  `elizabeth_anne_alexander/samples/inputs/`, plus `.env`, `.env.*`, `*.token` and `*.pem`.
  The monorepo's only trial-balance conformance allowlist is
  `contracts/xero-trial-balance-v1/`, whose fabricated files are pinned by `SHA256SUMS`.
- Keep anything drawn from a real Xero organisation out of the repository, including account names, tenant identifiers and balances.

## Local verification

Rust, at the version `rust-toolchain.toml` names, with the committed `Cargo.lock`.

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
```

CI runs these on Linux and Windows, builds the release binary and runs the smoke script from an empty directory.

The differential test compares the binary with the released 0.2.6 Python package. It needs Python 3.14 or newer:

```bash
mkdir -p /tmp/py-base
git archive elizabeth-anne-alexander/v0.2.6 packages/elizabeth-anne-alexander/elizabeth_anne_alexander | tar -x -C /tmp/py-base --strip-components=2
cargo build --features differential --examples
python tests/differential/differential.py --python-pkg /tmp/py-base --binary target/debug/elizabeth-anne-alexander --cases 1000
python tests/differential/decimal_differential.py target/debug/examples/decimal_oracle
```

`tools/gen_unicode_tables.py` regenerates `src/tables.rs` from the running Python's Unicode database.

## Pull requests

State which policy or gate your change affects and show the test that holds it. Delete the gate in your working copy and confirm the test fails. A test that passes either way holds nothing.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md) rather than opening an issue.
