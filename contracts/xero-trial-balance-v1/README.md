# Xero trial balance CSV contract v1

This directory is the data-only authority for the exporter-owned `xero-tb-csv.v1` corpus. The three fixture CSVs and `expected_results.json` were copied byte for byte from `packages/xero-trial-balance-export/evaluation/xero_tb_integrity/` at exporter source commit `2a0966e89e5f8daa587be8466f988d9adc16003a`. The corpus was already present at the exporter provenance commit `f87b5e4e224b930b3f6d9c9c43e365a9d4ea98d4` that the ledger-review gate vendored; consolidation changes the ownership location, not bytes or outcomes.

The ordered header is exactly:

```text
ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit
```

`Debit` and `Credit` are the movement during the report period. `YTDDebit` and `YTDCredit` are the as-at year-to-date trial balance. Both pairs must balance independently using exact decimal arithmetic; no tolerance and no binary floating point may mask a nonzero difference. `Tenant` plus the stable Xero report-row `AccountID` is the control identity. `AccountCode` is presentation metadata (it may carry a leading zero such as `090` and must stay text) and is not a substitute join key; `AccountName` is a display attribute.

All fixtures are fabricated, UTF-8, comma-delimited and LF-only. `passing.csv` must be accepted. `failing_movement.csv` changes only the movement pair (debits 1200.00, credits 1199.99, difference 0.01) and must be rejected. `failing_ytd.csv` changes only the YTD pair (debits 15234.50, credits 15234.49, difference 0.01) and must be rejected. Exact exit codes, output markers, byte digests, sources, the fixture version and the human-decision boundary are recorded in `expected_results.json`; `SHA256SUMS` makes the root copy independently reproducible.

A balanced export passes this integrity control only; a human still decides completeness, classification, accounting treatment and fitness for review.

This contract has no runtime library. The exporter is the producer; the readiness gate, monthly close, the ledger-review gate, the Excel adapter and the Power BI application consume these files only in tests, conformance checks or local import flows. Review components remain offline and must not import exporter code, OAuth helpers, Xero clients or sibling runtime packages.
