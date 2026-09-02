# accounting-excel-toolkit

```
+----------------------------------------------------------------------+
|                       accounting-excel-toolkit                       |
+----------------------------------------------------------------------+
|       Power Query and VBA utilities for Australian ledger work       |
+----------------------------------+-----------------------------------+
| DR  what it gives you            | CR  what it needs                 |
+----------------------------------+-----------------------------------+
| Xero TB parser (Power Query)     | a Xero trial balance CSV          |
| ABN checksum validation          | Excel for Power Query or VBA      |
| workpaper formatting VBA         | -                                 |
+----------------------------------+-----------------------------------+
```

[![Verify](https://github.com/ryanduguid/accounting-excel-toolkit/actions/workflows/verify.yml/badge.svg)](https://github.com/ryanduguid/accounting-excel-toolkit/actions/workflows/verify.yml) [![License: MIT](https://img.shields.io/badge/License-MIT-4F485E.svg?labelColor=04001F)](LICENSE)

Power Query (M) functions and VBA modules for accountants working with Australian ledger exports: a Xero trial balance parser, Xero aged receivables and payables parsers, an AU financial-year helper, ABN validation, workpaper formatting, keyed reconciliations.

Every function here solves a problem the Australian ledger-export formats create. Written independently, from scratch, in my own time and on my own equipment.

## Power Query functions

One `.pq` file per function. To load one into Excel:

1. Open the `.pq` file in any text editor (Notepad works) and copy all of it.
2. In Excel, open the **Data** tab, click **Get Data**, then **From Other Sources**, then **Blank Query**. The Power Query Editor window opens.
3. On the Power Query Editor's **Home** tab, click **Advanced Editor**.
4. Delete the placeholder text in the editor, paste the copied contents, and click **Done**.
5. In the **Query Settings** pane on the right, type the file's name (for example `Xero.TrialBalance`) into the **Name** box. Other queries call the function by this exact name, so match it.
6. Click **Close & Load** on the **Home** tab to save it into the workbook.

Power BI is the same from step 3 onward: **Get Data**, **Blank Query**, then **Advanced Editor**.

| Function | Category | What it does |
|---|---|---|
| [`Xero.TrialBalance`](powerquery/Xero.TrialBalance.pq) | Xero | Parse a Xero TB CSV: skips metadata rows, picks the right Debit/Credit pair (plain pair = period movement, YTD pair = as-at balances; default returns as-at, `useYTD = false` for movement), drops Total row, splits account code as text (alphanumeric up to 10 chars with at least one digit, leading zeros survive). Handles two observed CSV layouts: a combined account cell (`Business Bank Account (090)`) and separate `Account Code` / `Account Name` columns |
| [`Xero.AgedReceivables`](powerquery/Xero.AgedReceivables.pq) | Xero | Parse a Xero Aged Receivables Summary CSV: skips metadata rows, finds the header by name (`Contact` or `Customer`, returned as `Contact`), drops the summary Total row, keeps the contact key as text, and types every ageing bucket and the Total under a pinned en-AU culture (a blank cell is a genuine zero; any other cell that will not parse errors and names the column and the value). Buckets and Total come back as Xero exported them, so the bucket-to-total tie-out stays the caller's |
| [`Xero.AgedPayables`](powerquery/Xero.AgedPayables.pq) | Xero | The same contract on the payables side: header by name (`Contact`, `Supplier` or `Vendor`, returned as `Supplier`), summary Total row dropped, supplier key forced to text, ageing buckets and Total typed under en-AU with the same blank-is-zero and refuse-to-guess rules, and returned as exported |
| [`PaydaySuper.Report`](powerquery/PaydaySuper.Report.pq) | Close inputs | Load the fixed 18-column `payday-super-checker` report contract by header name. It keeps IDs as text, retains the producer's raw `verdict`, caveats, notes and unassessable range, types producer amounts without recalculating them, and returns the terminal provenance separately as metadata |
| [`Fx.PromoteHeaderAt`](powerquery/Fx.PromoteHeaderAt.pq) | Generic | Find-and-promote the real header row in any ledger export that buries it below title rows; errors clearly when the format changed |
| [`Fx.AUFinancialYear`](powerquery/Fx.AUFinancialYear.pq) | AU helpers | FY label, start, end for any date (1 July to 30 June); timezone-stamped values are read in AEST (+10), so an instant in the last two hours of 30 June in Perth (last 30 minutes in Adelaide/Darwin) lands in FY+1 unless you switch the zone first |
| [`Fx.ABNIsValid`](powerquery/Fx.ABNIsValid.pq) | AU helpers | ABN checksum validation (ATO weighting algorithm); checksum ≠ registered, so check ABN Lookup for status |

Test against [`samples/sample-xero-trial-balance.csv`](samples/sample-xero-trial-balance.csv) (combined layout) and [`samples/sample-xero-trial-balance-columns.csv`](samples/sample-xero-trial-balance-columns.csv) (separate-column layout). Both are fabricated, balanced TBs carrying the same accounts and amounts in both shapes, plus both the period-movement and YTD (as-at) pairs so the pair selection gets exercised. They also include a code-less `Rent (Sydney)` account: it must load with a null `AccountCode` and its full name intact.

The separate `Account Code` / `Account Name` column shape was observed in Xero's interactive Trial Balance CSV export on 20 August 2026. The checked-in CSVs are fabricated fixtures, and their static and native acceptance checks establish parser behaviour for these two shapes only, not an official or stable Xero interactive-export schema. If Xero changes the interactive report or export UI, validate the parser with a fresh non-client export kept outside the repository before relying on it.

After loading a TB, the first check is always: `Number.Abs(List.Sum(result[Debit]) - List.Sum(result[Credit])) < 0.005`, a tolerance, because the sums are IEEE doubles and exact `=` can fail on a genuinely balanced TB.

`PaydaySuper.Report` accepts the named, 18-column CSV producer contract from `payday-super-checker`; additional producer columns are ignored, but a renamed, missing or duplicate contract header raises an error. [`samples/sample-payday-super-report.csv`](samples/sample-payday-super-report.csv) is a wholly fabricated Payday Super report. It includes a leading-zero ID and a formula-like ID already escaped with an apostrophe so both remain text through the adapter.

The full-width terminal provenance record has `employee_id = NOTE`, provenance in `notes`, and blank values in every other contract field. The function removes only that shape from the returned table and retains its `notes` value in table metadata as `PaydaySuperProvenance` (for example, `Value.Metadata(PaydaySuper_Report(path))[PaydaySuperProvenance]`). A contribution whose literal employee identifier is `NOTE` remains data when its contribution fields are populated. Ordinary table loads reject an absent, duplicate, non-terminal or empty-provenance record, a report with no contribution rows, and a CSV ending inside a quoted field without requiring the caller to inspect metadata. This includes malformed quoting in an otherwise ignored extra producer column. Every CSV record must contain the same number of fields as the header: a present empty trailing field is valid, while an absent field is malformed. The adapter does not convert `UNKNOWN` into exposure, derive a replacement verdict or recompute any amount: the raw `verdict` and every producer caveat, note and amount remain authoritative for professional review.

For the contracts that remain deliberately unimplemented, see [the close-input contract roadmap](docs/close-input-contract-roadmap.md). It separates the contracts that have shipped, this producer contract and the two Xero aged summary parsers above, from the MYOB-specific parsers still behind an evidence gate.

## VBA modules

Importable `.bas` source in [`vba/`](vba/). See [`vba/README.md`](vba/README.md) for encoding rules and contribution notes.

| Module | Platform | What it does |
|---|---|---|
| [`modWorkpaperFormat`](vba/modWorkpaperFormat.bas) | Windows and Mac Excel | Workpaper header block, reviewer sign-off line, accounting number format, freeze panes |
| [`modReconCompare`](vba/modReconCompare.bas) | Windows Excel only | Keyed two-way recon between (key, amount) ranges with tolerance; duplicate keys summed; subledger vs GL pattern |

`modReconCompare` late-binds `Scripting.Dictionary`, which only exists in the Windows scripting runtime. Mac Excel has no `Scripting.Dictionary` and no reference can supply it, so the module raises a clear error on Mac instead of failing mid-run. `modWorkpaperFormat` uses no `CreateObject` call and runs on both platforms.

To import a module into Excel:

1. If the ribbon has no **Developer** tab: click **File**, then **Options**, then **Customize Ribbon**, tick **Developer** in the right-hand list, and click **OK**.
2. On the **Developer** tab, click **Visual Basic** (or press `Alt+F11`). The Visual Basic Editor opens.
3. In the Visual Basic Editor, click **File**, then **Import File...**, and pick the `.bas` file.
4. Close the Visual Basic Editor. Save the workbook with **File**, **Save As**, choosing **Excel Macro-Enabled Workbook (*.xlsm)** as the file type. A plain `.xlsx` save silently drops the module.

## What is tested where

Three layers check this repository, and each covers different ground:

- **CI (GitHub Actions, [`verify.yml`](.github/workflows/verify.yml))** runs the Python suite in `tests/` on every push and pull request, with no Excel present. These are static source checks: they read the `.pq` and `.bas` files as text and pin the guards, constants and structures the docs promise. Covered: the M parsers' predicates, pair selection, header promotion and AEST conversion expressions; the VBA recon sheet-marker safety logic and protected-sheet guards; the accounting number format staying byte-identical across both modules; `.bas` files staying ASCII with CRLF endings; sample fixtures balancing and matching across both layouts; README sentences the test docstrings quote; release archive determinism; and the PowerShell runner's own safety properties (portability, fabricated inputs only, COM cleanup). CI never executes M or VBA.
- **[`tools/native_excel_acceptance.ps1`](tools/native_excel_acceptance.ps1)** runs the Power Query functions for real. It evaluates 72 checks in Excel's actual Power Query engine: both fabricated trial-balance layouts, the fabricated Payday Super producer contract, financial-year boundaries, ABN validation, header promotion, and adverse and lazy-evaluation branches. The default run isolates the 46 core checks and 26 Payday Super checks in fresh child PowerShell and Excel processes; the Payday child uses 20 separate single-source queries across 19 fabricated files to avoid Excel's cross-source privacy/firewall composition boundary. The suite also preserves a quoted multiline field, materialises 500-, 5,000- and 10,000-contribution fabricated reports, and prints each measured refresh time. It needs Windows, Windows PowerShell 5.1+, desktop Excel with Power Query, and the `Microsoft.Mashup.OleDb.1` provider. It does not import or execute VBA. The two aged parsers load into the workbook with every other `.pq` file, but no check yet calls them.
- **Manual Excel run** is the only check for VBA behaviour end to end: importing the modules, running `modWorkpaperFormat` and `modReconCompare` against real worksheets, and confirming Mac behaviour (the recon module's platform error message). Nothing automated executes the macros.

## Principles

1. **Source as text.** M in `.pq` files, VBA in `.bas` files. Nothing lives only inside a binary workbook.
2. **Exports are hostile input.** Parsers find the header row instead of assuming row counts, force account codes to text, and fail loudly when the format changed.
3. **No client data, ever.** Fixtures are fabricated and follow the `samples/sample-*.csv` naming convention. The `.gitignore` allowlists only that pattern, so a real export dropped into `samples/` stays blocked. Real exports stay outside any repo.

## Roadmap

See [the close-input contract roadmap](docs/close-input-contract-roadmap.md) for the evidence required before any MYOB parser is added; both Xero aged parsers shipped in v0.1.5 ahead of their evidence gate, which stays open until its own criteria are met. Other planned work: Office Scripts ports of the VBA modules and an auto-generated function catalogue.

## Related

[`australian-accounting-skills`](https://github.com/ryanduguid/australian-accounting-skills): Claude Code skills for AU public-practice workflows; its `xero-exports` skill pairs with these parsers.

[`xero-trial-balance-export`](https://github.com/ryanduguid/xero-trial-balance-export): the API path for the same trial balance; use it when OAuth app access is on the table and skip the CSV cleanup entirely.

## Disclaimer

Utility code, MIT-licensed, no warranty. Nothing here is tax or financial advice; outputs need professional review like any workpaper input. Xero and MYOB are trademarks of their respective owners. This project is independent and not affiliated with or endorsed by either.

## Author

Ryan Duguid, accountant in Newcastle NSW, provisional member of Chartered Accountants ANZ.

## CI coverage

`verify.yml` runs static Python guards. `tools/native_excel_acceptance.ps1`
(72 checks) and VBA end-to-end remain local Windows work. Power Query
behaviour is not executed on GitHub-hosted runners. Tagged releases call
`ryanduguid/release-policy` `release-archive.yml` pinned by full commit SHA.
