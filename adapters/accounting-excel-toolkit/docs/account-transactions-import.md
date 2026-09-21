# Xero Account Transactions import

The observed Xero Demo Company AU workbook has report title, tenant and period rows followed by 9 headings: Date, Source, Description, Reference, Debit, Credit, Running Balance, Gross and GST. Account names form sections, each with opening and total rows. It has no account code or stable source-line ID. This shape was inspected locally on 21 September 2026.

The new standard-library converter handles that worksheet after native Excel recalculation and conversion to CSV. It does not claim direct XLSX support. Inspect before conversion:

```powershell
python tools/xero_account_transactions.py samples/xero-account-transactions.csv --tenant "Fabricated Services" --account-name Clearing
```

Inspection preserves source row numbers and the exact source SHA-256. For conversion, supply a JSON mapping with exactly `source_sha256` and `lines`. The latter maps every selected CSV row number, as text, to a stable source-line identifier. Obtain those identifiers from retained source evidence. Row numbers are locators inside one hash-bound export, not transaction identity.

```powershell
python tools/xero_account_transactions.py source.csv --tenant "Entity label" --account-name "Account label" --account-id 090 --currency AUD --mapping reviewed-line-map.json
```

The converter independently adds detail debits and credits and requires agreement to the exported account subtotal. Title rows may move; headings and record widths may not change silently. Missing amounts, changed headers, dates outside the report period, mismatched totals, incomplete line maps and repeated IDs fail. It emits the existing clearing CSV contract on standard output. Use a new output file outside the checkout and retain the original source and mapping. The clearing reconciler separately needs opening/closing ledger balances and prior outstanding items.

A fabricated conversion is covered by tests. Native Excel conversion of the observed workbook was also checked on 21 September 2026. All 23 Accounts Payable detail rows reconcile: debits 8938.05 and credits 11013.00. Dates such as `21 Jul 2025` and correctly grouped amounts such as `6,661.60` are supported, alongside numeric Australian dates. The source workbook remained byte-identical. Stable IDs remain unavailable in that report itself, so conversion into persistent clearing items still requires separate source identity evidence. Do not invent line IDs or treat equal totals as proof that no offsetting duplicates exist.

MYOB and payroll vendor observations remain unavailable locally. Existing synthetic payroll profiles do not establish real-export compatibility. The existing native Power Query acceptance suite passed 87 checks in Excel 16.0 build 20430, en-AU, on 21 September 2026; those checks are separate from this new converter.
