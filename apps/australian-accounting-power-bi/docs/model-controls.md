# Model controls and acceptance

The financial tables read six local, fabricated CSVs with unchanged headers. The separate review tables read three projection CSVs, making nine sources in total. The application has no OAuth, HTTP client or connection to Xero. The shared trial-balance contract tests still establish fixture compatibility only; a trial balance cannot be substituted for journal lines.

## Refresh checks

Refresh rejects conversion errors, missing required values and duplicate dimension or fact keys. The ledger key is JournalID plus LineNumber. A journal balances across all its lines and uses one posting date. It may span entities.

Each ledger Amount must equal Debit less Credit. Debits and credits must be non-negative and cannot both be positive on one line. Entity, account and intercompany counterparty keys must exist. Ledger dates, budget periods and pay dates must lie inside the model horizon, 1 July 2024 to 30 June 2027. Budgets use the first day of each month. Unknown industries and account classifications are refused.

The chart uses these class and subclass combinations. Normal balance remains a separate attribute, so a contra-asset may have a credit balance.

| Class | Subclasses |
| --- | --- |
| Asset | Current Assets, Non-Current Assets |
| Liability | Current Liabilities, Non-Current Liabilities |
| Equity | Equity |
| Revenue | Operating Revenue, Intercompany Revenue |
| Expense | Cost of Sales, Operating Expenses |

Budget rows use Revenue or Expense accounts. Every entity needs a revenue budget for each month in the model horizon. Multiple revenue accounts and zero budget amounts are allowed. Expense budgets are deliberately partial.

Intercompany lines require a named, different entity as counterparty. Refresh groups each pair and posting date separately for balance-sheet and profit-and-loss legs. Both counterparties must appear and each leg must balance. This catches a missing counterparty even when every remaining journal balances. It is a daily aggregate control, so two offsetting errors inside the same pair and leg can still cancel. Transaction matching would require another source identifier.

The entity master accepts AUD and an ownership weight of one. The report does not implement foreign exchange, minority ownership or a complete statutory consolidation process.

## Calculations and report scope

Gross figures respect the selected entities. Net and elimination figures require the complete group and a supported financial measure. Unsupported budgets, payroll or reference measures are blank under those calculation items. Monetary measures reconcile as gross less eliminations equals net. Net ratios are recalculated from net inputs.

Total expenses include cost of sales and operating expenses. The fabricated references use that same definition. Ratios are blank when the denominator is absent or zero. Revenue budget variance is actual less budget; the report avoids a profit-budget comparison because the sample expense budgets are incomplete.

Each page has its own entity and financial-year selectors. Financial pages start in FY25-26; payroll starts in FY26-27 with late Payday sample receipts selected. Clearing the year includes all loaded years, including fabricated future dates. The balance page starts with all three bases available. Selecting only part of the group leaves elimination and net values blank.

## Payroll assumptions

The source records liability and dates but no amount actually received. Every sample assumes full receipt before assessment, a nominated fund and no exceptional deadline. Missing receipts and inconsistent statuses are refused. Partial or split payments, assessment history and statutory exceptions need a separate assessment.

The report distinguishes quarterly and Payday samples. Its on-time rate describes the selected recorded receipts and intersects the status filter. It does not certify compliance. Projected GIC rates and part-day holiday review appear in EstimateBasis. Charge and interest totals are withheld when selected records require calendar review. See the dated sources and assumptions in [compliance methodology](compliance-methodology.md).

Quarterly and on-time rows must carry zero supplied charge and interest estimates. Positive estimates are supported for late Payday sample rows under the documented assumptions.

Employee dimension attributes come from the latest pay date, with EventID breaking ties. Historical employer and fund values remain on the event rows; the current dimension must not be used to infer past attributes.

## Repeatable native checks

Run the structural checks in [CONTRIBUTING.md](../CONTRIBUTING.md). Native checks require Power BI Desktop and a refreshed disposable copy of the fabricated project. Set that copy's SampleFolder to its own samples directory. Use the localhost port of that copy's Analysis Services process.

```powershell
powershell -NoProfile -File tools/test_financial_filters.ps1 -Server localhost:<port>
powershell -NoProfile -File tools/test_benchmark_measures.ps1 -Server localhost:<port>
powershell -NoProfile -File tools/test_review_controls.ps1 -Server localhost:<port>
```

The first two scripts cover financial filters and benchmark boundaries. The review-control script checks independent CSV amounts for all three reporting bases, unsupported combinations, ratios, budgets, time-group composition and payroll selections.

For a recorded disposable instance, run the same suites and query timings together:

```powershell
pwsh -NoProfile -File tools/test_native_project.ps1 -InstanceFile '<instance.json>' -SourceManifest '<source-manifest.json>' -EvidenceDirectory '<evidence-folder>'
```

The instance JSON records `server` (`localhost:<port>`), `project` (the disposable project folder), `desktop_pid`, `engine_pid`, `started_at` and `engine_started_at` (the recorded process creation timestamps). The manifest records `revision` and `source.file_inventory`, whose entries contain relative `path` and lowercase SHA-256 `source_sha256` fields for the project descriptors, report, model and nine CSVs. Capture these records when preparing the copy, before changing its SampleFolder parameter. `-SourceRoot` defaults to this application; `-PowerBIBin` defaults to the installed Desktop folder.

The runner compares the loaded model and copied CSVs with that source, checks the process identities and listener, and retains each suite's output. It checks the binding again after the suites. It does not refresh or modify the model. An incorrect manifest or stale instance record fails the run. Keyboard interaction, visual states and screen-reader acceptance still need separate evidence.

The rejection gate deliberately corrupts copied CSVs, restores their original bytes after each case and refreshes the restored inputs at the end. It refuses the committed samples folder and requires a marker in the disposable folder:

```powershell
New-Item -ItemType File -Path '<disposable-samples>/.native-test-copy'
powershell -NoProfile -File tools/test_refresh_guards.ps1 -Server localhost:<port> -SampleFolder '<disposable-samples>'
```

It tests offsetting amount corruption, duplicates, missing values, unknown keys, an out-of-range date, missing balanced counterparties, unsupported ownership and currency, invalid payroll receipts, account classifications and benchmark ranges. Never point it at business data.

Inspect all six pages after refresh, including the hidden supporting-journal page reached by drill-through. Exercise entity and year selections, clear selections and check keyboard navigation. Structural validation alone cannot establish visual or assistive-technology behaviour. Service permissions, gateway refresh, external-data reconciliation and performance at scale remain separate acceptance work.
