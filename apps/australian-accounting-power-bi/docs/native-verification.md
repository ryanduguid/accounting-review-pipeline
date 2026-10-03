# Native Desktop verification, 8 September 2026

Power BI Desktop 2.157.1354.0 on Windows opened the corrected project, refreshed the fabricated samples, and rendered all 4 pages and 21 visuals without visual error placeholders. This check used the changes accompanying this record, based on Accounting Review Pipeline commit `a42fd75ef1f70c58dd54a55e9cd5dc1e2359e072`.

This record predates the `KEEPFILTERS` change to the account category predicates described in [DAX patterns](dax-patterns.md). It describes the pages as they rendered before that change, so repeat the Desktop refresh, the page inspection and `tools/test_financial_filters.ps1` before relying on it again.

## Defects found in Desktop

- Opening failed because 6 named M expressions duplicated table query names. Prefixing those expressions with `Source_` and updating their partition references removes the collisions.
- Opening then failed because calculation groups require `discourageImplicitMeasures`. The model now declares that flag.
- Refresh failed because `File.Contents` requires absolute paths. All 6 CSV imports now use one required `SampleFolder` text parameter. Its committed default is blank; configure it for each checkout.
- The benchmark risk card clipped its text. Its width now uses the spare space beside it, and the variance card moves to the right.

Existing tests now reject expression/table name collisions, the missing calculation-group flag, and CSV imports that omit the folder parameter.

## Native procedure and observations

The native check used an isolated copy of the PBIP, report, semantic model and committed `samples/` folder. Only the copy's `SampleFolder` value contained an absolute local path. Desktop caches and screenshots remain outside the repository.

Open the PBIP, set `SampleFolder`, and run **Home > Refresh > Schema and data**. Inspect each page after refresh completes. The observations below describe the default page state, before selecting entity or period filters.

| Page | Visuals inspected | Observed result |
| --- | --- | --- |
| Executive Financial Performance | Title, 4 cards, P&L matrix, working-capital trend | Revenue $30.42 million; gross margin 66.8%; EBITDA $9.32 million; net assets $11.04 million. Matrix and trend populated. |
| Multi-Entity Consolidation & Eliminations | Title, entity matrix, transaction table | Entity totals and fabricated ledger rows rendered. |
| ATO Benchmark & Practice Diagnostic | Title, 2 cards, ratio matrix, scatter chart | Risk text and gross-profit variance rendered; variance was -6.6%. Four entities appeared in the matrix and scatter chart. |
| Payday Super & STP Compliance Monitor | Title, 4 cards, payroll table | Compliance 94.9%; SGC exposure $13.48; nominal interest $8.42; 16 late events. Payroll rows rendered. |

Wide matrices and tables use horizontal scrollbars. This check covers opening, local sample refresh and visible rendering. It does not validate every DAX result, filter combination, statutory assumption, production data source or Power BI Service deployment.

## Automated checks

All checks passed from the component directory:

```text
python -B -m unittest discover -s tests -v
npm ci --ignore-scripts
npx --no-install powerbi-report-author validate australian-accounting-power-bi.Report
python -m ruff check .
python -m mypy
```

The suite ran 43 tests. The Microsoft validator returned zero errors and zero warnings. Ruff 0.16.6 passed, and Mypy 2.3.1 reported no issues in 7 source files. Lint tools ran in isolated uv environments using the workflow's pinned versions.

## Entity and period checks, 9 September 2026

Power BI Desktop 2.157.1354.0 refreshed a disposable copy of the fabricated sample
model from repository commit `b5a42c7e86ba49a0bb29bd5dcb3d8879bb6a5515`.
The new check uses the installed ADOMD client to query the stored measures.
It calculates expectations separately from the sample CSVs using decimal
arithmetic. It neither copies the DAX expressions nor replaces production measures.

Run from this component directory, using the local port of that sample instance:

```powershell
powershell -NoProfile -File tools/test_financial_filters.ps1 -Server localhost:<port>
```

