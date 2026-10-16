# Review workflow 1.0 migration

Use the native `review-workflow` binary for the six existing commands: `build`,
`html`, `fixtures`, `verify`, `export` and `verify-export`. The Python workflow
module and its import API are removed. This is a binary-only major version;
the application is not published as a Python package.

Build and install the three binaries using [review-workflow.md](review-workflow.md).
The workflow needs no Python interpreter. Its default producer directory is the
directory containing its own executable. Pass `--bin-dir` to select another
directory; the previous Python default was the checkout's `.venv/Scripts`.

The fixed case and six fabricated source CSVs are embedded at compile time and
checked against the case's hashes. Moving the binary does not require the build
checkout. Editing external sample files does not alter native workflow inputs;
rebuild after an approved fixture change. Verification still checks the sealed
run's case, source bytes, derived trial balances, journal details and receipts.
`verify-export` needs only the binary and ZIP, with no installed producers or
external fixtures.

## Contracts and limits

The workflow retains the status prefix and successful status line. Successful
commands exit 0, refused operations exit 1 and argument errors exit 2. Amounts
use a 40-significant-digit decimal context. The fixture generator continues to
use its separate 28-digit context and ignores command arguments.

The two-month comparison checks sealed files, projected CSVs, HTML and canonical
stored ZIP bytes against the same frozen Python implementation and native
producers. Only the exact temporary run roots in three producer stdout logs and
the invocation start/finish times are normalised. Each original receipt and all
sealed file hashes are checked before that comparison.

The following 1.0 differences apply:

- Invoke `review-workflow`, replacing `python -B tools/review_workflow.py`.
- Use `--bin-dir` when producers are not beside the workflow executable.
- `--version` reports `review-workflow 1.0.0`. Help layout, argument diagnostics
  and operating-system error wording can differ from argparse/Python.
- JSON strings must contain valid Unicode scalar values. Escaped lone UTF-16
  surrogates are refused. Driver counts are non-negative integers within `u64`;
  Python could represent larger integers. The fixed case stays within this range.
- The binary embeds the fabricated inputs. The fixture generator's existing
  output location remains the `samples/` directory of its build checkout.

Native producer identity binds executable bytes and package/version text. It
does not authenticate the publisher or isolate concurrent local processes.
Archive verification checks unsigned internal byte consistency. Anyone able to
replace the full archive can replace its hashes. No command approves accounting,
posts a journal or closes a period.

## Reproduce the frozen comparison

Use CPython 3.15 and a separate checkout of
`d48b9fe681f3a3ae278a90c43cfdce9b863589d4` as the oracle. Its Power BI application
directory contains the original `tools/review_workflow.py` and fabricated samples.
Python is required here to execute the original implementation; it is test
tooling only. Build the two native producers and supply their shared directory.

From this application directory:

```text
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --all-features --bins
target/debug/workflow-differential --python PATH_TO_PYTHON_3_15 --oracle PATH_TO_ORACLE_APP --bin-dir PATH_TO_NATIVE_PRODUCERS --seed 16
target/debug/workflow-differential --python PATH_TO_PYTHON_3_15 --oracle PATH_TO_ORACLE_APP --bin-dir PATH_TO_NATIVE_PRODUCERS --seed 17
```

The harness creates disposable runs outside the checkout. It checks both months,
all six commands, previous-run HTML, resealed semantic tampering and generated
JSON/schema/ledger/CSV/ZIP cases. Native tests cover linked paths, single-link
readiness state, probe deadlines, output cleanup and archive admission. This
does not establish complete equivalence to every removed Python test method.

The retained generator and structural comparison scripts in `tests/differential/`
use their earlier baselines and remain separate checks. Microsoft PBIR validation
checks report structure. Power BI Desktop refresh, screen-reader acceptance,
macOS, hosted CI for this port and Aikido scans need separate verification.
