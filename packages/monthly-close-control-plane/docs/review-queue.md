# Local close review queue

Use `close-control queue` to inspect several existing close packs together, with filters for report date, recorded pack state and recorded reviewer.

```bash
close-control queue \
  --pack-dir C:/close-data/july-pack \
  --pack-dir C:/close-data/august-pack \
  --status REVIEW
```

Repeat `--pack-dir` for each directory you intend to read. The command reads only those packs, deduplicates paths and sorts the display by the recorded current report dates and path. It does not discover neighbouring directories.

Add `--period 2026-07-31` to match an exact current report date, or `--reviewer RD` to match the exact initials in a human acknowledgement. Filters combine, and reviewer matching is case sensitive. An acknowledgement records a past review action; it is not a preparer assignment or approval. The pack schema records neither owners nor due dates, so the queue cannot filter by them.

## Verification and coverage

Every supplied pack passes the same `viewer.verify_pack` checks as `close-control view` before any queue is displayed. An invalid or missing pack fails the command even when a filter would exclude it. No partial queue is printed.

After each verification, the queue retains display fields and artefact hashes rather than the full exception and query payloads. Each pack still needs memory for its verification; adding packs does not retain their full documents together.

The table retains the pack's `PASS`, `REVIEW` or `BLOCKED` state, exception and draft-query counts, recorded reviewer, and omitted controls. When an older pack lacks control coverage or the query register, the queue displays `not recorded`. A missing field does not establish that no control or query was needed. The display includes the SHA-256 of each verified artefact.

Supplied text uses JSON character escapes and escaped table separators in the display, including for non-ASCII characters. These escapes do not change the original evidence files or the values used by filters.

Exit `0` means the supplied packs verified and the display completed, including when they have `REVIEW` or `BLOCKED` states or no packs match the filters. Exit `1` means invalid arguments or failed verification. These exits describe display, not accounting approval.

The queue writes no files and changes no findings, acknowledgement, accounting result or source. Keep real packs in the approved location outside the checkout.

## Source pattern

The [LodgeiT dashboard guide](https://help.lodgeit.net.au/support/solutions/articles/60000609258-dashboard-work-status-filter-and-sort-team-form-period-client-group-), inspected on 9 October 2026, illustrates filters for work, period and people. This implementation uses only metadata already present in this component's verified packs. It adds no LodgeiT integration or shared pipeline status.
