# Accounting Review Pipeline agent instructions

This repository is the local assembly of the Accounting Review Pipeline: 8 independently
versioned components joined only by local files and commands. Its canonical GitHub repository
is `ryanduguid/accounting-review-pipeline`.
Follow the closest component `AGENTS.md`, `CONTRIBUTING.md` or `README.md` for component work.
These repository-wide rules apply everywhere:

- Only `packages/xero-trial-balance-export/` may use OAuth, call Xero, use an HTTP client,
  hold tokens or read Xero credentials (the environment names documented in the exporter's
  `.env.example`). Review packages, the Excel adapter and the Power BI application stay
  offline, read local files and ship fabricated data only. Native review commands
  preserve the original exact decimal money rules. Describe the Excel
  adapter and Power BI application's native numeric types separately; VBA
  `Double` is floating point and is outside the Decimal guarantee.
- Roles stay separate. The readiness gate emits `READY`, `NOT_READY` or `BLOCKED` and
  decides whether a pack reaches review. Monthly close emits `PASS`, `REVIEW` or `BLOCKED`
  (exit 0 only for `PASS`, exit 2 for `REVIEW` or `BLOCKED`, exit 1 for malformed input) and
  surfaces exceptions. Ledger review produces `REVIEW_READY` evidence and a
  `DECISION_RECORDED` receipt. None of them approves accounting, posts a journal, makes a
  payment, lodges a return or locks a period.
- Preserve every distribution name, import package, command, exit code, file schema,
  version, licence and component lockfile. Production code must not import a sibling
  component, and no review package may import the exporter. One shared `.venv` makes every
  Python component importable from every other; that is a convenience for tests, not a
  licence to depend on a sibling. The owner made one exception on 11 October 2026: the
  Xero Ledger Review Gate became a Rust binary in 1.0.0 and has no Python import package;
  its command, exit codes and file formats are unchanged. It has no release caller until a
  Rust release path is approved; 0.2.6 stays the released Python distribution on PyPI.
  The owner authorised the same binary-only major-version exception for evatt
  1.0.0 on 13 October 2026. Its Python import and map-writing facility are removed;
  its CLI contracts remain, with the documented version and failure-path changes.
  It has no release caller until a Rust release path is approved; 0.2.0 remains
  the prior Python release, which was never published to PyPI.
  The same approved binary-only exception applies to the Xero exporter in
  1.0.0: its Rust commands retain the CSV and token-cache contracts, and the
  Python import API is removed. It has no native release caller yet.
  The same approved binary-only exception applies to Workpaper Review Gate
  1.0.0. Its Python runtime and import API are removed; its native commands
  retain the readiness and file contracts, with documented parser and diagnostic
  limits. Its release workflow explicitly blocks publication until a native
  publisher is approved.
  The same approved binary-only exception applies to Monthly Close Controls
  1.0.0. Its Python runtime and import API are removed. The native close-control
  command preserves its file and status contracts, with documented compatibility
  limits. Native publication is explicitly blocked.
  The approved binary-only exception also applies to the Power BI review workflow
  in 1.0.0. Its six commands use the native review-workflow binary and an embedded
  fabricated case; the previous Python workflow module is removed.
- The root `pyproject.toml`, `uv.lock` and `justfile` are a development entrypoint only.
  The root is a virtual uv workspace: it declares no package, no version and no runtime
  dependency, and publishes nothing. Do not add a root distribution, shared runtime library,
  unified version, generated dependency graph or monorepo framework. Movement and behaviour
  changes are separate pull requests.
- The root `uv.lock` covers retained Python development tooling. Native components
  use their own `Cargo.lock`. Update the owning lock after dependency changes.
- Only workflows under the root `.github/workflows/` are active. Components carry no
  `.github/` directory; the subtree merge commits recorded in IMPORTS.md hold the imported ones.
- Release only through the root callers `.github/workflows/release-<component>.yml` on a
  namespaced annotated tag `<component>/vMAJOR.MINOR.PATCH`, never through a nested
  `release.yml`. One tag publishes exactly one component.
