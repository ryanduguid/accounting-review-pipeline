# Evidence schedules

Source-only additions for local preparation. Run from this component after installing its dev environment:

```powershell
cargo run --locked --release --bin close-control -- schedule --kind expenses --input examples/schedules/expenses.json
cargo run --locked --release --bin close-control -- schedule --kind migration --input examples/schedules/migration.json
cargo run --locked --release --bin close-control -- schedule --kind interentity --input examples/schedules/interentity.json
cargo run --locked --release --bin close-control -- schedule --kind prepayments --input examples/schedules/prepayments.json
cargo run --locked --release --bin close-control -- schedule --kind rollforward --input examples/schedules/rollforward.json
```

Each fabricated example returns REVIEW (exit 2). JSON goes to standard output. PASS exits 0; malformed input exits 1. A PASS concerns only the supplied records, never approval, completeness of a ledger or a tax conclusion. Redirect output to a new file outside the checkout when retaining a workpaper. The result contains the SHA-256 of the exact input bytes.

The example files are the version 1 contracts. Fields are mandatory, unexpected fields and duplicate JSON keys fail. IDs are case-sensitive text; leading zeros survive. Money is a decimal string with at most 2 places and 16 integer digits. Arithmetic uses Decimal without rounding. Currency and cutoff must be supplied explicitly. These are canonical preparation schemas, not verified vendor exports.

## Expense claims

Invoices are gross source amounts. Claims allocate those invoices to named employees; payments link explicitly to claim IDs. The ledger balance is the independently supplied credit-positive closing expense-payable balance, on the same scope as all claims less all payments. Include brought-forward unpaid claims and linked source invoices in the opening population. This first contract does not infer posting dates or tax treatment.

The example's $100 claim receives 2 payments of $60. Both payments remain visible, the $20 overpayment requires review, and the ledger tie-out can still agree. Missing receipts, repeated supplier/reference pairs, excessive source allocations, unapproved claims, unpaid approved claims and orphan payments are separate findings. Exact duplicate references prompt inspection; they do not prove duplicate expenditure. Empty evidence explicitly means missing evidence.

Use the existing `reconcile` command for clearing transaction matching and aged carry-forward. This schedule consumes already-linked claim and payment evidence; it does not provide a second matching engine.

## Migration

Supply balances or open items at a common cutoff in both arrays, using the same record grain. Each row has its own unique row ID. Item IDs, reference and tax codes must survive the conversion exactly under this policy. Different target tax codes require a separately reviewed conversion policy, which version 1 does not support.

Explicit weights support one source account split across target accounts. Positive weights must sum to 1 per source. No account-name inference occurs. Splits retain exact arithmetic, so fractional-cent differences remain visible. Missing zero-value records also appear. Repeated item/account pairs are flagged even when their amounts offset. A trial-balance total cannot prove that invoices survived: the example keeps the total at $200 while duplicating I1 and losing I2.

## Inter-entity

Declare entity IDs and counterparties explicitly. Amounts are debit-positive; reciprocal balances should sum to zero for each entity pair and exact reference. The example's $50,000 receivable and $47,000 payable expose a $3,000 difference. Different references never cancel each other. Disputed items require review even when amounts agree. Dates beyond the cutoff fail rather than being silently dropped.

Outputs retain the source records and supplied evidence references. Timing explanations and possible eliminations remain the reviewer's work. The tool does not infer counterparty identities, calculate Division 7A or produce consolidation entries.

## Prepayments and accruals

The schedule declares the reporting period with `period_start` and `cutoff`, and
lists `prepayments`, `accruals` and the ledger's closing `ledger_balances`
(debit-positive) for the accounts they sit in.

A prepayment has an `amount` paid for the service period from `start` to `end`
and a `basis`. On a `daily` basis its unexpired part at the cutoff is the
amount times the days after the cutoff in that period, over the days in the
period. On a `monthly` basis it is the amount times the whole months after the
cutoff, over the months in the period; `start` must be the first of a month and
`end` and the cutoff month ends. A prepayment whose period starts after the
cutoff is wholly unexpired, and one that has ended is nil. Each unexpired part
is rounded to the cent, half up.