All 16 filter cases passed, with 192 assertions: FY2025 and FY2026 group totals,
each of 4 entities, a 2-entity selection, June and July across the financial
year boundary, a July group selection, September FYTD through the calculation
group, and a 2-year selection. Four additional cases select financial years
through `FinancialYearNumber` and `FinancialYear`, and June/July through
`FinancialYearMonth`. Their CSV expectations still use independent date ranges.
Each case checks the filtered row count and 11 financial measures, including
Working Capital and cumulative balance-sheet amounts.

| Selection | Revenue | EBITDA | Net assets |
| --- | ---: | ---: | ---: |
| Group FY2025 | 8,777,700.00 | 2,647,872.00 | 4,601,472.00 |
| Group FY2026 | 10,138,500.00 | 3,107,520.00 | 7,592,592.00 |
| ENT001 July 2025 | 204,800.00 | 90,180.00 | 1,543,420.00 |
| Group FYTD September 2025 | 2,407,050.00 | 733,788.00 | 5,306,160.00 |

As a negative control, a disposable copy of the check replaced Revenue with zero
in query scope. It failed the first case: native zero versus independent
8,777,700.00. The stored model and the checked-in script were not altered by
that control. The existing 13 benchmark cases also passed.

The expanded check ran after reopening the same refreshed sample project. A
Working Capital override of zero passed the original 132 assertions, then failed
the expanded check against independent 3,500,472.00. A separate negative control
selected financial year 2025 for the 2026 case and failed on 566 rows versus
expected 540. Both controls used disposable script copies; the stored model and
checked-in script retain the real measures and intended selections.

The native checks exercise filter contexts directly. They do not certify filter
card interaction, every visual, every calculation group combination, statutory
currency or production data. A blank aggregate is accepted only for an expected
zero category; row-count assertions still require a populated result. Numeric
differences greater than half a cent fail.

## Review corrections, 12 September 2026

Power BI Desktop 2.157.1354.0 refreshed a disposable copy of the corrected model
based on commit `b2f340845b422b0a822b1b60cedd39d119a9fe32`. All 4 pages rendered.
The native checks passed 192 financial assertions across 16 filter cases, 8
ABN cases and 9 fixed-decimal column checks.

A fabricated budget value of `TBC` still loaded as blank after removing
`returnErrorValuesAsNull`. Explicit error guards in all 6 CSV source expressions
then rejected that value. Restoring the sample CSV allowed refresh to complete;
the 192 financial assertions passed again. The committed `SampleFolder` remains
blank. No production data or Power BI Service deployment was used.

The final component suite passed 47 tests. The Microsoft report validator 0.1.4
returned zero errors and warnings; Ruff and Mypy passed.

## Refresh and report controls, 25 September 2026

Power BI Desktop 2.157.1354.0 loaded the local improvements over commit
`75c26508294bfd0cc5f339dba19e99afbe91fcec`. Tests used disposable copies of the
fabricated project. The committed `SampleFolder` remains blank.

An initial refresh waited on a privacy-level prompt behind the progress window.
The subsequent native refresh succeeded. All 4 pages and 45 visuals rendered
without error placeholders. Desktop inspection found clipped and repeated card
labels and excessive revenue rounding; the final report corrects them.

| Check | Result |
| --- | --- |
| Python 3.10.21, 3.12.10 and 3.13 | 64 tests passed on each interpreter |
| Ruff 0.16.6 and Mypy 2.3.1 | Passed; Mypy checked 9 files |
| Locked dependency installation and PBIR validator 0.1.4 | Installation succeeded; zero validation errors or warnings |
| `test_financial_filters.ps1` | 16 cases, 192 assertions passed |
| `test_benchmark_measures.ps1` | 13 checks passed |
| `test_review_controls.ps1` | 60 assertions passed |
| `test_refresh_guards.ps1` | 19 invalid inputs rejected; restored inputs refreshed successfully |
| PowerShell syntax and `git diff --check` | Passed |

