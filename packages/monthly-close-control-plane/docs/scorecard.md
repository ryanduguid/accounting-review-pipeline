# Close scorecard

`close-control scorecard` reads one client's saved results from `jet`,
`duplicates`, `coding`, `fixed-assets` and `schedule` and summarises them by
category, so a reviewer can see which areas are clear and which need work. It
reads local files, writes JSON to standard output and changes nothing. It
exits `0` when every result is at `PASS`, `2` otherwise and `1` for an unusable
result or option.

```text
close-control coding --lines lines.csv --currency AUD --period-start 2026-09-01 --period-end 2026-09-30 > coding.json
close-control fixed-assets --register register.csv --trial-balance tb.csv --currency AUD > fixed-assets.json
close-control scorecard --result coding.json --result fixed-assets.json
```

## What it needs

Each `--result` is a JSON result carrying `status` (`PASS` or `REVIEW`),
`tenant` and a `findings` array. Its category is its file name without the
extension, so 2 results may not share a file name. Every result must name the
same tenant; summarise each client separately.

## What it reports

For each category: its `status`, the number of `findings`, the count of each
finding `code` (or `test`, for `jet`; `(no code)` when a finding has neither)
and the result file's SHA-256. Overall: `results_supplied`, `results_at_pass`,
`findings_total` and the score:

```text
score_percent = floor(100 * results at PASS / results supplied)
```

The score counts only the results supplied: a check that was not run cannot
lower it, and a result at `PASS` shows only what that check tested. The score weights
every category equally and does not weigh findings by amount.