An accrual has an `amount` `accrued` on or before the cutoff and the date it
was `reversed`, or `null`. It is outstanding at the cutoff, as a credit, until
a reversal on or before the cutoff. An outstanding accrual raised before
`period_start` is `UNREVERSED_ACCRUAL`: an earlier period's accrual still open,
which double-counts the cost once its bill is posted.

Each account's scheduled balance is the sum of its unexpired prepayments less
its outstanding accruals. `BALANCE_DIFFERENCE` gives the ledger balance less
that sum; `NO_LEDGER_BALANCE` names a scheduled account with no ledger balance;
`UNSCHEDULED_BALANCE` names a ledger account with a balance and nothing
scheduled; `MISSING_EVIDENCE` names an item with empty evidence. The example
amortises a monthly insurance prepayment to $900 and a daily licence to $350,
finds the licence account still at its $365 cost, and finds a June accrual
never reversed and without evidence. The schedule does not post the
amortisation or reversal, judge whether a cost should be prepaid or accrued, or
find items missing from it.

## Roll-forward

`--kind rollforward` rolls each prepayment, deferred revenue and accrual
account from its opening balance through the period's additions and releases
to its closing balance, and ties both ends to the ledger. The schedule declares
`period_start` and `cutoff`, and lists `prepayments`, `deferred_revenue`,
`accruals`, the ledger's `opening_balances` (the day before `period_start`) and
its closing `ledger_balances`, all debit-positive.

Prepayments and deferred revenue take the prepayment fields above plus
`recorded`, the date the item was first booked, on or before the cutoff. Both
amortise as a prepayment does, on the same `daily` or `monthly` basis; a
monthly item also needs `period_start` on the first of a month. Deferred
revenue is a liability, so its balances are credits (negative). Accruals take
the same fields as above.

For each item and each account:

| Column | Holds |
| --- | --- |
| `opening` | An item recorded before `period_start`: its unexpired part, or an accrual outstanding, the day before. Otherwise nil. |
| `additions` | An item recorded, or an accrual raised, in the period: its full amount. |
| `releases` | What expired or was reversed in the period: `closing` less `opening` and `additions`. |
| `closing` | The unexpired part, or the accrual outstanding, at the cutoff. |

An item recorded part-way through its service period is added in full, and
the part already expired is released in the same period. Each account's row
gives the ledger's `ledger_opening` and `ledger_closing` and their
`opening_difference` and `closing_difference`. Findings:
`OPENING_DIFFERENCE` and `BALANCE_DIFFERENCE` for an end that does not tie,
`NO_LEDGER_BALANCE` (with the `end`) for a scheduled account missing from a
ledger list, `UNSCHEDULED_BALANCE` for a ledger account with a balance at
either end and nothing scheduled, and `UNREVERSED_ACCRUAL` and
`MISSING_EVIDENCE` as above.

```powershell
cargo run --locked --release --bin close-control -- schedule --kind rollforward --input examples/schedules/rollforward.json
```

The fabricated schedule rolls September 2026: insurance from $1,000 to $900,
a support contract's deferred revenue from $2,400 to $2,200 with a $500
workshop deposit added, and the audit fee accrued while August's cleaning
accrual reverses. It finds the licence account still at its $365 cost, $15
above the $350 unexpired.

What the roll-forward tests show: the tests use fabricated schedules whose
expected figures were worked out by hand from the rules above. Within each row
`closing` equals `opening` plus `additions` plus `releases` by construction,
because `releases` is derived from the other 3, so that identity cannot fail
and proves nothing. What can fail is each end against the ledger, and the
tests check that a wrong opening, a wrong closing, a missing ledger balance
and an unscheduled balance are each listed. They cannot show that a schedule
lists every item, that an item's service period or `recorded` date is right,
or that releases were posted to the right revenue or expense account.
