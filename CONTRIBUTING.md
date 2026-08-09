# Contributing

Contributions should preserve this project's narrow role: a local, deterministic review-pack generator. It must not gain authority to post journals, make payments, lodge returns, lock periods, send reports, or approve a close.

## Data boundary

- Use fabricated fixtures only. Do not commit client trial balances, subledgers, workpapers, review packs, credentials, `.env` files, tokens, or screenshots from a live accounting system.
- Keep fabricated CSV fixtures under `examples/`; the repository's `.gitignore` blocks ordinary CSV files elsewhere.
- Treat source CSV content and optional review notes as untrusted input. Changes must retain the existing fail-closed validation and spreadsheet-formula safeguards.

## Local verification

The supported runtime is Python 3.10 or newer. The repository uses `uv` and commits its lock file.

```bash
uv lock --check
uv sync --locked --all-extras
uv run pytest
uv build
```

For behaviour changes, add or update a focused test under `tests/`. Keep output deterministic: do not add wall-clock timestamps, client identifiers, or hidden state to review packs.

## Pull requests

Explain the control or boundary affected, include the relevant test result, and state any operational limitation that remains. A review acknowledgement must never be represented as an approved or completed close.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md) and do not publish credentials, client data, or exploit details in an issue.
