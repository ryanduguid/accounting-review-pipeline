# Porting status: `monthly-close-control-plane`

## Ported to Rust

- `src/main.rs` (`[[bin]]` target `close-control`): native Rust close control CLI and verification suite.
- `examples/clearing_demo.rs`, `examples/equity_demo.rs`, `tools/fuzz.rs`, `tools/differential.rs`: native Rust tools and examples.
- `examples/utility_results.rs` and `examples/utility_workflows.rs` (`[[example]]` target `utility_workflows`): Rust port of `examples/utility_results.py` and `examples/utility_workflows.py`, reusing `monthly_close_control_plane::{decimal, pyjson, pypath, sha256}` with zero added `Cargo.lock` dependencies.
  - Includes a self-contained `[tool.uv.workspace]` TOML parser and `PurePath.match` glob matcher for Python companion lock resolution.
  - Uses cross-platform process-group isolation (`setsid`/`killpg` on Unix, `CREATE_NEW_PROCESS_GROUP`/`taskkill` on Windows) for timeout cleanup.

## Retained scripts

- `examples/utility_results.py` and `examples/utility_workflows.py`: kept intact until a separate CI/joined-workflow cutover PR updates `.github/workflows/joined-fixtures.yml`, `.github/workflows/joined-latest-main.yml` and `.github/workflows/ci.yml`'s 13-file maintained Python inventory check.
- `examples/close-loop.ps1`: retained under Constraint 1/2 as a local Windows PowerShell wrapper around `close-control` and `elizabeth-anne-alexander`.
- `tests/test_utility_results.py`, `tests/test_utility_workflows.py`, `tests/conftest.py`: retained test coverage for the `.py` entrypoints whilst they remain in the maintained inventory.
