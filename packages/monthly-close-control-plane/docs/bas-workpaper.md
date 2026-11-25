# Drafted BAS workpaper

`close-control bas` reads one tenant's ledger export of sales and purchases,
with the tax code and GST on each row, and drafts the business activity
statement labels a reviewer checks: G1, G2, G3, G10, G11, 1A and 1B, plus W1
and W2 when a payroll register is supplied. Every label total names the rows
behind it, and every row in the period that cannot be placed is listed rather
than guessed. It reads local files, writes JSON to standard output and changes
nothing. It lodges nothing, and its output is a draft for a reviewer, not a
BAS. It exits `0` for `PASS`, `2` for `REVIEW` or `BLOCKED` and `1` for
malformed input or options.

From the component's source:

```text
cargo run --locked --release --bin close-control -- bas --ledger examples/bas_ledger.csv --tax-codes examples/bas_tax_codes.csv --payroll examples/payroll.csv --currency AUD --period-start 2026-07-01 --period-end 2026-09-30 --basis accruals
```

The fabricated ledger holds the September 2026 quarter for one company. The
command drafts G1 of $8,610.00 (taxable, export, GST-free and input-taxed
sales, less a credit note), G10 of $2,750.00 for a computer, G11 of $825.00,
1A of $290.00 and 1B of $280.00, and W1 and W2 from the payroll example. It
exits `2` with `BLOCKED`, because row `U-1` carries a tax code the map does not
place.

## Supply the ledger

```text
Tenant,LineID,Date,PaidDate,AccountID,TaxCode,Gross,GST,Currency
```

- One `Tenant`, each `LineID` once, every row in the declared `--currency`.
- `Gross` is the amount including GST and `GST` the GST in it, each with at
  most 2 decimal places. A credit note or refund is negative in both.
- `Date` is the invoice date. `PaidDate` is when the row was paid in full, or
  blank when unpaid. On the cash basis, supply one row per payment for a part
  payment, as the cash basis reports only the part paid.
- Rows outside the period may be included; they are counted, not drafted.

## Map the tax codes

```text
TaxCode,AccountID,Treatment
```

The command does not know what any ledger's tax code means: the map says. A
row with a blank `AccountID` places that tax code on every account; a row
naming an account overrides it there, which is how a ledger that uses one
purchase code for running costs and equipment separates capital purchases.

| `Treatment` | Gross goes to | GST goes to |
| --- | --- | --- |
| `taxable_sale` | G1 | 1A |
| `export_sale` | G1 and G2 | carries none |
| `gst_free_sale` | G1 and G3 | carries none |
| `input_taxed_sale` | G1 | carries none |
| `capital_creditable` | G10 | 1B |
| `capital_no_gst` | G10 | carries none |
| `capital_not_creditable` | G10 | not claimed |
| `non_capital_creditable` | G11 | 1B |
| `non_capital_no_gst` | G11 | carries none |
| `non_capital_not_creditable` | G11 | not claimed |
| `bas_excluded` | no label; listed under `bas_excluded_rows` | carries none |

Use `*_no_gst` for purchases with no GST in the price: GST-free or
input-taxed purchases and purchases from suppliers not registered for GST. Use
`*_not_creditable` for purchases with GST in the price that cannot be claimed:
private use, costs not deductible for income tax, or purchases for making
input-taxed sales.

## The rules, from the ATO

Read at ato.gov.au on 26 November 2026: 'Step 1: Sales' and 'Step 3:
Purchases' (both last updated 9 September 2026), 'Step 4: Calculating
purchases using the calculation worksheet' and 'Step 5: Summary' (9 September
2026), 'Identify your accounting basis' (14 September 2026) and 'Pay as you go
(PAYG) withholding' under business activity statements.

- G1 takes GST-free, input-taxed and taxable sales. G2 (exports) and G3 (other
  GST-free sales) are also in G1. Input-taxed sales are in G1 with no label of
  their own on the BAS (G4 is on the worksheet only).
- G10 takes capital purchases and G11 every other business purchase. Both
  include purchases with no GST in the price and purchases whose GST cannot be
  claimed, as the worksheet's G13, G14 and G15 must also be in G10 or G11.
- 1A is GST on sales and 1B the GST credits claimed.
- On the accruals (non-cash) basis a row belongs to the earlier period in which
  it was paid or invoiced, so it is placed by the earlier of `Date` and
  `PaidDate`. On the cash basis it belongs to the period in which it was paid,
  so it is placed by `PaidDate`, and an unpaid row is counted under
  `rows_unpaid`.
- W1 is total salary, wages and other payments, and W2 the amounts withheld
  from them, by pay date.
- The BAS takes whole dollars, cents dropped, and no negative figure. Each
  label gives its exact `amount` and its `whole_dollars`.

Amounts are drafted including GST (`amounts_include_gst`), which both the
calculation worksheet method and the accounts method allow.

## What is listed

| Field | Holds |
| --- | --- |
| `labels` | Each drafted label with its `amount`, `whole_dollars`, `source` (`ledger` or `payroll`) and the `rows` behind it: ledger `LineID`s, or `payroll row N`. |
| `unplaced` | Each row in the period that could not be placed, with its reason: no map row for its tax code, GST on a treatment that takes none, or GST that is not part of its `Gross`. |
| `bas_excluded_rows` | Rows in the period mapped `bas_excluded`. |
| `labels_not_drafted` | W1 and W2 when no `--payroll` register was supplied. |
| `findings` | `GST_REASONABLENESS` and `NEGATIVE_LABEL`, below. |

`GST_REASONABLENESS` reuses the control `review --gst-mapping` runs: 1A against
1/10 of the GST-exclusive taxable sales, 1B against 1/10 of the GST-exclusive
creditable purchases, and 1A less 1B against the net, each with that control's
default review heuristic of 1.00 plus 1% of the expected GST. It is a review
heuristic, not a rounding bound. `NEGATIVE_LABEL` is a label whose total is
negative, which the BAS cannot take.

The status is `BLOCKED` when any row in the period is unplaced, since the
labels would leave it out. Otherwise it is `REVIEW` when there is a finding or
no row falls in the period, and `PASS` when every row was placed and nothing
was listed. `PASS` means the draft is complete against the map, not that the
map or the coding is right.

## Limits

- The map is the reviewer's. A tax code mapped to the wrong treatment, or a row
  coded with the wrong tax code, puts its amount on the wrong label without any
  finding. The [coding review](coding-review.md) lists rows coded away from
  their usual tax code.
- Adjustments (worksheet G7 and G18), GST instalments, the financial
  acquisitions threshold, importations, the export split of freight and
  insurance to G3, luxury car tax, wine equalisation tax, fuel tax credits and
  PAYG instalments are not drafted. Map them `bas_excluded` and place them by
  hand, or leave them unmapped so the draft is `BLOCKED`.
- W1 takes the register's `Earnings` as given; salary sacrifice and other
  payments the register does not hold are not seen. The register is read as
  `close-control payroll` reads it, so pay dates must fall in 2026-27. W3, W4
  and W5 are not drafted.
- A part payment on the accruals basis is not split: the whole row is placed by
  the earlier date.

## What the tests show

The tests run on fabricated ledgers whose expected labels were worked out by
hand from the rules above. They show that the command applies those rules as
written: placement, totals, the rows named and the refusals. They are circular
in one respect: they cannot show that the rules match the ATO's, because the
same reading of the ATO's pages wrote both the code and the expected figures.
That check is the reviewer's, against the pages cited above.
