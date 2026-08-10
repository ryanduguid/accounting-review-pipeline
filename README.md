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

The first MVP accepts the canonical CSV written by [xero-trial-balance-export](https://github.com/ryanduguid/xero-trial-balance-export). Each file must contain exactly one tenant and one report date; current and prior files must name the same tenant, and the prior date must be earlier. It does **not** connect to Xero, store OAuth tokens, write journals, make payments, lodge BAS, lock a period, distribute a client report, or claim that a close has been approved.

## Why this exists

A close can be technically balanced and still need review. This tool keeps the evidence visible:

- Exact `Decimal` arithmetic—not binary floating point—for money controls.
- Schema, duplicate-key, date, and numeric gates fail closed.
- Current-period and YTD debits must exactly equal credits.
- Material YTD variances, new/missing accounts, account metadata changes, unmapped accounts, and supplied subledger differences become explicit exceptions.
- Output has only `PASS`, `REVIEW`, and `BLOCKED` states. A reviewer, not the tool, decides whether a close is acceptable.
- Source SHA-256 digests travel with the generated review pack so its source files can be identified later.
- Spreadsheet-facing CSV text beginning with `=` is always neutralised with a leading apostrophe. `+`, `-` and `@` are neutralised only where the rest of the value could be read as a formula, so an account code like `-1000` or an ID like `@123` stays joinable in Excel and Power BI. Reviewer-note text is flattened before Markdown rendering.
- `exceptions.csv` is written with a UTF-8 byte-order mark, matching the canonical input files, so a spreadsheet reads non-ASCII entity and account names correctly.
- The three pack files are staged beside their destinations and moved into place only once all three have been written. If one cannot be replaced — a reviewer holding `exceptions.csv` open is the usual cause — the files already moved are rolled back to the content they replaced, so the previous pack survives whole instead of half describing one trial balance and half describing another. A failed run never deletes a pack file it did not write. Run one export at a time into a given `--output` directory; concurrent runs are not serialised.
- Amounts are rendered with at least two decimal places and never fewer than the value carries. A percentage is rendered with at least two places and always enough to show its leading significant digit, so neither a tolerance finer than one cent nor a threshold finer than a hundredth of a percent is flattened to `0.00`.

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

Use exit code `0` only for an all-`PASS` pack, `2` for `REVIEW` or `BLOCKED`, and `1` for a malformed file, an invalid command configuration, or an `--output` path that cannot be written.

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
- Keep this checkout limited to fabricated fixtures. Its `.gitignore` blocks CSVs outside `examples/` and `schemas/`, and blocks all three generated pack files by name wherever `--output` points them, including inside those two fixture directories.
- Produce the source CSV through a read-only export workflow. Live Xero OAuth, token storage, and client authorisation are deliberately outside this MVP.
- Do not use this as tax, financial, audit, or legal advice. It is a configurable review aid that requires professional judgement.

## Development

```bash
python -m pip install -e ".[dev]"
pytest
python -m build
```

The test suite covers schema gates, exact balancing, variance and metadata exceptions, mapping and subledger checks, deterministic pack generation, acknowledgement parsing, and the command-line exit contract.

Continuous integration verifies the committed `uv.lock`, runs the test suite on Python 3.10, 3.11, 3.12, and 3.13, then builds and smoke-tests the wheel with the fabricated demo. CodeQL scans the Python source, and Dependabot is configured to propose updates for `uv` dependencies and pinned GitHub Actions. See [CONTRIBUTING.md](CONTRIBUTING.md) for the local verification and data-handling requirements.

## Roadmap

The next layers are deliberately separated from the control engine:

1. A safe, read-only Xero AI review gateway with a defined query allowlist, redaction boundary, source evidence, and no mutation tools.
2. A tax-change impact monitor that records authoritative source versions, produces drafts for human review, and never turns legislation changes into an automatic client conclusion.

See [docs/follow-on-safety-layers.md](docs/follow-on-safety-layers.md) for the intended boundary contracts.

Built with AI assistance (Claude); design, review, and testing by the author.

MIT licensed.
