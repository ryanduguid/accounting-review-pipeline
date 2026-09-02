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

## Export-mode constraints (from MYOB support, checked 2026-08-24)

Sources:

- <https://www.myob.com/au/support/myob-business/reporting/exporting-reports>
- <https://www.myob.com/au/support/myob-business/import-export/exporting-data>

1. Reports in MYOB Business export as **Excel or PDF only**. There is no CSV
   option for reports. The parser contract must therefore accept an `.xlsx`
   workbook, not a text export like the Xero contracts do.
2. **What you see is what you get**: the exported report mirrors on-screen
   customisation. Filters, removed columns and reordered columns all flow into
   the export. Two observers with different customisations produce different
   headers. The observation step must fix one named customisation and record it.
3. **Account No. is not shown by default.** Adding it requires the Insert /
   Modify > Show/Hide column flow before export, saved via Print Preview. The
   contract must decide whether the customer key is the display name (default)
   or the account number (customised), and reject the wrong variant rather
   than guessing.
4. MYOB names two reports as not exportable at all (Card List [Detail],
   Employee Employment Details). Neither is our target, but the fact confirms
   report-level quirks are real in this product.
5. Separately from reports, the Import and export data assistant exports
   selected fields for some data types. That is a list export, not an aged
   balance, and is out of scope for this contract.

## Contract skeleton v0.1 (`MYOB.AgedReceivables`, not implemented)

Input: one `.xlsx` workbook produced by MYOB Business's report Export >
Excel flow for the receivables ageing summary, with the customisation named
below.

| Aspect | Contract value | Basis |
| --- | --- | --- |
| Sheet | Single worksheet; first sheet | To confirm at observation |
| Title rows | Expected above headers; skipped by exact match | To observe |
| Header row | Matched by exact header names, order-insensitive | Repo convention |
| Customer key | Display name coerced to text | Default export; revisit if Account No. variant observed |
| Bucket columns | Returned exactly as exported | Xero contract precedent |
| Total column | Returned exactly as exported | Xero contract precedent |
| Summary/total rows | Dropped; tie-out stays the caller's | Xero contract precedent |
| Sign convention | Preserve source values; no resigning | Repo boundary |
| Currency | AUD assumed; no conversion ever | Repo boundary |
| Date semantics | As-at report date recorded from the workbook if present | To observe |
| Shape change | Fail closed with a named error, never best-effort | Repo boundary |
| Formula injection | Guard `=`, `+`, `-`, `@` per CONTRIBUTING rules | CONTRIBUTING.md |

Every cell marked "to observe" is unresolved. Filling them from anything other
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
   "to observe" entry, and `docs/close-input-contract-roadmap.md` moved out of
   the MYOB evidence gate.

## Boundaries

- Client exports stay outside the repository. Fabricated samples only.
- No OAuth, network calls, Excel automation, write-back or cross-repo runtime
  dependencies in the adapter.
- The query reports buckets; it makes no credit, collectibility or close
  decision. Human review remains the decider.
