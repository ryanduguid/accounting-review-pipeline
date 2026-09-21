# Reconcile ledger equity movements

This source-only optional control compares an independent movement schedule with
selected equity accounts in two trial balances. It uses credit-positive balances
and exact decimal arithmetic. It adds no profit figure inferred from the ledger.

```text
close-control review --current current.csv --prior prior.csv --equity-schedule equity.json --equity-currency AUD --equity-currency-evidence "Both source report currency fields" --output OUTSIDE_CHECKOUT
```

The same options work with workbench. The ten-column trial balance has no currency
field, so the run declaration must come from the source reports or other reviewed
evidence, independently of the schedule. Currency text is a supplied assertion;
the engine cannot authenticate its evidence.

The schedule has exactly these fields: schema_version (integer 1), tenant,
opening_date, closing_date, currency, basis (ledger_equity), prior_source_sha256,
current_source_sha256, equity_account_ids, complete (boolean), and movements.
Dates are YYYY-MM-DD. Digests identify the exact trial-balance bytes. Selected
account IDs must be unique, present in both files and retain an explicit Equity
section. No name-based inference or currency conversion is performed.

Each movement has exactly movement_id, date, account_id, debit, credit, description
and evidence_reference. IDs are unique. Dates are after opening and on or before
closing. Debit and credit are non-negative decimal strings with exactly one side
positive. References are local review labels, never instructions to open a file
or fetch a URL. Empty movements with complete true explicitly declares none.

Per account: opening equity plus scheduled credits less scheduled debits gives
expected closing equity. Actual closing less expected closing is unexplained.
Individual accounts and their aggregate must each be within the supplied
reconciliation tolerance. Opposite errors cannot cancel. Missing completeness,
currency evidence, account continuity or matching source metadata produces BLOCKED.
Malformed input produces exit 1 before any output is replaced. Differences produce
REVIEW; a supported matching schedule produces PASS for this control. Other
controls still determine overall status. No acknowledgement changes it.

The additive pack member equity_reconciliation appears only when a schedule was
supplied. It contains schema_version, basis, tenant, opening_date, closing_date,
currency, currency_evidence, schedule_sha256, status, issues, accounts, total,
movements, tolerance and complete. Account entries contain account_id, opening,
movement, expected_close, actual_close, unexplained and movement_ids. Monetary
values remain decimal strings. A blocked result retains evidence but has no
calculated accounts or total.

The Markdown summary displays the arithmetic, movements and a digest witnessing
every member of the JSON result. Exceptions and eligible draft questions retain
the existing CSV contracts. The viewer verifies all four files together and
shows successful equity evidence too. Existing packs without this optional block
retain their previous shape.

Run the fabricated example from this component's installed development checkout:

```text
python examples/equity_demo.py --output OUTSIDE_CHECKOUT
```

The independent movements are a 25,000 contribution, 10,000 withdrawal and
15,000 evidenced profit transfer against opening equity of 100,000. Expected
closing is 130,000; the ledger closes at 129,750, leaving -250 for review.
The example asserts that arithmetic and verifies the generated pack. No journal
is posted and no period is approved.

Focused regression coverage: tests/test_equity.py. It covers matching and nil
movements, loss transfers, debit balances, sub-cent tolerances, offsetting errors,
incomplete evidence, changed identity and currency, malformed input, tampered
output, inert source text and preservation of an old pack on input failure.

