# Variance drivers

A `period_variance` exception says an account moved beyond both thresholds. The
reviewer still has to explain why. `close-control drivers` lists the largest
transactions posted to each of those accounts between the pack's two report
dates, so the explanation starts from ledger evidence. It writes no commentary
and clears nothing: the reviewer decides what drove the movement and whether it
is right.

The development source adds this command; the published 0.1.5 package does
not include it.

```bash
close-control drivers --pack-dir ../close-demo/pack \
  --transactions examples/variance_transactions.csv \
  --currency AUD --top 5 --output ../close-demo/drivers
```

`--pack-dir` must hold a review pack that verifies with `close-control view`; an
edited pack is refused. `--transactions` uses the same columns as
`close-control reconcile`:

```text
Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit
```

Dates use `YYYY-MM-DD`, and each row carries exactly one positive `Debit` or
`Credit` with at most two decimal places. `--currency` names the trial
balance's currency, and a row in any other currency is refused: amounts are
compared with the trial balance, never converted. A repeated `TransactionID` for the
same account is refused. A Xero Account Transactions report converted for
`reconcile` works here unchanged. Only rows dated after the prior report date
and on or before the current report date count.

The output directory must be new and outside version control. It receives:

- `variance-drivers.json`: the window, the digest of the transaction file and of
  each pack file read, and per account the movement, the total of the supplied
  transactions, the unexplained remainder and the ranked transactions;
- `variance-drivers.csv`: one row per listed transaction, or one row with a
  blank `Rank` for an account with no transactions in the window. Text that a
  spreadsheet would read as a formula is prefixed with an apostrophe.

Transactions are ranked by absolute amount, then date, then `TransactionID`.
`Unexplained` is the account's YTD movement less the window's transactions.
The command exits `0` when every variance account has nil unexplained, `2`
when any does (`REVIEW`), and `1` for a malformed input, a pack that does not
verify or an existing output directory. A non-nil remainder usually means the
transaction file is incomplete or covers a different window. When the pack
carries a `financial_year_reset` exception, profit-and-loss YTD figures have
restarted, the movement and the window's transactions are not comparable, and
the run returns `REVIEW` whatever the remainders are.

With the fabricated example, Trade Debtors is fully covered, Operating
Expenses has $600.00 unexplained and the other 5 variance accounts have no
transactions supplied, so the run returns `REVIEW`.
