# MYOB Business aged-receivables discovery and input contract

Status: **DISCOVERY COMPLETE, CONTRACT DRAFT v0.1. NO PARSER.**

No MYOB export has been observed yet. This document records the product
decision, the export-mode constraints from MYOB's own support material, and
the contract skeleton a future `MYOB.AgedReceivables` query must satisfy once
an independent observation exists. It exists so the observation session has a
fixed target and so nobody infers a schema from AccountRight documentation,
screenshots or another product's export.

## Product under contract

**MYOB Business** (browser product), Pro subscription tier. Not AccountRight.

MYOB Business and AccountRight are separate products with separate report
engines and separate export paths. Nothing in this repository may label an
AccountRight layout as MYOB Business support, and vice versa.

## First slice

Receivables ageing summary (customer balances by ageing bucket). Payables
follows as its own contract once receivables is observed and shipped; do not
assume identical shapes.

## Export-mode constraints (from MYOB support, checked 2026-08-24, re-read 2026-09-13)

Sources:

- Browser view: <https://www.myob.com/au/support/myob-business/reporting/exporting-reports?productview=Browser>
- Desktop view: <https://www.myob.com/au/support/myob-business/reporting/exporting-reports?productview=Desktop>
- Company-file data export: <https://www.myob.com/au/support/myob-business/import-export/exporting-data>

The exporting-reports article carries a Browser view and a Desktop view of the
same URL, and they describe different report windows. Only the Browser view
covers the product under contract. The Desktop view is recorded below as
background so nobody reads its instructions as constraints on our export.

### Browser view (the product under contract)

1. Reports export as **Excel or PDF only**. The Browser view offers no CSV or
   TSV option for reports, so the parser contract must accept an `.xlsx`
   workbook, not a text export like the Xero contracts do.
2. **What you see is what you get**: the exported report mirrors on-screen
   customisation. Filtering the information, and adding, removing or
   reordering columns, all flow into the export, and a report with account
   levels keeps the level chosen on screen. Two observers with different
   customisations produce different headers. The observation step must fix one
   named customisation and record it.
3. Exporting to Excel requires Microsoft Excel 2010 or later installed on the
   machine, not Excel reached through a web browser.

### Desktop view (AccountRight report window, background only)

4. The Desktop view is where the `Insert/Modify` tab, the Show/Hide column
   flow and the `Print Preview` step that saves a customisation before export
   appear, together with the `Account No.` column. Its FAQ names the balance
   sheet, profit and loss, accounts list and trial balance reports as the ones
   that omit account numbers by default. It also offers XPS, CSV and TSV
   alongside PDF and Excel. None of this has been observed in the browser
   product.
5. The Desktop view names 2 reports as not exportable at all (Card List
   [Detail], Employee Employment Details). Neither is our target, and neither
   exception has been shown to apply to the browser product.
6. Separately from reports, the Import and export data assistant exports
   selected fields of company-file data as comma-separated or tab-separated
   text, for moving data into another AccountRight company file. That is a
   list export, not an aged balance, and is out of scope for this contract.

**Unresolved:** whether the browser product exposes a customer account number
on this report at all, and what it is called there. The contract keeps the
display name as the customer key until an observation settles it. Do not
import the Desktop view's account-number flow into this contract.

## Contract skeleton v0.1 (`MYOB.AgedReceivables`, not implemented)

Input: one `.xlsx` workbook produced by MYOB Business's report Export >
Excel flow for the receivables ageing summary, with the customisation named
below.

| Aspect | Contract value | Basis |
| --- | --- | --- |
| Sheet | Single worksheet; first sheet | To confirm at observation |
| Title rows | Expected above headers; skipped by exact match | To observe |
| Header row | Matched by exact header names, order-insensitive | Repo convention |
| Customer key | Display name coerced to text | Default export; the `Account No.` column is Desktop-view guidance and is unresolved in the browser product |
| Bucket columns | Returned exactly as exported | Xero contract precedent |
| Total column | Returned exactly as exported | Xero contract precedent |
| Summary/total rows | Dropped; tie-out stays the caller's | Xero contract precedent |
| Sign convention | Preserve source values; no resigning | Repo boundary |
| Currency | AUD assumed; no conversion ever | Repo boundary |
| Date semantics | As-at report date recorded from the workbook if present | To observe |
| Shape change | Fail closed with a named error, never best-effort | Repo boundary |
| Formula injection | Guard `=`, `+`, `-`, `@` per CONTRIBUTING rules | CONTRIBUTING.md |

Every cell marked 'to observe' is unresolved. Filling them from anything other
than a real export is prohibited.

## Acceptance required before a parser ships

1. One fresh, non-client observation outside this repository recording: exact
   sheet name, title-row lines, every header string verbatim, bucket labels,
   total-row label and placement, date presentation, and the named report
   customisation used. Commit only the minimum non-sensitive shape evidence.
2. A fabricated header-only fixture built from that observation, never from
   this document.
3. Static tests: valid parse, header drift rejection, missing bucket rejection,
   altered digest-style tamper rejection, malformed workbook rejection.
4. Native Excel acceptance run before release, matching the Xero contracts.
5. This document updated to v1.0 with the observed values replacing every
   'to observe' entry, and `docs/close-input-contract-roadmap.md` moved out of
   the MYOB evidence gate.

## Boundaries

- Client exports stay outside the repository. Fabricated samples only.
- No OAuth, network calls, Excel automation, write-back or cross-repo runtime
  dependencies in the adapter.
- The query reports buckets; it makes no credit, collectibility or close
  decision. Human review remains the decider.
