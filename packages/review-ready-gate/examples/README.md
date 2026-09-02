# examples

The assault course: every move the tool has, run against fabricated data, with
nothing at stake. Learn the flags here before pointing it at a real pack.

Do not commit client workpapers. Every CSV and JSON in this directory is
synthetic. The entities are Cedar and Pine Consulting Pty Ltd (BAS) and
Varrock Ventures Pty Ltd (month-end and year-end).

| Directory | Profile | Expected status | What it demonstrates |
| --- | --- | --- | --- |
| `bas-ready/` | `bas` | `READY` | Complete pack: TB balances, 1A−1B ties to GST control, self-review all true, no blocking open items |
| `bas-not-ready/` | `bas` | `NOT_READY` | Missing GST control, incomplete self-review, blocking open item, prior finding still OPEN (repeat) |
| `bas-blocked/` | `bas` | `BLOCKED` | Unbalanced trial balance, not a review topic |
| `month-end-ready/` | `month_end` | `READY` | Current and prior TB, same tenant, prior date earlier, bank rec within tolerance |
| `year-end-ready/` | `year_end` | `READY` | Tie-out matrix with no `UNSUPPORTED` lines; TIED rows name workpaper and source |

`review_note.json` is an optional acknowledgement. Supplying it never changes
status.

```bash
review-ready gate --profile bas --pack examples/bas-ready --output outputs/bas-ready
review-ready gate --profile bas --pack examples/bas-not-ready --output outputs/bas-not-ready
review-ready gate --profile bas --pack examples/bas-blocked --output outputs/bas-blocked
review-ready gate --profile month_end --pack examples/month-end-ready --output outputs/month-end-ready
review-ready gate --profile year_end --pack examples/year-end-ready --output outputs/year-end-ready
review-ready view --pack-dir outputs/bas-ready
```

To run the gate on a schedule in CI, copy
[github-actions-readiness-check.yml](github-actions-readiness-check.yml) into
`.github/workflows/`. It fails the job when the pack is `BLOCKED`, and reports
`NOT_READY` for a human. A `READY` result is still not an approval.
