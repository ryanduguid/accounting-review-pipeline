# Porting status: `monthly-close-control-plane`

## Ported to Rust

- `src/main.rs` (`[[bin]]` target `close-control`): native Rust close control CLI and verification suite.
- `examples/clearing_demo.rs`, `examples/equity_demo.rs`, `tools/fuzz.rs`, `tools/differential.rs`: native Rust tools and examples.
- `examples/utility_results.rs` and `examples/utility_workflows.rs` (`[[example]]` target `utility_workflows`): Rust port of `examples/utility_results.py` and `examples/utility_workflows.py`, reusing `monthly_close_control_plane::{decimal, pyjson, pypath, sha256}` with zero added `Cargo.lock` dependencies.
  - Includes a self-contained `[tool.uv.workspace]` TOML parser and `PurePath.match` glob matcher for Python companion lock resolution.
  - Kills each timed-out or failed subprocess with its descendants (a POSIX session and `killpg` on Unix, a job object on Windows).

## Retained scripts

- `examples/utility_results.py` and `examples/utility_workflows.py`: kept whole for the companion repositories' joined-fixture workflows and documentation, which call `utility_workflows.py`. This repository's CI runs the Rust example.
- `examples/close-loop.ps1`: retained under Constraint 1/2 as a local Windows PowerShell wrapper around `close-control` and `elizabeth-anne-alexander`.
- `tests/test_utility_results.py`, `tests/test_utility_workflows.py`, `tests/conftest.py`: retained test coverage for the `.py` entrypoints whilst they remain in the maintained inventory.
