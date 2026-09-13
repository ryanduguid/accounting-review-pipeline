# Reconcile a clearing account across month ends

`close-control reconcile` suggests transaction matches, records a reviewer's
allocations and carries outstanding items into the next period. It works locally
with one account and one currency. This command is an unreleased source addition;
the existing published package version does not establish its availability.

## Run the worked example

From `packages/monthly-close-control-plane` in an installed development checkout:

```bash
uv run --locked --extra dev python examples/clearing_demo.py --output ../../../clearing-demo
```

Choose a new output directory. Open `clearing-demo/july-reviewed/review.html` in
your browser. The 4 runs use fabricated inputs and allocations:

| Run | Opening balance | Current movement | Closing balance | Outstanding items | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| June | 0.00 | 100.00 | 100.00 | 1 | REVIEW |
| July before allocations | 100.00 | -75.00 | 25.00 | 6 | REVIEW |
| July with reviewed allocations | 100.00 | -75.00 | 25.00 | 1 | REVIEW |
| August with reviewed allocations | 25.00 | -25.00 | 0.00 | 0 | PASS |

July matches a June receipt against a July settlement, then 2 July receipts
against one settlement. The remaining $25 retains its original 31 July date.
August settles that item and demonstrates a manually supplied group with different
references. The expected amounts above are hand-derived from the supplied CSVs.
PASS means these checks found no remaining items; it does not approve the close.

## Prepare the transaction CSV

Use the exact headings in [the transaction schema](../schemas/clearing-transactions.csv).
Column order may differ. Include all current-period movements for this one account,
with no subtotal, opening-balance or closing-balance rows. A header-only CSV is valid
for a period with no movements. Supply ledger balances separately.

| Field | Required content |
| --- | --- |
| Tenant | One consistent entity label |
| AccountID | One consistent account identifier |
| Currency | One uppercase 3-letter code; no currency conversion |
| TransactionID | Stable, unique source-line identifier; 1-80 ASCII letters, digits, dots, underscores, colons or hyphens; first character alphanumeric |
| Date | Posting date as YYYY-MM-DD |
| Reference | Source reference; may be empty |
| Description | Source description; may be empty |
| Debit / Credit | Explicit non-negative amounts; exactly one side positive and the other 0 |

Amounts have at most 2 decimal places and magnitude below 10^15. The first
version supports at most 100,000 opening and current items together. It rejects
missing fields, non-finite amounts, duplicate IDs and mixed account identities.
It cannot detect an omitted transaction or a duplicate given a new ID merely by
inspecting that ID. Balance checks can also miss offsetting source errors.

Map your source export to this schema and retain the original. A trial balance
contains totals and is insufficient. Native Xero Account Transactions file import
has not been verified. Do not treat this as a drop-in importer for any export
headed 'Xero'. If the export lacks stable line IDs, document a repeatable mapping
before use; renumbering or inventing fresh IDs each month defeats duplicate checks.

## Review and record matches

For example, to reproduce July before recording decisions:

```bash
uv run --locked --extra dev close-control reconcile --transactions examples/clearing-july.csv --tenant Demo --account-id clearing --currency AUD --period-start 2026-07-01 --period-end 2026-07-31 --opening-balance 100 --closing-balance 25 --opening-items ../../../clearing-demo/june/carry-forward.json --output ../../../july-new-review
```

Exit 2 is the expected REVIEW result. Open the generated `review.html` and
`suggestions.csv`. The HTML needs no server, scripts or internet connection.

Suggestions group all items with an identical, non-empty reference only when the
group has 2 to 20 items and sums exactly to zero. Amount alone never produces a match.
Repeated references are not proof of a relationship; examine the source evidence.
Large groups, partial reference groups and different references require manual
selection. The tool does not search arbitrary combinations or split a transaction.

In the decisions CSV:

1. Set every row of a group to `accept` or `reject`, with the same explanatory
   note on each row. Leave Decision blank to keep the group pending. Pending
   groups retain their IDs, labels and draft notes when you rerun the period.
2. Add a manual group by giving 2 or more outstanding IDs a common Group label.
3. Each ID may occur only once in the decisions file. Accepted groups must sum
   exactly to zero. A rejected group remains outstanding with its note.
4. Run the command again with `--decisions path/to/reviewed-decisions.csv` and a
   new output directory. Include all earlier decisions for the period when revising
   it. Each run starts from its source files, with no hidden session state.

The generated suggestions file includes accepted, rejected and pending groups,
plus new suggestions for items not already grouped. It can be the starting point
for the next revision. Remove a pending group from the decisions file to make its
items eligible for suggestions again. Carry-forward retains original outstanding
items and non-empty notes; group membership belongs to the current period only.
Review notes
whose first character could start a spreadsheet formula are prefixed with an
apostrophe in CSV output. JSON preserves original text. An Excel edit may change
that text; inspect the saved CSV if exact note preservation matters.

## Outputs and balance checks

| File | Purpose |
| --- | --- |
| `review.html` | Balance checks, ageing, suggestions and recorded allocations |
| `reconciliation.json` | Transactions, decisions, balances and hashes of the exact source bytes read |
| `outstanding.csv` | Spreadsheet-safe outstanding transactions, AgeDays and ReviewNote |
| `suggestions.csv` | Editable review decisions and remaining proposed groups |
| `carry-forward.json` | Original outstanding data and notes for the next period, emitted only when balance checks agree |

The sum of opening outstanding items must equal the separately supplied opening
ledger balance. Opening ledger balance plus current movements must equal the
closing ledger balance. Debit balances are positive; credit balances are negative.
There is no rounding tolerance and no balancing plug.

Any difference gives BLOCKED and suppresses carry-forward. Agreement with remaining
items gives REVIEW; agreement with no remaining items gives PASS. Exit codes are
0 for PASS, 2 for REVIEW or BLOCKED, and 1 for malformed input or an output error.
These are reconciliation results, not accounting approval or period-lock states.

Use the preceding period's `carry-forward.json` with `--opening-items`. Its identity
must agree, its item total must equal its recorded balance, and its period end must
be the day before the new period starts. Original transaction dates and notes
survive. This is an operator-supplied record, not authenticated historical evidence.
Source hashes identify bytes; they do not prove completeness or authenticity.

Outputs must be outside version control and in a new directory for each run.
Existing directories are refused. An ordinary write failure removes this run's
files; a killed process may leave an incomplete directory. Discard incomplete
results and rerun into a new directory. Keep the output under the same access
controls as the source ledger.

## Why this workflow

Research on 10 September 2026 found a long-running
[Xero customer request for clearing and non-bank-account reconciliation](https://productideas.xero.com/forums/967136-banking-chart-of-accounts/suggestions/44960296-reconciliation-ability-to-reconcile-clearing-co).
Users describe matching transactions across months and maintaining outstanding
items in Excel. [RecHound](https://apps.xero.com/au/app/rechound) already provides
connected reconciliation and review workflows. This experiment tests a smaller,
local alternative using mapped files. Demand for that specific alternative,
reviewer time savings and willingness to pay have not been established.

## Before a practice pilot

Use the [Xero export trial](xero-clearing-pilot.md) to check source mapping,
repeat-export identity and an independent reviewer's outstanding-item list.

Have a bookkeeper check the mapped export and reference reconciliation. Compare
the tool's outstanding items with that independent result, including repeated
amounts, missing settlements, reversals and partial payments. Record preparation
and review time separately. The fabricated example proves reproducibility only;
it is not evidence of client outcomes, native Xero compatibility or reviewer adoption.
