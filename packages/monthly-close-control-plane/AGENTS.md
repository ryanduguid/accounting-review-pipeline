# Agent instructions

This component is a local, deterministic review-pack generator. Read
[root AGENTS.md](../../AGENTS.md) for workspace, component and release routing. Preserve these
accounting and human-review boundaries:

- Preserve exactly the `PASS`, `REVIEW`, and `BLOCKED` pack states. For `review` and
  `workbench`, exit `0` only for `PASS`, exit `2` for `REVIEW` or `BLOCKED`, and exit
  `1` for malformed input, invalid command configuration or an unwritable output.
  The read-only `view` command exits `0` after verified display and `1` on verification
  failure.
- An acknowledgement records a human action only. It never changes a control status,
  approves or signs off a close, or proves that a period was closed.
- Keep client source files, workpapers, review notes and generated packs in a separate,
  access-controlled directory outside the checkout. Repository fixtures must remain
  fabricated.
- Parse and calculate money, balances, thresholds and tolerances with exact `Decimal`
  arithmetic, never binary floating point. Preserve fail-closed schema and integrity gates.
- Do not add network or live Xero access, credential or token handling, journal, payment
  or report mutation, approval or sign-off authority, period locking, or tax lodgement.
- `src/calculation_evidence.rs` reads evidence files from disk and must stay that way. It has
  no HTTP client and no calculator, and it never makes the call the evidence describes.
  Treat an evidence file as untrusted input. A required calculation with no evidence, a
  tampered file, a recorded refusal and a period mismatch each map onto an existing state
  and can never become `PASS`; do not add a status, and do not let an acknowledgement
  clear one.
- For release work, read [RELEASING.md](RELEASING.md) together with the root
  instructions. The active caller is
  [release-monthly-close-control-plane.yml](../../.github/workflows/release-monthly-close-control-plane.yml),
  using the component tag prefix required at the root. Never build or upload release
  assets by hand, and do not tag or publish without explicit action-time approval.

## Repository map

- [README.md](README.md) owns the review-pack, status and exit-code contracts.
- [CONTRIBUTING.md](CONTRIBUTING.md) owns fixture, data-handling and pull-request rules.
- [RELEASING.md](RELEASING.md) owns release preflight, tagging and verification.

## Native checks

Version 1.0.0 is an authorised standalone Rust binary. The Python import package
is removed. Preserve command names, flags, exit states and saved file contracts.
Run these checks from this component:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --release
cargo audit --deny warnings
```

The root ci.yml runs these gates on Linux and Windows, with the dependency audit
on Linux. The feature-gated close-control-differential compares fabricated values,
file bytes and command execution with frozen CPython 3.15 source from the port's
base revision. Its Python process is a migration oracle, never a production
runtime. The root `.github/ci/monthly-close-control-plane/smoke.sh` exercises the
release binary from an empty directory using fabricated examples.

Native release publishing is not configured. The root release caller fails with
read permission only; a publisher requires separate approval.