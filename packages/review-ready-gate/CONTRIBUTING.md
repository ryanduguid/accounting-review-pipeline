# Contributing

Keep this project in its narrow role: a local, deterministic readiness gate.
No contribution should give it authority to post journals, make payments, lodge
returns, lock periods, send reports, or approve a file.

## Data boundary

- Use fabricated fixtures. Keep client trial balances, subledgers, workpapers,
  readiness packs, credentials, `.env` files, tokens and screenshots from a live
  accounting system out of the repository.
- Put fabricated CSV fixtures under `examples/` and schema references
  (header-only CSVs, plus `self_review.json`) under `schemas/`. The `.gitignore`
  blocks ordinary CSV files outside those directories.
- Treat source CSV content and review notes as untrusted input. Keep the
  fail-closed validation and the spreadsheet-formula safeguards in place.

## Local verification

Python 3.10 or newer. The repository uses `uv` and commits its lock file.

```bash
uv lock --check
uv sync --locked --all-extras
uv run pytest
uv run ruff check reviewready tests
uv run mypy reviewready
uv build
```

CI also runs those checks on Python 3.10, 3.12, 3.13 and 3.14, plus CodeQL on the Python source. Do not expand the ruff rule set; it is `E4`, `E7`, `E9`, `F` and `I`, matching Monthly Close Controls.

For a behaviour change, add or update a focused test under `tests/`. Keep the
output deterministic: no wall-clock timestamps, client identifiers or hidden
state in a readiness pack.

Two checks read files that live above this component and are not in the source
archive. Run them from a full `accounting-review-pipeline` checkout:
`tests/test_xero_trial_balance_contract.py` reads
`contracts/xero-trial-balance-v1/`, and
`test_current_release_metadata_uses_immutable_documentation` in
`tests/test_workflow_examples.py` reads the root release caller. From an
extracted source archive they fail for want of those files, not for a defect in
this package.

## Pull requests

Explain which gate or boundary your change affects, include the test result, and
name any operational limitation that remains. Never present a review
acknowledgement as an approved file.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md), and
keep credentials, client data and exploit details out of the issue tracker.

## Releasing

Do not tag from a feature branch. Follow [RELEASING.md](RELEASING.md). For the
first monorepo publication, complete the one-time `pypi-review-ready-gate`
environment and trusted-publisher setup in that file before tagging.
