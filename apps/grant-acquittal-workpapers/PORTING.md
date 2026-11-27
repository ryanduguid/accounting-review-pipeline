# Porting status: `grant-acquittal-workpapers`

## Ported to Rust

- `src/main.rs` (`[[bin]]` target `grant-workpaper`): native Rust workpaper generator.
- `examples/setup_utility.rs` (`[[example]]` target `setup_utility`): Rust port of `setup_utility.py`, which it replaced; `tests/fixtures/setup_utility_cli.json` holds the Python script's recorded command-line cases as its regression test. Built on `grant_acquittal_workpapers::{argtheme, oserror, pyjson}` with zero added `Cargo.lock` dependencies.
  - Invokes `cargo run --locked --quiet --example utility_workflows -- ...` when the pipeline checkout includes `packages/monthly-close-control-plane/examples/utility_workflows.rs` and `Cargo.toml`.
  - Falls back to `utility_workflows.py` when replaying historical pipeline commits that predate the Rust port.

## Retained scripts

- `tools/gen_unicode_tables.py`: retained under Constraint 2; generates `src/tables.rs` directly from CPython's `unicodedata` tables (`PY_SPACE`, `DECIMAL_ZEROS`, `PY_NOT_PRINTABLE`, `CATEGORY`).
- `tests/differential/*.py`: the differential harness that compares `grant-workpaper` with the frozen Python release.
