# Porting status: `grant-acquittal-workpapers`

## Ported to Rust

- `src/main.rs` (`[[bin]]` target `grant-workpaper`): native Rust workpaper generator.
- `examples/setup_utility.rs` (`[[example]]` target `setup_utility`): Rust port of `setup_utility.py`, built on `grant_acquittal_workpapers::{argtheme, oserror, pyjson}` with zero added `Cargo.lock` dependencies.
  - Invokes `cargo run --locked --quiet --example utility_workflows -- ...` when the pipeline checkout includes `packages/monthly-close-control-plane/examples/utility_workflows.rs` and `Cargo.toml`.
  - Falls back to `utility_workflows.py` when replaying historical pipeline commits that predate the Rust port.

## Retained scripts

- `setup_utility.py`: kept alongside `examples/setup_utility.rs` until the CI/joined-workflow cutover PR removes `setup_utility.py` from `.github/workflows/ci.yml`'s 13-file maintained Python inventory and switches `.github/workflows/joined-latest-main.yml` to the Rust target.
- `tools/gen_unicode_tables.py`: retained under Constraint 2; generates `src/tables.rs` directly from CPython's `unicodedata` tables (`PY_SPACE`, `DECIMAL_ZEROS`, `PY_NOT_PRINTABLE`, `CATEGORY`).
- `tests/differential/*.py`, `tests/test_replay.py`, `tests/test_setup_utility.py`: differential oracle and regression suites for the retained Python entrypoint.
