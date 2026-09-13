# Xero clearing-account export trial

Use one Australian Xero Demo Company account over 2 adjacent months to verify
the mapping before using this reconciler in a practice. The command currently
accepts [mapped CSV](clearing-reconciliation.md), not a native Xero workbook.
This trial has not yet been run against a Xero export or by a bookkeeper.

## What the primary sources establish

Research checked on 10 September 2026:

| Source | Finding | Consequence for this trial |
| --- | --- | --- |
| [Xero's payroll-account checking guide](https://central.xero.com/0/article/Check-transactions-and-fix-errors-in-your-payroll-accounts) | Account Transactions offers Date, Debit, Credit, Reference, Source and Account Code columns, account/date selection and opening/closing balances. | These are candidate mapping fields. Check the exported cells and totals before conversion. |
| [Xero's export guide](https://central.xero.com/0/article/Export-data-out-of-Xero-AU) | Xero lists Account Transactions among its exportable reports and advises including FX columns when exporting multicurrency data. | Retain the original export. The first trial should use AUD base-currency activity only. |
| [Xero's Journals API reference](https://developer.xero.com/documentation/api/accounting/journals) | Journal lines expose JournalLineID, AccountID and a base-currency NetAmount with debit-positive and credit-negative signs. Access requires an Advanced-tier app, security assessment and use-case approval. | This is a possible future source, not a dependency for the local trial. |
| [Xero's demo-company guide](https://central.xero.com/0/article/Use-the-demo-company) | The demo contains fictional data, requires a Xero login and resets after 28 days. | Capture both periods in the same demo lifecycle. Keep the original files together. |

The sources inspected do not establish a stable line identifier in the standard
Account Transactions spreadsheet export. This is an evidence gap, not proof that
no export can contain one. The main Account Transactions help URL returned no
readable article body during research. No native sample workbook was available.

## Collect the sample

In Xero's Australian Demo Company, choose one account with transaction activity
across 2 adjacent months. Prefer a clearing account with a known opening-item
list or a zero opening balance. Record the account, dates, base currency, report
basis and all filters used. Keep transaction detail unsummarised for this trial.

For each month, export the Account Transactions report as an Excel workbook with
Date, Debit, Credit, Reference, Description, Account Code and Source where those
columns are available. Show opening/closing balances and cents. Retain the untouched
workbook outside the checkout. Export the first month again with the same settings
so the same transactions can be compared across exports. Do not reset the demo
between captures. These settings are the proposed trial procedure; their exact
export layout remains to be checked.

## Mapping decisions to verify

| Canonical field | Proposed mapping and check |
| --- | --- |
| Tenant | A recorded, consistent demo-entity label. Confirm the workbook's organisation agrees. |
| AccountID | One recorded account identifier. An Account Code can be the local label if retained consistently; it is not proof of a Xero API AccountID. |
| Currency | AUD only after confirming the values use the demo organisation's AUD base currency. Do not mix FX values into base-currency amounts. |
| TransactionID | Verify a stable source-line key across repeat exports. Reference, date, amount and row number alone are insufficient. |
| Date | Convert the observed posting date explicitly to YYYY-MM-DD. Resolve ambiguous day/month formats from the export, not a guess. |
| Reference / Description | Preserve the available source text. Blank references are permitted and produce no suggestion. |
| Debit / Credit | Map the observed separate amounts, confirm their meaning, and preserve cents. Convert an empty unused side to 0 only after checking the export's convention. |

Keep a record of every omitted non-transaction row, such as titles, totals and
opening/closing balances. Compare the retained line count and separate debit and
credit totals with the original, then check opening balance plus movement equals
closing balance. The declared balances must come from the source evidence, not
from a total calculated solely from the mapped rows.

For IDs, inspect any identifier columns and source links preserved in the workbook.
A link to an invoice or bank transaction may identify a document with several
account lines; it is not sufficient on its own. Verify uniqueness within the
account and stability on the repeat export, including genuinely repeated lines.
Do not silently collapse identical rows or give each refreshed row a new ID.

If no suitable key exists, pause the repeat-import claim. A one-off manual mapping
may use a retained, reviewer-checked row register, but it must disclose that identity
is assigned by the operator and cannot reliably detect changed or repeated source
rows on refresh. Implement an importer only after this identity rule and an actual
export layout have been verified.

## Reconciliation cases checked locally

The automated scenario uses hand-derived amounts and exercises the real parser,
decision handling, report writer and next-period carry-forward. It is separate
from the native-export trial and has no independent human review.

| Case | July entries | July treatment | August treatment |
| --- | --- | --- | --- |
| Partial payment | Debit 100, credit 60 | Keep both original lines outstanding, net 40. | A further credit 40 allows all 3 lines to be accepted together. |
| Full reversal | Debit 20, credit 20 | Clear only after a reviewer accepts the pair. | No remaining lines. |
| Settlement net of fee | Debit 200, credit 197 | Keep both lines outstanding, net 3. | An existing source-ledger credit 3 permits the group to clear. The tool does not create the fee posting. |
| Unrelated equal amounts | Debit 50 and credit 50, different references | Keep both lines outstanding, net 0. | Keep REVIEW with both lines aged 52 days, despite the zero closing balance. |

July's source debits total 370 and credits total 327. Its closing balance is 43
with 6 outstanding lines after accepting the reversal. August's movement is
-43 and its closing balance is zero, but 2 unrelated lines remain outstanding.
The test is `test_partial_settlements_roll_forward_without_clearing_unrelated_zero_net_items`
in [test_reconciliation.py](../tests/test_reconciliation.py). Run the documented
component test command, `uv run pytest`.

## Bookkeeper trial record

Before viewing tool suggestions, the reviewer should independently list the
expected outstanding lines and valid allocation groups for each month. Compare
IDs, amounts, dates and notes with the generated results. Record discrepancies
even when the closing totals agree. Use source evidence to adjudicate differences.

Complete this record locally; it currently contains no trial result:

| Measure | Result to record |
| --- | --- |
| Source mapping | Workbook settings, ID rule, line count and separate debit/credit totals agree: yes/no, with discrepancies |
| Outstanding items | Expected versus actual IDs, amounts, original dates and notes for each month |
| Suggestions | Valid and invalid proposed groups, assessed against the reviewer's reference result |
| Repeat export | Same source lines retain the same IDs: yes/no, including duplicate-looking lines |
| Time | Minutes for the manual reference; separately, export/mapping, tool review and correction time |
| Review outcome | Every difference explained; willingness to use it for another period and why |

Proceed to a practice pilot only when the mapping and item-level results agree
with the reviewer. Report observed preparation and review time together; a faster
matching step can still lose time to export preparation. No adoption or time-saving
claim follows from the fabricated test alone.
