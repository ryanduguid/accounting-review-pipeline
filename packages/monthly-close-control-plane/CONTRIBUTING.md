# Contributing

Keep this project in its narrow role: a local, deterministic review-pack generator. No contribution should give it authority to post journals, make payments, lodge returns, lock periods, send reports, or approve a close.

## Data boundary

- Use fabricated fixtures. Keep client trial balances, subledgers, workpapers, review packs, credentials, `.env` files, tokens and screenshots from a live accounting system out of the repository.
- Put fabricated CSV fixtures under `examples/` and header-only schema references under `schemas/`. The `.gitignore` blocks ordinary CSV files outside those two directories.
- Treat source CSV content and review notes as untrusted input. Keep the fail-closed validation and the spreadsheet-formula safeguards in place.

## Local verification

Python 3.10 or newer. The repository uses `uv` and commits its lock file.

```bash
uv lock --check
uv sync --locked --all-extras
uv run pytest
uv build
```

For a behaviour change, add or update a focused test under `tests/`. Keep the output deterministic: no wall-clock timestamps, client identifiers or hidden state in a review pack.

## Pull requests

Explain which control or boundary your change affects, include the test result, and name any operational limitation that remains. Never present a review acknowledgement as an approved or completed close.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md), and keep credentials, client data and exploit details out of the issue tracker.
