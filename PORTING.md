# Rust porting status

This repository retains a small set of Python and PowerShell scripts alongside its standalone Rust crates. Each retained script falls under an explicit constraint or an active CI cutover hold.

## Ported utility scripts

- `apps/grant-acquittal-workpapers/examples/setup_utility.rs` (`[[example]]` target `setup_utility`): Rust port of `apps/grant-acquittal-workpapers/setup_utility.py`, which it replaced. Uses the crate's existing `pyjson`, `oserror` and `argtheme` modules with zero new `Cargo.lock` dependencies, enforces workspace and `--summary` isolation, invokes the Rust `utility_workflows` example when present in the target pipeline checkout and falls back to `utility_workflows.py` when replaying older pipeline commits.
- `packages/monthly-close-control-plane/examples/utility_results.rs` and `packages/monthly-close-control-plane/examples/utility_workflows.rs` (`[[example]]` target `utility_workflows`): Rust port of `examples/utility_results.py` and `examples/utility_workflows.py`. Reuses the crate's `pyjson`, `pypath`, `decimal` and `sha2` helpers with zero new `Cargo.lock` dependencies, kills each timed-out or failed subprocess with its descendants (a POSIX session and `killpg` on Unix, a job object on Windows) and preserves the source mutation guard and provenance checks.

## Retained `.py` entrypoints (outside callers)

- `packages/monthly-close-control-plane/examples/utility_results.py`
- `packages/monthly-close-control-plane/examples/utility_workflows.py`

CI in this repository runs the Rust `setup_utility` and `utility_workflows` examples; `setup_utility.py` is gone. These 2 scripts stay whole because the companion repositories' joined-fixture workflows and documentation still call `utility_workflows.py`. They stay in `.github/workflows/ci.yml`'s maintained Python inventory, and their recorded command-line cases (`tests/fixtures/utility_workflows_cli.json`) are checked against both implementations.

## Retained scripts by constraint

### Constraint 1 and 2: Windows COM / desktop PowerShell tooling

- `adapters/accounting-excel-toolkit/tools/native_excel_acceptance.ps1`: automates desktop Microsoft Excel through `Excel.Application` COM and `Microsoft.Mashup.OleDb.1` to evaluate Power Query M modules.
- `apps/australian-accounting-power-bi/tools/*.ps1` (`measure_report_queries.ps1`, `native_receipts.ps1`, `prepare_native_project.ps1`, `test_benchmark_measures.ps1`, `test_financial_filters.ps1`, `test_native_project.ps1`, `test_refresh_guards.ps1`, `test_review_controls.ps1`, `test_review_evidence.ps1`, `test_review_presentation.ps1`, `test_review_projection.ps1`, `verify_native_model.ps1`): drive Windows Power BI Desktop, Analysis Services / ADOMD local model queries and Windows desktop checks that lack cross-platform Rust equivalents.
- `packages/monthly-close-control-plane/examples/close-loop.ps1`: Windows PowerShell orchestration wrapper invoking `close-control` and `elizabeth-anne-alexander` against local sample directories.

### Constraint 2: CPython frozen reference and Unicode/regex table generators

- `adapters/accounting-excel-toolkit/tools/benchmark_aged_review.py`: measures the frozen Python reference implementations under `tests/differential/python_reference/` (`debtor_evidence.py`, `xero_aged_receivables.py`) using CPython `tracemalloc` (`peak_traced_python_bytes`). See `adapters/accounting-excel-toolkit/PORTING.md`.
- `apps/grant-acquittal-workpapers/tools/gen_unicode_tables.py`
- `packages/elizabeth-anne-alexander/tools/gen_unicode_tables.py`
- `packages/evatt/tools/gen_unicode_tables.py`
- `packages/evatt/tools/gen_unicode_cases.py`
- `packages/evatt/tools/gen_patterns.py`
- `packages/evatt/tools/record_python_tests.py`

These generator scripts read CPython's frozen `unicodedata` database, internal `re._parser` AST and 0.2.0 test records to emit deterministic Rust lookup tables and test fixtures.

### Constraint 3: CI workflow helpers without a Rust build step

- `.github/ci/check_ca_bundle.py`
- `.github/ci/check_gates.py`
- `.github/ci/select_paths.py`

These scripts execute directly in GitHub Actions workflows using the runner's preinstalled Python without compiling a Rust binary first.

### Differential oracles and test suites (`tests/`)

Python files under `tests/` and component `tests/` directories are test suites and frozen migration oracles, excluded from production builds and from the maintained source inventory check in `.github/workflows/ci.yml`.
