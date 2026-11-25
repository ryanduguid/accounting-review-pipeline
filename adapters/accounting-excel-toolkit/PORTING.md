# Porting status: `accounting-excel-toolkit`

## Native Rust targets

- `check-vba-encoding` (`src/main.rs`)
- `xero-account-transactions` (`src/transactions_main.rs`)
- `xero-aged-receivables` (`src/aged_main.rs`)
- `debtor-evidence` (`src/debtor_main.rs`)
- `replay-aged-review` (`src/replay_main.rs`)

## Retained scripts and assessment

- `tools/benchmark_aged_review.py` (116 lines): inspected and retained under **Constraint 2** (frozen CPython reference + `tracemalloc`) and **Constraint 3** / CI inventory.
  - Every function in `benchmark_aged_review.py` (`workload`, `measure`, `benchmark`) either constructs synthetic zero-contact CSV/JSON fixtures for the benchmark or directly imports and profiles the frozen Python reference implementations (`tests/differential/python_reference/debtor_evidence.py::briefing` and `tests/differential/python_reference/xero_aged_receivables.py::inspect_export`) using CPython's `tracemalloc.get_traced_memory()` (`peak_traced_python_bytes`).
  - There is no separate production or non-benchmark component to port; the production aged-review pipeline (`xero-aged-receivables`, `debtor-evidence`, `replay-aged-review`) is already Rust.
  - Boundary: `benchmark_aged_review.py` measures only the frozen Python reference and CPython traced allocations (never the Rust binaries, RSS or production SLAs), as also asserted by `tests/test_benchmark_reference.py`.
- `tools/native_excel_acceptance.ps1`: retained under **Constraint 1/2** because it automates desktop Microsoft Excel COM (`Excel.Application`) and Power Query (`Microsoft.Mashup.OleDb.1`) on Windows.
- `tests/differential/*.py`: retained differential test oracles against the frozen Python implementations.
