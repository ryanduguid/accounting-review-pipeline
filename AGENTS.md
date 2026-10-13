# Accounting Review Pipeline agent instructions

This repository is the local assembly of the Accounting Review Pipeline: 8 independently
versioned components joined only by local files and commands. Its canonical GitHub repository
is `ryanduguid/accounting-review-pipeline`.
Follow the closest component `AGENTS.md`, `CONTRIBUTING.md` or `README.md` for component work.
These repository-wide rules apply everywhere:

- Only `packages/xero-trial-balance-export/` may use OAuth, call Xero, use an HTTP client,
  hold tokens or read Xero credentials (the environment names documented in the exporter's
  `.env.example`). Review packages, the Excel adapter and the Power BI application stay
  offline, read local files and ship fabricated data only. Python review packages
  parse and calculate money with exact `Decimal` arithmetic, and the Rust Xero Ledger
  Review Gate reproduces that arithmetic exactly. Describe the Excel
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
- The root `pyproject.toml`, `uv.lock` and `justfile` are a development entrypoint only.
  The root is a virtual uv workspace: it declares no package, no version and no runtime
  dependency, and publishes nothing. Do not add a root distribution, shared runtime library,
  unified version, generated dependency graph or monorepo framework. Movement and behaviour
  changes are separate pull requests.
- `uv run --locked` inside a component directory validates the root `uv.lock`, not the
  component's. After changing any component's dependencies, run `uv lock` at the root and
  commit the result alongside the component's own `uv.lock`, which stays the authority for
  building and releasing that component alone.
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

From a fresh clone, `uv sync` at the root installs the 3 Python components as editable
workspace members and the shared test toolchain into one `.venv`, and `just test` runs
every component's suite plus the joined conformance test. That is the whole setup. `just`
comes from `uv tool install rust-just`; its recipes are `setup`, `lint`, `typecheck`,
`test` and `check` (the last 3 together). Each recipe loops over the per-component
commands in the table below, which remain the authority. `just check` is not a CI
equivalent: it does not verify component lockfiles, install the exporter's hash-locked
requirements, build or smoke-test a wheel, run actionlint, the Power BI CLI
or CodeQL, or check the contract digests.

## Command routing

Run every check from the owning component directory with its documented commands:

| Component | Directory | Checks |
|---|---|---|
| Xero Trial Balance Export | `packages/xero-trial-balance-export/` | the shared component gates below (mypy and coverage scoped to `xero_client.py export_tb.py auth.py token_store.py` by its `pyproject.toml`), plus `python -m pip install --require-hashes -r requirements.lock` and `python -m unittest discover -s tests -v` from that hash-locked environment |
| Workpaper Review Gate | `packages/review-ready-gate/` | the shared component gates below, scoped to `reviewready` |
| Monthly Close Controls | `packages/monthly-close-control-plane/` | its `AGENTS.md` CI gates, which are the `test`, `dependency-audit`, `package` and `lint` jobs `ci.yml` keeps for this component; the package job and manual Windows recipe run the same installed-command smoke helper |
| Xero Ledger Review Gate | `packages/elizabeth-anne-alexander/` | `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings` and `cargo test --locked`, on Linux and Windows; `tests/differential/differential.py` compares the binary with the 0.2.6 Python release |
| Accounting Excel Toolkit | `adapters/accounting-excel-toolkit/` | `python -B -m unittest discover -s tests -v`; optional `tools/native_excel_acceptance.ps1` on Windows with Excel |
| Australian Accounting Power BI | `apps/australian-accounting-power-bi/` | `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`; `python -B -m unittest discover -s tests -v`; `npm ci --ignore-scripts` then `npx --no-install powerbi-report-author validate australian-accounting-power-bi.Report` |
| Grant Acquittal Workpapers | `apps/grant-acquittal-workpapers/` | `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked` for the Rust workpaper; `python -B -m unittest discover -s tests -v` for the setup and replay utility; both on Linux and Windows; Ruff from its `pyproject.toml` and mypy from its `mypy.ini` |
| evatt | `packages/evatt/` | `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings` and `cargo test --locked --all-features`; the differential probe is enabled only with `--features probe` |

The shared component gates are defined once in `.github/workflows/ci-package.yml`, which
`ci.yml` calls for the exporter and Workpaper Review Gate with the component
directory. `ci.yml`'s `rust` job runs the Xero Ledger Review Gate and evatt cargo checks. Run them from the component directory:

```bash
uv lock --check
uv run --locked --extra dev ruff check .
uv run --locked --extra dev mypy
uv run --locked --extra dev pytest --cov --cov-branch --cov-report=term-missing --cov-report=xml
uv run --locked --extra dev --with "pip-audit==2.10.1" pip-audit --local --strict
uv run --locked --extra dev --python 3.14 python -m build
```

The tests run on Python 3.14 and 3.15, and the build gate installs the wheel into a
clean virtual environment and imports the component's package from it. Every gate above
runs on Linux; `ci.yml`'s `windows` job adds the same pytest command on `windows-latest` for
the exporter and Monthly Close Controls, and the `rust` job runs on Windows too, covering
the 3 components with platform-specific behaviour. Each call filters
itself to its component directory, the shared contract and the root files, so a change to
one component runs that component alone. Monthly Close Controls keeps its own `test`,
`package` and `lint` jobs in `ci.yml` because its release workflow selects them by
name. The Excel adapter, the Power BI application and the grant workpapers application are
not workspace members, so their root workflow runs ruff and mypy from their own settings and
their unittest suites on the same Python matrix, with a Windows leg for the grant application.
The grant workpaper is a Rust crate from 0.2.0; the same workflow's `rust` job checks it on
Linux and Windows. The Power BI application's fixture generator and structural checks are a
Rust crate that the same job checks; its review workflow and that workflow's tests stay Python
until the close-control and review-ready producers it inspects are Rust.

A change to the shared Xero trial-balance contract directory (`contracts/xero-trial-balance-v1/`) must run the exporter, all 3 review packages,
the Excel adapter, Power BI structural validation and the joined conformance test. A change
to the root `pyproject.toml`, `uv.lock` or `justfile` triggers every component workflow,
because the root lock is what `uv run --locked` validates inside every component directory.
