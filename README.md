# Monthly Close Control Plane

A small, **review-first** monthly-close control pack for a validated trial-balance export. It turns current and prior period trial balances into deterministic integrity checks, variance exceptions, reconciliation exceptions, and a human-review pack.

It is intentionally narrow:

```text
Validated trial-balance export
            |
            v
Exact control gates and variance checks
            |
            v
Explicit exception queue
            |
            v
Human review and workpaper acknowledgement
```

The first MVP accepts the canonical CSV written by [xero-trial-balance-export](https://github.com/ryanduguid/xero-trial-balance-export). It does **not** connect to Xero, store OAuth tokens, write journals, make payments, lodge BAS, lock a period, distribute a client report, or claim that a close has been approved.

## Why this exists

A close can be technically balanced and still need review. This tool keeps the evidence visible:

- Exact `Decimal` arithmetic—not binary floating point—for money controls.
- Schema, duplicate-key, date, and numeric gates fail closed.
- Current-period and YTD debits must exactly equal credits.
- Material YTD variances, new/missing accounts, account metadata changes, unmapped accounts, and supplied subledger differences become explicit exceptions.
- Output has only `PASS`, `REVIEW`, and `BLOCKED` states. A reviewer, not the tool, decides whether a close is acceptable.
- Source SHA-256 digests travel with the generated review pack so its source files can be identified later.

## Quick demo

The repository contains fabricated data only. Do not commit client trial balances, workpapers, exports, or credentials.

```bash
python -m pip install -e ".[dev]"

close-control review \
  --current examples/current_trial_balance.csv \
  --prior examples/prior_trial_balance.csv \
  --mapping examples/account_mapping.csv \
  --subledger examples/subledger_balances.csv \
  --absolute-threshold 10000 \
  --percentage-threshold 0.10 \
  --reconciliation-tolerance 0.01 \
  --review-note examples/review_note.json \
  --output outputs/demo
```

The demo exits `2` because its deliberately fabricated exceptions need human review. It writes:

- `close-summary.md` — a concise, deterministic review pack.
- `exceptions.csv` — filterable exception detail for Excel or Power BI.
- `close-review-pack.json` — structured evidence, thresholds, source hashes, and any supplied review acknowledgement.

Use exit code `0` only for an all-`PASS` pack, `2` for `REVIEW` or `BLOCKED`, and `1` for a malformed file or invalid command configuration.

## Canonical trial-balance contract

The initial input is the ten-column, normalised trial-balance schema from `xero-trial-balance-export`:

```text
ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit
```

`Tenant` plus `AccountID` is the control key. `AccountCode` and `AccountName` are display attributes, not stable identifiers. The loader rejects unknown/missing columns, duplicate control keys, malformed ISO dates, empty identifiers, and malformed monetary values.

The current-period `Debit`/`Credit` pair represents movement. `YTDDebit`/`YTDCredit` represents the position used for variance comparison. All values are read as exact decimals.

## Optional mapping and reconciliation inputs

An account mapping is a two-column CSV:

```text
AccountID,ReviewGroup
```

Any current TB account that is missing from a supplied mapping remains in the pack as a `REVIEW` exception. The mapping is a review label; it does not transform source numbers.

An optional subledger CSV must have:

```text
Tenant,AccountID,SubledgerBalance
```

`SubledgerBalance` must use the same signed convention as `YTDDebit - YTDCredit`: debit balances positive; credit balances negative. Each supplied subledger row is compared only with the matching current TB account. A missing GL account, or a difference beyond `--reconciliation-tolerance`, requires review.

## Human acknowledgement

If a reviewer wants the pack to record that it was read, supply a separate JSON file:

```json
{
  "reviewer_initials": "RD",
  "reviewed_on": "2026-08-08",
  "comment": "Reviewed demo exceptions; no client close was approved by this example."
}
```

An acknowledgement is evidence of a human action only. It **never** changes `REVIEW` or `BLOCKED` to `PASS`, and it never asserts that a period has been closed.

## Data and operational boundaries

- Use a separate, access-controlled working directory for client source files and outputs.
- Keep this checkout limited to fabricated fixtures. Its `.gitignore` deliberately blocks CSVs outside `examples/` and `schemas/`, and blocks generated packs.
- Produce the source CSV through a read-only export workflow. Live Xero OAuth, token storage, and client authorisation are deliberately outside this MVP.
- Do not use this as tax, financial, audit, or legal advice. It is a configurable review aid that requires professional judgement.

## Development

```bash
python -m pip install -e ".[dev]"
pytest
python -m build
```

The test suite covers schema gates, exact balancing, variance and metadata exceptions, mapping and subledger checks, deterministic pack generation, acknowledgement parsing, and the command-line exit contract.

Continuous integration verifies the committed `uv.lock`, runs the test suite on Python 3.10, 3.12, and 3.13, then builds and smoke-tests the wheel with the fabricated demo. CodeQL scans the Python source, and Dependabot is configured to propose updates for `uv` dependencies and pinned GitHub Actions. See [CONTRIBUTING.md](CONTRIBUTING.md) for the local verification and data-handling requirements.

## Roadmap

The next layers are deliberately separated from the control engine:

1. A safe, read-only Xero AI review gateway with a defined query allowlist, redaction boundary, source evidence, and no mutation tools.
2. A tax-change impact monitor that records authoritative source versions, produces drafts for human review, and never turns legislation changes into an automatic client conclusion.

See [docs/follow-on-safety-layers.md](docs/follow-on-safety-layers.md) for the intended boundary contracts.


MIT licensed.
