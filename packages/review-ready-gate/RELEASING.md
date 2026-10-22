# Releasing

Native release publishing is not configured. Do not create or push a
`review-ready-gate/v1.0.0` tag or upload release assets.
The root `release-review-ready-gate.yml` fails explicitly and has read permission
only. It cannot create a GitHub Release or publish to PyPI.

Version 1.0.0 replaces the Python distribution with the `review-ready` Rust
binary. Build and test a local candidate with:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --release
```

A future native publisher needs reviewed platform builds, checksums, provenance,
dependency evidence and immutable release checks before publication is enabled.
Changing the workflow or configuring that publisher requires a separate release
task and explicit authorisation.

The Python tags, PyPI artefacts and release history remain historical records.
Do not move or reuse them. Standalone releases through `v0.1.1` came from the
retired `workpaper-review-gate` repository; namespaced Python releases came from
Accounting Review Pipeline. The protected `v0.1.0` tag failed before publication
and must not be reused.

A `READY` pack is a review aid. It does not approve a client file or authorise a
software release.
