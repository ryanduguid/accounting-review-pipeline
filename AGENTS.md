# Accounting Review Pipeline agent instructions

This repository is the local assembly of the Accounting Review Pipeline: six independently
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
  component, and no review package may import the exporter.
- Do not add a root package manager, shared runtime package, unified version, generated
  dependency graph or monorepo framework. Movement and behaviour changes are separate pull
  requests.
- Only workflows under the root `.github/workflows/` are active. Nested `.github/`
  directories inside components are inert historical records imported with their sources;
  do not run them and do not treat their pins as current.
- Release only through the root callers `.github/workflows/release-<component>.yml` on a
  namespaced annotated tag `<component>/vMAJOR.MINOR.PATCH`, never through a nested
  `release.yml`. One tag publishes exactly one component.
- Never commit client data, workpapers, credentials, tokens, generated review packs or native
  application evidence containing client data.

## Path decision

Component directories use each component's normalised distribution name because the
reviewed Release Policy identity gate requires a nested release's directory leaf,
`tag-prefix` and distribution name to be identical. The migration plan's
`packages/monthly-close-controls`, `packages/workpaper-review-gate` and
`packages/xero-ledger-review-gate` therefore became `packages/monthly-close-control-plane`,
`packages/review-ready-gate` and `packages/elizabeth-anne-alexander`. The exporter, the Excel
adapter and the Power BI application keep the plan's paths. `IMPORTS.md` records the decision.

## Command routing

Run every check from the owning component directory with its documented commands:

| Component | Directory | Checks |
|---|---|---|
| Xero Trial Balance Export | `packages/xero-trial-balance-export/` | `python -m pip install --require-hashes -r requirements.lock`; `python -m unittest discover -s tests -v`; Ruff and mypy over `xero_client.py export_tb.py auth.py token_store.py` |
| Workpaper Review Gate | `packages/review-ready-gate/` | `uv lock --check`; `uv run --locked --extra dev pytest -q`; Ruff over `reviewready tests`; mypy over `reviewready`; `python -m build`; clean-wheel `review-ready gate` runs |
| Monthly Close Controls | `packages/monthly-close-control-plane/` | its `AGENTS.md` CI gates and Windows clean-wheel smoke |
| Xero Ledger Review Gate | `packages/elizabeth-anne-alexander/` | `uv lock --check`; `uv run --locked --extra dev pytest`; Ruff over `elizabeth_anne_alexander tests`; mypy over `elizabeth_anne_alexander`; `python -m build`; clean-wheel `evaluate` and `validate-review` demo |
| Accounting Excel Toolkit | `adapters/accounting-excel-toolkit/` | pinned actionlint and ShellCheck; `python -B -m unittest discover -s tests -v`; optional `tools/native_excel_acceptance.ps1` on Windows with Excel |
| Australian Accounting Power BI | `apps/australian-accounting-power-bi/` | `python -B -m unittest discover -s tests -v`; `npx --yes @microsoft/powerbi-report-authoring-cli@0.1.4 validate australian-accounting-power-bi.Report` |

A change to the shared Xero trial-balance contract directory (`contracts/xero-trial-balance-v1/`) must run the exporter, all three review packages,
the Excel adapter, Power BI structural validation and the joined conformance test.