The financial script's independent CSV expectations still apply after the
intercompany fixture split: existing journal identifiers and aggregate amounts
were preserved. The new review script covers all three reporting bases, unsupported
measure and entity combinations, total expenses, revenue budgets, time-group
composition and payroll filters. The rejection script tests the actual Power Query
refresh, including removal of an otherwise balanced counterparty journal.

Native interaction checks confirmed that selecting Draynor Fresh Foods changes
revenue to $4.24 million in FY25-26 and $3.64 million in FY24-25. Selecting a
consolidated basis for that single entity blanks the financial measures and shows
the complete-group instruction. Restoring all entities, FY25-26 and gross reporting
restores revenue of $10.14 million.

The default payroll selection shows 16 late receipts, an on-time share of 0%,
estimated SGC of $13.51 and estimated GIC earnings of $8.44. Clearing the receipt
status includes 14 calendar-review events and withholds both estimated totals.
The benchmark page shows Draynor's 39.6% gross margin and 84.8% total expense ratio.

Alt text appeared in the native accessibility tree, and selectors could be opened
with Enter. Tab traversal could not be established reliably through the automation
interface. Full keyboard traversal and screen-reader acceptance remain unverified.
This is not a WCAG conformance claim. No Power BI Service, gateway, production-data
reconciliation or large-data performance test was run.

## Dashboard reference pass and compatibility repair, 25 September 2026

The follow-up report has 4 pages, 43 visuals, 11 tables and 54 measures. The
[design record](report-design.md) explains the Microsoft and financial-dashboard
references, shared theme, native page navigation, combined cards, comparison
charts, selection context and readable receipt labels.

Desktop rejected the project when its saved local model was at compatibility
level 1606 and `database.tmdl` still requested 1600. The reported error was
`Tabular databases do not support CompatibilityLevel downgrade.` Changing the
definition to 1606 allowed all 11 tables to open. A regression assertion now
prevents the project definition from dropping below that level.

After a native save and close, the same project reopened without recopying its
definition. The local engine reported compatibility 1606, 11 tables and 54
measures. Full refresh and all 69 review-control assertions passed again.

Refresh then exposed a separate privacy-level error when combining the local
CSV queries. Each of the 6 fabricated sample sources was explicitly set to
Public in the current-file data source permissions. Privacy checks remained
enabled. Full refresh and the 19 invalid-input rejection cases then passed;
the restored inputs refreshed successfully. The README records these first-run
steps for fabricated data.

The final Python suite passed 64 tests on Python 3.10, 3.12 and 3.13. Ruff,
Mypy, PowerShell syntax and whitespace checks passed. PBIR validator 0.1.4
reported zero errors and warnings. Existing financial and benchmark checks
passed 192 assertions and 13 cases. The extended review-control script passed
69 assertions, including receipt labels, calendar and empty-selection notes,
discontinuous dates and text context under consolidation.

Native inspection found and corrected a vertical page navigator and an
overcrowded P&L matrix. All 4 revised pages rendered. Ctrl+click navigation,
entity and reporting-basis selections, readable receipt filters, complete-group
warnings and calendar-review withholding were exercised. The checked colour
pairs had contrast ratios of 5.36:1 or better, and alt text stayed within
250 characters. These are bounded checks, not a WCAG conformance assessment.

The local query timing script measured 10 warm requests after one initial
request for each query. Observed timings in milliseconds were:

| Query | First observed | Warm median | Warm p95 |
| --- | ---: | ---: | ---: |
| Financial summary | 30.32 | 4.96 | 5.68 |
| Monthly revenue and budget | 4.32 | 2.72 | 3.08 |
| Profit and loss comparison | 10.13 | 8.07 | 8.45 |
| Consolidated balances | 10.92 | 10.70 | 10.94 |
| Payroll review | 6.28 | 5.09 | 5.58 |

These timings include local engine round trips and row reads on the fabricated
dataset. Other read-only checks ran on the same machine. Cache state was not
cleared, so the first request is not a cold-cache measurement. Visual rendering,
service latency, concurrency and production-scale performance remain untested.
Full Tab traversal was again inconclusive through automation. Screen-reader
acceptance and mobile layouts remain unverified.
