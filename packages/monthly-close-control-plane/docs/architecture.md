# Architecture and control boundary

## The first release boundary

Monthly Close Controls is a local, deterministic **review-pack generator**. It deliberately accepts an already validated canonical trial-balance export rather than connecting to a ledger.

```text
Read-only validated source export
             |
             v
Canonical CSV loader ---- schema / date / Decimal / duplicate-key gates
             |
             v
Control engine ---------- TB integrity / variance / mapping / reconciliation
             |
             v
Review-pack writer ------ Markdown / CSV / JSON evidence
             |
             v
Human reviewer ---------- investigate, document, decide
```

The application cannot cross the final boundary. It has no code to create a journal, post a transaction, make a payment, lodge a return, lock a period, email a report, call an accounting API, or declare approval.

## Local workbench façade

`close-control workbench` is a second local entry point to the same loader,
control engine, source/output collision guard, and three-file review-pack
writer as `close-control review`. It accepts the same already-created
canonical CSV inputs and returns the same status and exit codes; it adds only a
concise reviewer handoff to the console.

```text
Human-operated read-only export, outside repository
             |
             v
close-control workbench --- existing review_close() / write_review_pack()
             |
             v
close-summary.md + exceptions.csv + close-review-pack.json
             |
             v
Human reviewer opens or imports the exception detail
```

The façade does not launch Excel, create a workbook, import VBA, run Power
Query, access Xero or OAuth tokens, call a model or AI gateway, copy inputs, or
write to an accounting system. It is not a new data contract or a workflow
approval state: the three existing local artefacts remain the only outputs.

## Source contract

The canonical source has the same normalised contract as the companion Xero trial-balance exporter. `Tenant + AccountID` is the stable key because account codes and display names can change. The loader rejects unknown columns rather than silently accepting a different report shape.

Money is parsed into `Decimal`, not `float`. The integrity controls compare debit and credit totals exactly; they do not round a mismatch away.

## Controls

| Control | Output when it fails | Why it matters |
| --- | --- | --- |
| Canonical schema, ISO date, text key and Decimal parse | Input error; no pack | A close pack must not reinterpret a changed export. |
| Duplicate `(Tenant, AccountID)` | Input error; no pack | A duplicate could double-count the same account. |
| Current and prior debit/credit equality | `BLOCKED` | Variance analysis cannot make an unbalanced TB trustworthy. |
| Report dates in different Australian financial years | `REVIEW` | YTD figures reset on 1 July, so a comparison across the reset puts a full year against a month and its variance verdicts on P&L-style rows mean nothing. |
| Account movement | `REVIEW` | A high variance is an investigation trigger, not an error by itself. |
| New/missing account or metadata drift | `REVIEW` | Chart-of-accounts changes can be legitimate but must be explained. |
| Missing supplied mapping | `REVIEW` | Grouped review should not silently omit a new account. |
| Supplied subledger difference | `REVIEW` | An out-of-tolerance control-account difference needs supporting reconciliation. |

Account movement is raised only when a YTD variance clears both the absolute and the percentage threshold. The one carve-out is a nil prior YTD balance, which leaves no percentage change to compute: the absolute threshold decides alone, `percentage_change` carries the sentinel `n/a (prior period zero)`, and the exception reason names the absolute threshold only. The sentinel is used instead of a blank cell because a blank reads as "no change", while no consumer can read `n/a (prior period zero)` as a zero percentage.

## Deterministic evidence

The output has no wall-clock generated timestamp. It records source filenames only by role and SHA-256 digest, report dates, thresholds, ordered exceptions, and an optional review acknowledgement. Given identical inputs and options, the content is identical.

The acknowledgement is separate from calculation and never flips `REVIEW` or `BLOCKED` to `PASS`. This is intentional: a human can document a conclusion, but a program cannot represent professional approval.

## Data handling

The repository contains fabricated data. Its `.gitignore` blocks ordinary CSV files outside `examples/` and `schemas/`, the three generated review-pack files by name (`exceptions.csv` explicitly inside those two directories as well, since their fixture negations would otherwise re-include it), token-like files, and environment files. Client exports and review packs belong in an access-controlled local location outside the repository.
