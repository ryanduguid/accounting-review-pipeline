# Evidence schedules

Source-only additions for local preparation. Run from this component after installing its dev environment:

```powershell
uv run --locked close-control schedule --kind expenses --input examples/schedules/expenses.json
uv run --locked close-control schedule --kind migration --input examples/schedules/migration.json
uv run --locked close-control schedule --kind interentity --input examples/schedules/interentity.json
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
