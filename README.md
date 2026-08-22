# Russell Mathews

[![tests](https://github.com/ryanduguid/monthly-close-control-plane/actions/workflows/ci.yml/badge.svg)](https://github.com/ryanduguid/monthly-close-control-plane/actions/workflows/ci.yml) [![License: MIT](https://img.shields.io/badge/License-MIT-4F485E.svg?labelColor=04001F)](LICENSE) [![Python 3.10+](https://img.shields.io/badge/python-3.10%2B-5C2D91.svg?logo=python&logoColor=white&labelColor=04001F)](https://www.python.org/downloads/)

The repository name is the public project identity; the `monthly-close-control-plane` distribution and `close-control` command remain compatibility identifiers.

A small, **review-first** monthly-close control pack for a validated trial-balance export. You point it at a current and a prior trial balance and it hands you an exception pack for close review:

- `close-summary.md` answers "what needs my attention this close?": a concise, deterministic review pack with an overall status, the thresholds used, source evidence, and an exception table a reviewer reads top to bottom.
- `exceptions.csv` answers "which accounts, by how much, and why?": filterable exception detail for Excel or Power BI, one row per exception with values, differences, thresholds, and a suggested reviewer action.
- `close-review-pack.json` answers "what exactly did this run look at?": structured evidence, thresholds, source hashes, and any supplied review acknowledgement, for archiving or downstream tooling.

The pack surfaces material YTD variances, new and missing accounts, account metadata changes, unmapped accounts, and supplied subledger differences as explicit exceptions. Output has only `PASS`, `REVIEW`, and `BLOCKED` states. A reviewer, not the tool, decides whether a close is acceptable.

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

The first MVP accepts the canonical CSV written by [xero-trial-balance-export](https://github.com/ryanduguid/xero-trial-balance-export) (`xero-trial-balance-export`). Each file must contain exactly one tenant and one report date; current and prior files must name the same tenant, and the prior date must be earlier. It does **not** connect to Xero, store OAuth tokens, write journals, make payments, lodge BAS, lock a period, distribute a client report, or claim that a close has been approved.

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

The demo exits `2` because its deliberately fabricated exceptions need human review. It writes the three pack files described above.

Use exit code `0` only for an all-`PASS` pack, `2` for `REVIEW` or `BLOCKED`, and `1` for a malformed file, an invalid command configuration, or an `--output` path that cannot be written.

To run the check on a schedule in CI, copy [examples/github-actions-close-check.yml](examples/github-actions-close-check.yml) into `.github/workflows/`.
It runs against a repo-stored synthetic trial balance and fails the job when the pack is `BLOCKED`.

To run this pack against the sibling gateway's same-financial-year sample CSVs (not the Acme June/July demo pair, which crosses the 1 July reset), see [examples/close-loop.md](examples/close-loop.md). That loop is local files only; it does not connect to Xero.

## Local close workbench

`close-control workbench` is a local façade over the same validation, control
engine, and three-file writer used by `close-control review`. It is useful when
the close process starts with two already-created canonical exports in an
access-controlled directory outside this repository:

```bash
close-control workbench \
  --current C:\close-data\current.csv \
  --prior C:\close-data\prior.csv \
  --mapping C:\close-data\account-mapping.csv \
  --subledger C:\close-data\subledger.csv \
  --output C:\close-data\review-pack
```

It writes exactly `close-summary.md`, `exceptions.csv`, and
`close-review-pack.json`. Open or import `exceptions.csv` in Excel or Power
Query if useful, then investigate and document conclusions through your normal
workpaper process. The command never starts Excel, creates a workbook, calls
Xero, reads OAuth credentials or tokens, calls an AI service, posts anything,
or copies the supplied sources. It records review evidence only; it does not
approve or close a period.

Keep inputs and output outside the checkout: repository fixtures remain
fabricated, and the existing `.gitignore` rules deliberately prevent ordinary
exports and generated packs becoming repository content.

## Worked example

Running the quick-demo command above against the fabricated fixtures in `examples/` prints:

```text
close-control: REVIEW; 8 exception(s)
  json: outputs/demo/close-review-pack.json
  summary: outputs/demo/close-summary.md
  exceptions: outputs/demo/exceptions.csv
```

`close-summary.md` opens with the status, scope, and source digests, then lists every exception (abridged here to four of the eight rows):

```markdown
# Monthly Close Review Pack

**Overall status: REVIEW**

This pack is a review aid. It does not approve a close, post a journal, make a payment, lodge a return, or lock a period.

## Scope

- Current report date(s): 2026-07-31
- Prior report date(s): 2026-06-30
- Material variance thresholds: $10000.00 and 10.00%
- Reconciliation tolerance: $0.01
- Exceptions: 8 total; 0 blocked; 8 requiring review.

## Exceptions

| Status | Control | Tenant | Account | Difference | Reason |
| --- | --- | --- | --- | ---: | --- |
| REVIEW | account_mapping | Acme Demo Pty Ltd | 6000 / Operating Expenses | n/a | Current account has no supplied review-group mapping. |
| REVIEW | financial_year_reset | n/a | n/a | n/a | Current ReportDate 2026-07-31 and prior ReportDate 2026-06-30 fall in different Australian financial years (1 July to 30 June). YTD figures reset on 1 July, so this YTD-vs-YTD comparison crosses a year reset and the period_variance verdicts for profit-and-loss-style rows are not meaningful. |
| REVIEW | period_variance | Acme Demo Pty Ltd | 1000 / Operating Bank | 15000.00 | YTD net balance moved beyond both configured materiality thresholds. |
| REVIEW | subledger_reconciliation | Acme Demo Pty Ltd | 2000 / Trade Creditors | -250.00 | Current trial-balance balance differs from the supplied subledger beyond tolerance. |
```

`exceptions.csv` carries the same exceptions with full numeric detail. The Operating Bank variance row (wrapped here for readability):

```csv
control,status,tenant,account_id,account_code,account_name,review_group,current_value,prior_value,difference,threshold,percentage_change,reason,reviewer_action
period_variance,REVIEW,Acme Demo Pty Ltd,100,1000,Operating Bank,Cash and cash equivalents,120000.00,105000.00,15000.00,10000.00,14.29%,
  YTD net balance moved beyond both configured materiality thresholds.,
  "Investigate the driver, retain supporting evidence, and document the reviewer conclusion."
```

The reviewer reads this as: the Operating Bank YTD balance moved from $105,000.00 to $120,000.00, a $15,000.00 (14.29%) change that clears both the $10,000.00 absolute threshold and the 10% threshold, so a human must investigate the driver and document a conclusion.

`close-review-pack.json` records the same exception as structured evidence next to the thresholds and the source digests (abridged):

```json
{
  "exceptions": [
    {
      "account_code": "1000",
      "account_name": "Operating Bank",
      "review_group": "Cash and cash equivalents",
      "control": "period_variance",
      "current_value": "120000.00",
      "prior_value": "105000.00",
      "difference": "15000.00",
      "percentage_change": "14.29%",
      "threshold": "10000.00",
      "status": "REVIEW",
      "tenant": "Acme Demo Pty Ltd"
    }
  ],
  "overall_status": "REVIEW",
  "thresholds": {
    "absolute_variance": "10000.00",
    "percentage_variance": "10.00%",
    "reconciliation_tolerance": "0.01"
  }
}
```

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

`reviewed_on` must not be earlier than the current `ReportDate`. A note dated before the period it claims to review is rejected as a malformed input: the run stops with exit `1` and writes no pack.

An acknowledgement is evidence of a human action only. It **never** changes `REVIEW` or `BLOCKED` to `PASS`, and it never asserts that a period has been closed.

## Design and integrity

A close can be technically balanced and still need review. This tool keeps the evidence visible:

- Exact `Decimal` arithmetic for money controls, never binary floating point.
- Schema, duplicate-key, date, and numeric gates fail closed.
- Current-period and YTD debits must exactly equal credits.
- Material YTD variances, new/missing accounts, account metadata changes, unmapped accounts, and supplied subledger differences become explicit exceptions.
- A YTD variance is raised only when it clears both the absolute and the percentage threshold, with one carve-out: an account whose prior YTD balance is nil has no percentage change to compute, so the absolute threshold decides alone. Those exceptions name the absolute threshold only and leave `percentage_change` blank, rather than reporting that a percentage test passed that never ran.
- Output has only `PASS`, `REVIEW`, and `BLOCKED` states. A reviewer, not the tool, decides whether a close is acceptable.
- Source SHA-256 digests travel with the generated review pack so its source files can be identified later. Each digest is calculated from the same immutable byte snapshot the loader parses, so a file replaced during a run cannot be misidentified as the source of the calculations.
- Spreadsheet-facing source text whose first non-whitespace character is `=`, `+`, `-` or `@` is neutralised with a leading apostrophe. This includes identifier- and number-shaped text such as `+unsafe`, `@123` and `-1000`; the guard does not try to decide which formula-looking values a particular spreadsheet may evaluate. Every exception table cell rendered into `close-summary.md` is flattened onto one line, and its backslashes are escaped before its pipes so that neither a pipe nor a backslash shielding one can add a cell and shift the columns a reviewer reads. A reviewer-note comment keeps its line breaks: a multi-line comment renders as an indented blockquote under the acknowledgement item, with each line escaped the same way and a leading `#` escaped so quoted text cannot forge a document heading.
- `exceptions.csv` is written with a UTF-8 byte-order mark, matching the canonical input files, so a spreadsheet reads non-ASCII entity and account names correctly.
- The three pack files are staged beside their destinations and moved into place only once all three have been written. If one cannot be replaced (a reviewer holding `exceptions.csv` open is the usual cause), the files already moved are rolled back to the content they replaced, so the previous pack survives whole instead of half describing one trial balance and half describing another. A failed run never deletes a pack file it did not write. Run one export at a time into a given `--output` directory; concurrent runs are not serialised.
- Amounts are rendered with at least two decimal places and never fewer than the value carries. A percentage is rendered with at least two places and always enough to show its leading significant digit, so neither a tolerance finer than one cent nor a threshold finer than a hundredth of a per cent is flattened to `0.00`.

### What formula neutralisation covers, exactly

The escaping in `exceptions.csv` applies to the five source-controlled text fields: `tenant`, `account_id`, `account_code`, `account_name`, and `review_group`. For those fields:

Neutralised (prefixed with an apostrophe so a spreadsheet reads them as text):

- Any value whose first non-whitespace character is `=`, `+`, `-` or `@`. For example, `=SUM(A1)` becomes `'=SUM(A1)`, `@123` becomes `'@123`, and a leading tab cannot bypass the check.
- Identifier- and number-shaped source text receives the same treatment. Account codes such as `-1000` remain visually recognisable in a spreadsheet but are explicitly stored as text.

Passed through unchanged:

- Values that do not start with a formula trigger after leading whitespace.
- Values already prefixed with an apostrophe; the guard does not add a second one.
- Numeric fields rendered by the tool (`current_value`, `difference`, thresholds and percentages) do not pass through this text guard and retain their ordinary numeric representation.

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

## Related

The next layers exist as separate repositories. This project stays a local review-pack generator; it does not grow a Xero client or a tax-advice engine.

- [xero-ai-review-gateway](https://github.com/ryanduguid/xero-ai-review-gateway) - a fixed-policy, synthetic-data review boundary for AI-assisted trial-balance analysis. No OAuth, no mutation tools.
- [Tax Radar AU](https://github.com/ryanduguid/tax-radar-au) - a provenance-first monitor that turns source-version metadata into a technical-review queue.

[examples/close-loop.md](examples/close-loop.md) runs close-control and the sibling gateway on local files: `close-control` reviews the gateway's May/June sample CSVs, then the gateway evaluates its own bundled context.

## Roadmap

The next layers are deliberately separated from the control engine; they already live in the sibling repositories above. The close-loop example runs the gateway on local files only.

See [docs/follow-on-safety-layers.md](docs/follow-on-safety-layers.md) for the intended boundary contracts.


MIT licensed.
