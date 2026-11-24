# Statement tie-out

`close-control statement` reads one account's bank statement, as its rows were
printed, and checks that its own figures are consistent: each line carries the
running balance forward, each page closes where its lines end and each page
opens where the page before closed. It reads a local file, writes JSON to
standard output and changes nothing. It exits `0` for `PASS`, `2` for `REVIEW`
and `1` for malformed input or options.

From the component's source:

```text
cargo run --locked --release --bin close-control -- statement --statement examples/statement.csv --currency AUD
```

The fabricated statement holds 2 pages of October 2026 lines. The command
exits `2` and lists a supplier payment of $1,245.00 after which the running
balance fell by $1,425.00, so the stated balance is $180.00 below the one the
line gives. Later lines carry the stated balance forward and are not listed
again.

## Supply the statement

```text
Account,Page,Kind,Date,Description,Amount,Balance,Currency
```

- Every row is for one `Account` in the `--currency` declared.
- Each page is an `OPENING` row, its `LINE` rows and a `CLOSING` row, in that
  order. Pages run from 1 with none missing.
- `OPENING` and `CLOSING` rows carry a `Balance` and a blank `Amount`. A `LINE`
  carries its signed `Amount`, deposits positive and withdrawals negative, and
  the running `Balance` printed beside it.
- Rows are in printed order, so `Date` never goes backwards.

A statement that breaks these rules is refused rather than reviewed, with the
row named.

## What it lists

| Code | When |
| --- | --- |
| `STATEMENT_RUNNING_BREAK` | A line's `Balance` is not the balance above it plus its `Amount`. |
| `STATEMENT_CLOSING_BREAK` | A page's closing balance is not the balance its last line reached. |
| `STATEMENT_PAGE_BREAK` | A page opens at a balance other than the previous page's closing balance. |

Each finding gives the row, page, date, the expected and stated balances and
their difference. The next row is checked against the stated balance, so each
broken equation is listed once. One mistyped balance can break 2: its own line
and the row after it.

## Limits

- A statement whose figures tie shows that they are arithmetically consistent,
  not that the statement is genuine, complete or matches the bank's records.
- The command does not match statement lines to the ledger; see bank
  reconciliation for that.
- Pages missing from the end of the statement, or a line whose amount and
  balance were both altered by the same figure, are not detected.
