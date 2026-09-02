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

CI also runs those checks on 3.10–3.13, plus CodeQL on the Python source. Do not expand the ruff rule set; it is `E9`/`F82` only, matching Monthly Close Controls.

For a behaviour change, add or update a focused test under `tests/`. Keep the
output deterministic: no wall-clock timestamps, client identifiers or hidden
state in a readiness pack.

## Pull requests

Explain which gate or boundary your change affects, include the test result, and
name any operational limitation that remains. Never present a review
acknowledgement as an approved file.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md), and
keep credentials, client data and exploit details out of the issue tracker.

## Releasing

Do not tag from a feature branch. Follow [RELEASING.md](RELEASING.md). For a
first PyPI publication, complete the one-time `pypi` environment and
trusted-publisher setup in that file before tagging.