- Never commit client data, workpapers, credentials, tokens, generated review packs,
  entity maps or native application evidence containing client data.

## Path decision

Component directories use each component's normalised distribution name because the
reviewed Release Policy identity gate requires a nested release's directory leaf,
`tag-prefix` and distribution name to be identical. The migration plan's
`packages/monthly-close-controls`, `packages/workpaper-review-gate` and
`packages/xero-ledger-review-gate` therefore became `packages/monthly-close-control-plane`,
`packages/review-ready-gate` and `packages/elizabeth-anne-alexander`. The exporter, the Excel
adapter and the Power BI application keep the plan's paths. The grant workpapers application,
imported later, sits at `apps/grant-acquittal-workpapers/` because it is never released. `IMPORTS.md` records the decision.

## Setup

From a fresh clone, `uv sync --locked --group dev` at the root installs the shared
Python test toolchain into one `.venv`. Install Rust for native components.
`just test` runs every component's suite plus joined conformance. `just`
comes from `uv tool install rust-just`; its recipes are `setup`, `lint`, `typecheck`,
`test` and `check` (the last 3 together). Each recipe loops over the per-component
commands in the table below, which remain the authority. `just check` is not a CI
equivalent: it does not verify component lockfiles, run the exporter's pinned Python
comparison or native binary smoke checks, run actionlint, the Power BI CLI
or CodeQL, or check the contract digests.

## Command routing

Run every check from the owning component directory with its documented commands:

| Component | Directory | Checks |
|---|---|---|
| Xero Trial Balance Export | `packages/xero-trial-balance-export/` | `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-features`, and the pinned Python 3.15 differential, on Linux and Windows |
| Workpaper Review Gate | `packages/review-ready-gate/` | `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-features`, and its frozen Python 3.15 differential, on Linux and Windows |
| Monthly Close Controls | `packages/monthly-close-control-plane/` | `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-features`, its frozen Python 3.15 differential and native binary smoke on Linux and Windows; RustSec dependency audit |
| Xero Ledger Review Gate | `packages/elizabeth-anne-alexander/` | `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings` and `cargo test --locked`, on Linux and Windows; `tests/differential/differential.py` compares the binary with the 0.2.6 Python release |
| Accounting Excel Toolkit | `adapters/accounting-excel-toolkit/` | `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-features`, `cargo run --locked --release --bin check-vba-encoding`, its frozen Python 3.15 differential on Linux and Windows, and `python -B -m unittest discover -s tests -v`; optional `tools/native_excel_acceptance.ps1` on Windows with Excel |
| Australian Accounting Power BI | `apps/australian-accounting-power-bi/` | `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-features`, and its frozen Python 3.15 workflow differential on Linux and Windows; `npm ci --ignore-scripts` then `npx --no-install powerbi-report-author validate australian-accounting-power-bi.Report` |
| Grant Acquittal Workpapers | `apps/grant-acquittal-workpapers/` | `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked` for the Rust workpaper; `python -B -m unittest discover -s tests -v` for the setup and replay utility; both on Linux and Windows; Ruff from its `pyproject.toml` and mypy from its `mypy.ini` |
| evatt | `packages/evatt/` | `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings` and `cargo test --locked --all-features`; the differential probe is enabled only with `--features probe` |

`ci.yml`'s `rust` job runs native package checks on Linux and Windows. Its
`dependency-audit` job checks Monthly Close Controls against RustSec. The Excel
adapter and grant application retain Python tooling checked with their own
Ruff, mypy and unittest settings. The application Rust crates run in the
standard-library workflow. Power BI uses native commands for its generator,
review workflow and both review producers; Python is retained for frozen oracles.

A change to the shared Xero trial-balance contract directory (`contracts/xero-trial-balance-v1/`) must run the exporter, all 3 review packages,
the Excel adapter, Power BI structural validation and the joined conformance test. A change
to the root `pyproject.toml`, `uv.lock` or `justfile` triggers every component workflow,
because the root lock is what `uv run --locked` validates inside every component directory.
