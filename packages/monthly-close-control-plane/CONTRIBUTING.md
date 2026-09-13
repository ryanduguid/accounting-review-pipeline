# Contributing

Keep this project in its narrow role: a local, deterministic review-pack generator. No contribution should give it authority to post journals, make payments, lodge returns, lock periods, send reports, or approve a close.

## Data boundary

- Use fabricated fixtures. Keep client trial balances, subledgers, workpapers, review packs, credentials, `.env` files, tokens and screenshots from a live accounting system out of the repository.
- Put fabricated CSV fixtures under `examples/` and header-only schema references under `schemas/`. The `.gitignore` blocks ordinary CSV files outside those 2 directories.
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

Some of these checks read files that live above this component and are not in
the source archive. Run them from a full `accounting-review-pipeline` checkout:
`tests/test_repository_guidance.py` reads the root `AGENTS.md`, `README.md` and
`.github/workflows/`; `tests/test_xero_trial_balance_contract.py` reads
`contracts/xero-trial-balance-v1/`; and
`test_repository_identity_is_distinct_from_package_identity` in
`tests/test_workflow_examples.py` reads the root release caller. From an
extracted source archive they fail for want of those files, not for a defect in
this package. `test_output_inside_repository_rejected_before_source_read` needs
the run to sit inside a version-control checkout, which an extracted archive is
not.

## Pull requests

Explain which control or boundary your change affects, include the test result, and name any operational limitation that remains. Never present a review acknowledgement as an approved or completed close.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md), and keep credentials, client data and exploit details out of the issue tracker.
