# Benchmark verification

The benchmark measures compare one entity and financial year with one matching
turnover band in the fabricated sample. They do not estimate audit risk. The
legacy `ATO Compliance Risk Profile` identifier remains for report compatibility;
its displayed name is Gross profit comparison.

Annual turnover determines the band even when a month or account filter narrows
the displayed ratios. Turnover bands exclude their lower bound and include their
upper bound. Gross profit comparisons include both range endpoints. Missing,
overlapping or ambiguous selections return an explanatory status and blank
reference measures.

## Native regression check

Open the project in Power BI Desktop with the fabricated samples and refresh the
model. Run this command from the application directory, using that instance's
local Analysis Services port:

```powershell
powershell -NoProfile -File tools/test_benchmark_measures.ps1 -Server localhost:<port>
```

The script uses Power BI Desktop's installed ADOMD client. It reads the production
DAX expressions and evaluates them in the native engine with controlled inputs;
it does not change the model. The port must belong to the sample project instance.

The 13 cases cover both gross profit endpoints, values outside each endpoint,
the 1,000,000 turnover boundary and the next band, unmatched turnover, no
turnover, multiple entities, multiple financial years, a month filter and an
account filter, and a conflicting benchmark value filter. The original measures
failed the boundary and selection cases. The benchmark value filter also failed
before the reference lookups cleared benchmark-table filters. The corrected
measures passed all 13 on 8 September 2026. Full sample model
refresh also succeeded in Power BI Desktop.

All four pages rendered after the model change. The final benchmark page displayed
the revised card label, the selection prompt and both FinancialYear and TradingName
filter cards. A completed selection through the Desktop UI was not verified;
the native DAX cases verify entity and year filter contexts directly.

This check requires Windows and Power BI Desktop and is separate from the
repository's structural checks. It does not verify that sample benchmark values
are current ATO figures or replace inspection of all four report pages.
