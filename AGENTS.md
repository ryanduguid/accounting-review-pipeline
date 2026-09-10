# Accounting Review Pipeline agent instructions

This repository is the local assembly of the Accounting Review Pipeline: seven independently
versioned components joined only by local files and commands. Its canonical GitHub repository
is `ryanduguid/accounting-review-pipeline`.
Follow the closest component `AGENTS.md`, `CONTRIBUTING.md` or `README.md` for component work.
These repository-wide rules apply everywhere:

- Only `packages/xero-trial-balance-export/` may use OAuth, call Xero, use an HTTP client,
  hold tokens or read Xero credentials (the environment names documented in the exporter's
  `.env.example`). Review packages, the Excel adapter and the Power BI application stay
  offline, read local files, parse money with exact `Decimal` arithmetic and ship
  fabricated data only.
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
  licence to depend on a sibling.
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
adapter and the Power BI application keep the plan's paths. `IMPORTS.md` records the decision.

## Setup

From a fresh clone, `uv sync` at the root installs the five Python components as editable
workspace members and the shared test toolchain into one `.venv`, and `just test` runs
every component's suite plus the joined conformance test. That is the whole setup. `just`
comes from `uv tool install rust-just`; its recipes are `setup`, `lint`, `typecheck`,
`test` and `check` (the last three together). Each recipe loops over the per-component
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
| Monthly Close Controls | `packages/monthly-close-control-plane/` | its `AGENTS.md` CI gates and Windows clean-wheel smoke |
| Xero Ledger Review Gate | `packages/elizabeth-anne-alexander/` | the shared component gates below, scoped to `elizabeth_anne_alexander` |
| Accounting Excel Toolkit | `adapters/accounting-excel-toolkit/` | `python -B -m unittest discover -s tests -v`; optional `tools/native_excel_acceptance.ps1` on Windows with Excel |
| Australian Accounting Power BI | `apps/australian-accounting-power-bi/` | `python -B -m unittest discover -s tests -v`; `npx --yes @microsoft/powerbi-report-authoring-cli@0.1.4 validate australian-accounting-power-bi.Report` |
| evatt | `packages/evatt/` | the shared component gates below, scoped to `evatt` |

The shared component gates are defined once in `.github/workflows/ci-package.yml`, which
`ci.yml` calls for the exporter, Workpaper Review Gate, Xero Ledger Review Gate and evatt
with the component directory. Run them from the component directory:

```bash
uv lock --check
uv run --locked --extra dev ruff check .
uv run --locked --extra dev mypy
uv run --locked --extra dev pytest --cov --cov-branch --cov-report=term-missing --cov-report=xml
uv run --locked --extra dev --with "pip-audit==2.10.1" pip-audit --local --strict
uv run --locked --extra dev --python 3.12 python -m build
```

The tests run on Python 3.10, 3.12 and 3.13, and the build gate installs the wheel into a
clean virtual environment and imports the component's package from it. Each call filters
itself to its component directory, the shared contract and the root files, so a change to
one component runs that component alone. Monthly Close Controls keeps its own `test`,
`package` and `lint` jobs in `ci.yml` because they are the anchor required checks on
`main`. The Excel adapter and the Power BI application have no `pyproject.toml`, so their
root workflows run ruff and mypy from `ruff.toml` and `mypy.ini` and their unittest suites
on the same Python matrix.

A change to the shared Xero trial-balance contract directory (`contracts/xero-trial-balance-v1/`) must run the exporter, all three review packages,
the Excel adapter, Power BI structural validation and the joined conformance test. A change
to the root `pyproject.toml`, `uv.lock` or `justfile` triggers every component workflow,
because the root lock is what `uv run --locked` validates inside every component directory.
