# Workpaper Review Gate

```
+----------------------------------------------------------------------+
|                        Workpaper Review Gate                         |
+----------------------------------------------------------------------+
|     Stop incomplete workpapers reaching manager review               |
+----------------------------------+-----------------------------------+
| DR  what it gives you            | CR  what it needs                 |
+----------------------------------+-----------------------------------+
| READY / NOT_READY / BLOCKED      | a pack directory of artefacts     |
| cover sheet for the reviewer     | a self-review JSON from the prep  |
| repeat-finding flags             | -                                 |
+----------------------------------+-----------------------------------+
```

[![tests](https://github.com/ryanduguid/workpaper-review-gate/actions/workflows/ci.yml/badge.svg)](https://github.com/ryanduguid/workpaper-review-gate/actions/workflows/ci.yml)
[![Python 3.10+](https://img.shields.io/badge/python-3.10%2B-5C2D91.svg?logo=python&logoColor=white&labelColor=04001F)](https://www.python.org/downloads/)
[![License: MIT](https://img.shields.io/badge/License-MIT-4F485E.svg?labelColor=04001F)](LICENSE)

[Browser evaluation](https://duguid.com.au/evaluate/manager-review-gate/) · [Reproduce locally](evaluation/manager_review_gate/README.md) · [Quick demo](#quick-demo) · [Release notes](RELEASE_NOTES.md)

The repository is Workpaper Review Gate (`workpaper-review-gate`); the `review-ready-gate` distribution, `review-ready` command and `reviewready` import remain compatibility identifiers.

A local, **review-first readiness gate** for Australian public-practice packs. You point it at a folder of workpapers from a junior, an offshore team, or an AI agent. It tells you whether that folder is allowed to enter manager review.

A public evaluation pack reproduces the v0.1.1 manager-review result on fabricated BAS fixtures. It is a local review aid, not an approval system.

### Fabricated proof

| Pack | Result | What the gate reports |
| --- | --- | --- |
| `examples/bas-ready` | `READY` | No configured findings. A human still decides whether the pack may proceed. |
| `examples/bas-not-ready` | `NOT_READY` | Missing GST control export, incomplete self-review and a blocking open item. |

Open the [manager-facing browser evaluation](https://duguid.com.au/evaluate/manager-review-gate/) or reproduce the [versioned evaluation pack](evaluation/manager_review_gate/README.md) from the fabricated fixtures.

It is the missing upstream step in this stack:

```text
Incomplete pack
      |
      v
workpaper-review-gate   <-- this repository
      |
      |  READY
      v
Manager review (judgement, risk, client impact)
      |
      v
Monthly Close Controls / payday-super-checker / other engines
```

[Monthly Close Controls](https://github.com/ryanduguid/monthly-close-controls) answers "what material exceptions exist on these trial balances?". This tool answers a prior question: **"is the pack even allowed onto the review desk?"** A file can have material variances and still be READY, because the variances are documented. A file with a missing GST control export is NOT_READY even if the numbers look tidy.

It does **not** connect to Xero, store OAuth tokens, write journals, lodge BAS, lock a period, call an LLM, or claim that a file is correct.

> [!WARNING]
> **Not tax advice.** A `READY` result means no configured gate tripped on the files that were present. A human still decides. See [DISCLAIMER.md](DISCLAIMER.md).

## Why this exists

Preparation got cheaper. Review did not. Software, offshore capacity, and AI all increase the number of files that hit the review desk. The scarce resource in a firm is the manager who can actually sign.

Most of what that manager then does is not judgement. It is reconstructing a pack that was never review-ready: missing tie-outs, open questions in email, a trial balance that does not balance, the same finding as last period. This gate keeps that work off the review desk.

## Quick demo

The repository contains fabricated data only. Do not commit client workpapers.

For the manager-facing, reproducible BAS evaluation, see the [manager review gate evaluation pack](evaluation/manager_review_gate/README.md).

[`examples/`](examples/README.md) is the assault course: every move the tool has, run against
fabricated data, with nothing at stake. Learn the flags here before pointing
it at a real pack.

```bash
python -m pip install -e ".[dev]"

review-ready gate \
  --profile bas \
  --pack examples/bas-ready \
  --output outputs/bas-ready
```

The ready demo exits `0` and writes three files:

- `readiness-summary.md`: cover sheet a manager reads top to bottom
- `findings.csv`: one row per finding, for Excel or Power BI
- `readiness-pack.json`: structured evidence, source hashes, and any supplied acknowledgement

```bash
review-ready gate \
  --profile bas \
  --pack examples/bas-not-ready \
  --output outputs/bas-not-ready
```

The not-ready demo exits `2`. The GST control export is missing, the preparer has not certified the pack, a blocking open item is still OPEN, and the same missing artefact was OPEN on the prior pack, so it is flagged as a repeat.

```bash
review-ready view --pack-dir outputs/bas-ready
```

Use exit code `0` only for `READY`, `2` for `NOT_READY` or `BLOCKED`, and `1` for a malformed file, an invalid command, or an `--output` path that cannot be written.

To run the gate on a schedule in CI, copy [examples/github-actions-readiness-check.yml](examples/github-actions-readiness-check.yml) into `.github/workflows/`.
It runs against a repo-stored synthetic pack, fails the job when the pack is `BLOCKED`, and reports `NOT_READY` for a human. A `READY` result is still not an approval.

## What gets gated

| Profile | Required artefacts | Extra controls |
| --- | --- | --- |
| `bas` | trial balance, activity statement, GST control GL, open items, self-review | 1A less 1B ties to GST control movement |
| `month_end` | current TB, prior TB, open items, self-review | optional bank rec; prior date earlier; same tenant |
| `year_end` | current TB, prior TB, tie-out matrix, open items, self-review | no `UNSUPPORTED` statement lines |

Optional in every profile: `prior_findings.csv`. An OPEN prior finding that is still present is marked `repeat`.

Filenames inside the pack directory are fixed. Header-only CSV schemas live under `schemas/`, together with the JSON Schema for `self_review.json`.

### Self-review

`self_review.json` is part of the pack, not a courtesy. Exact keys, exact assertion names, JSON booleans only:

```json
{
  "preparer_initials": "AB",
  "prepared_on": "2026-04-10",
  "engagement_type": "bas",
  "period_end": "2026-03-31",
  "assertions": {
    "pack_complete": true,
    "tie_outs_done": true,
    "open_items_listed": true,
    "variances_explained": true,
    "self_reviewed": true
  }
}
```

`engagement_type` must match `--profile`. `prepared_on` must not be earlier than `period_end`. Any assertion that is not `true` is `NOT_READY`. The assertions are necessary, not sufficient: a preparer who ticks `pack_complete` while the GST control file is missing still gets `MISSING_ARTEFACT`. The JSON Schema is [schemas/self_review.json](schemas/self_review.json).

### Open items

```text
ItemID,Severity,Owner,DueDate,Status,Description,Resolution
```

`Severity` is `BLOCKING`, `EXPLAIN`, or `TRIVIAL`. `Status` is `OPEN` or `CLEARED`. A `BLOCKING` item that is still `OPEN` with no resolution blocks the gate. A `CLEARED` item with no resolution text also fails: cleared means someone wrote down what happened.

### Trial balance

The ten-column canonical CSV from [xero-trial-balance-export](https://github.com/ryanduguid/xero-trial-balance-export):

```text
ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit
```

One tenant, one report date, unique `Tenant`+`AccountID`. Movement debit must equal movement credit, and YTD debit must equal YTD credit, or the pack is `BLOCKED`. An unbalanced ledger is not a review topic. It is a reason not to start.

### BAS tie-out

If both the activity statement and the GST control GL are present:

- labels `1A` and `1B` are required
- `1A - 1B` is compared with GST-control `sum(Credit) - sum(Debit)`
- a difference beyond `--tieout-tolerance` (default `$0.01`) is `NOT_READY`

This is a cash-style control-account tie-out on the files you supply. It is not a lodgment, not a cash-versus-accruals bridge, and not a substitute for the `bas-preparation` skill.

## Human acknowledgement

Optional `--review-note` JSON:

```json
{
  "reviewer_initials": "RD",
  "reviewed_on": "2026-04-12",
  "comment": "Reviewed fabricated demo findings only; no client file was approved by this example."
}
```

`reviewed_on` must not be earlier than `period_end`. An acknowledgement is evidence of a human action only. It **never** changes `NOT_READY` or `BLOCKED` to `READY`.

## Viewing an existing pack

`review-ready view` is the read-only half of the gate: it loads a generated pack, proves the three files still agree with each other, and prints the cover sheet. It never writes, renames or deletes anything, and it cannot change what the engine computed.

```bash
review-ready view --pack-dir outputs/bas-ready
```

Before displaying anything it fails closed on: a missing artefact; JSON that is not valid UTF-8, not valid JSON, or carries unknown, missing or duplicated top-level members; a threshold or nested source digest that no longer parses as the writer rendered it; a `readiness-summary.md` whose overall status, source-evidence digests or review-boundary statement disagree with the JSON (including a second, conflicting status line); and a `findings.csv` whose header, row count or any cell disagrees with the JSON findings, honouring the writer's formula-injection guard exactly. On success the sheet ends with the SHA-256 of each artefact's exact bytes, so the displayed evidence can itself be archived. Exit code is 0 when a pack was verified and shown, 1 when verification failed.

## Design

- Exact `Decimal` arithmetic for money, never binary floating point.
- Schema, duplicate-key, date, and numeric gates fail closed: a malformed file is exit `1` and writes no pack.
- Missing or empty required artefacts are findings, not crashes, so the cover sheet can tell the preparer what to send back.
- Source SHA-256 digests travel with the pack. Each digest is taken from the same immutable byte snapshot the loader parsed.
- Spreadsheet-facing finding text whose first non-whitespace character is `=`, `+`, `-` or `@` is prefixed with an apostrophe.
- The three pack files are staged beside their destinations and moved into place only once all three have been written. A failed run does not leave two runs mixed together.
- No wall-clock timestamps in the pack.

## Data boundary

- Use a separate, access-controlled working directory for client source files and outputs.
- Keep this checkout limited to fabricated fixtures. Its `.gitignore` blocks CSVs outside `examples/` and `schemas/`, and blocks all three generated pack files by name wherever `--output` points them, including inside those two fixture directories.
- Do not use this as tax, financial, audit, or legal advice.

## Related

- [Monthly Close Controls](https://github.com/ryanduguid/monthly-close-controls) - exception pack once a trial balance is allowed onto the review desk
- [Xero Ledger Review Gate](https://github.com/ryanduguid/xero-ledger-review-gate) - zero-network variance boundary for AI-assisted TB review
- [australian-accounting-skills](https://github.com/ryanduguid/australian-accounting-skills) - `workpaper-tie-out` and `bas-preparation` workflows this gate enforces mechanically
- [DrDebits](https://github.com/ryanduguid/DrDebits) - APES 110 / TPB guardrails for any LLM sitting *after* a READY pack

## Development

Use the locked toolchain. `python -m pytest` is not what CI runs.

```bash
uv lock --check
uv sync --locked --all-extras
uv run pytest
uv run ruff check reviewready tests
uv run mypy reviewready
uv build
```

The test suite covers schema gates, the three fabricated engagement packs, empty and incomplete artefacts, GST and bank-rec breaks, unsupported tie-outs, acknowledgement parsing, deterministic pack generation, fail-closed pack viewing, and the command-line exit contract.

Continuous integration verifies the committed `uv.lock`, runs the test suite on Python 3.10, 3.11, 3.12, and 3.13, then builds and smoke-tests the wheel with the fabricated demo. CodeQL scans the Python source, and Dependabot is configured to propose updates for `uv` dependencies and pinned GitHub Actions. See [CONTRIBUTING.md](CONTRIBUTING.md) for the local verification and data-handling requirements. To cut a release, follow [RELEASING.md](RELEASING.md). Do not tag until you intend to publish.

MIT licensed. Boundary statement: [DISCLAIMER.md](DISCLAIMER.md).
