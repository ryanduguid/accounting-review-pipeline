# Native Desktop verification, 8 September 2026

Power BI Desktop 2.157.1354.0 on Windows opened the corrected project, refreshed the fabricated samples, and rendered all four pages and 21 visuals without visual error placeholders. This check used the changes accompanying this record, based on Accounting Review Pipeline commit `a42fd75ef1f70c58dd54a55e9cd5dc1e2359e072`.

## Defects found in Desktop

- Opening failed because six named M expressions duplicated table query names. Prefixing those expressions with `Source_` and updating their partition references removes the collisions.
- Opening then failed because calculation groups require `discourageImplicitMeasures`. The model now declares that flag.
- Refresh failed because `File.Contents` requires absolute paths. All six CSV imports now use one required `SampleFolder` text parameter. Its committed default is blank; configure it for each checkout.
- The benchmark risk card clipped its text. Its width now uses the spare space beside it, and the variance card moves to the right.

Existing tests now reject expression/table name collisions, the missing calculation-group flag, and CSV imports that omit the folder parameter.

## Native procedure and observations

The native check used an isolated copy of the PBIP, report, semantic model and committed `samples/` folder. Only the copy's `SampleFolder` value contained an absolute local path. Desktop caches and screenshots remain outside the repository.

Open the PBIP, set `SampleFolder`, and run **Home > Refresh > Schema and data**. Inspect each page after refresh completes. The observations below describe the default page state, before selecting entity or period filters.

| Page | Visuals inspected | Observed result |
| --- | --- | --- |
| Executive Financial Performance | Title, four cards, P&L matrix, working-capital trend | Revenue $30.42 million; gross margin 66.8%; EBITDA $9.32 million; net assets $11.04 million. Matrix and trend populated. |
| Multi-Entity Consolidation & Eliminations | Title, entity matrix, transaction table | Entity totals and fabricated ledger rows rendered. |
| ATO Benchmark & Practice Diagnostic | Title, two cards, ratio matrix, scatter chart | Risk text and gross-profit variance rendered; variance was -6.6%. Four entities appeared in the matrix and scatter chart. |
| Payday Super & STP Compliance Monitor | Title, four cards, payroll table | Compliance 94.9%; SGC exposure $13.48; nominal interest $8.42; 16 late events. Payroll rows rendered. |

Wide matrices and tables use horizontal scrollbars. This check covers opening, local sample refresh and visible rendering. It does not validate every DAX result, filter combination, statutory assumption, production data source or Power BI Service deployment.

## Automated checks

All checks passed from the component directory:

```text
python -B -m unittest discover -s tests -v
npx --yes @microsoft/powerbi-report-authoring-cli@0.1.4 validate australian-accounting-power-bi.Report
python -m ruff check .
python -m mypy
```

The suite ran 43 tests. The Microsoft validator returned zero errors and zero warnings. Ruff 0.16.6 passed, and Mypy 2.3.1 reported no issues in seven source files. Lint tools ran in isolated uv environments using the workflow's pinned versions.

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
each of four entities, a two-entity selection, June and July across the financial
year boundary, a July group selection, September FYTD through the calculation
group, and a two-year selection. Four additional cases select financial years
through `FinancialYearNumber` and `FinancialYear`, and June/July through
`FinancialYearMonth`. Their CSV expectations still use independent date ranges.
Each case checks the filtered row count and eleven financial measures, including
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
