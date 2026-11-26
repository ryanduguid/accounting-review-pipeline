# Overdue against history

`close-control overdue` reads one tenant's register of sales invoices, paid and
open, and lists each open invoice past its due date with a comparison against
how late that customer has paid before. A customer who always pays 35 days
late is not news at 30 days; one who pays within a week is news at 20. A fixed
ageing bucket cannot tell the two apart. The command reads a local file, writes
JSON to standard output and changes nothing. It exits `0` for `PASS`, `2` for
`REVIEW` or `BLOCKED`, and `1` for malformed input or options.

From the component's source:

```text
cargo run --locked --release --bin close-control -- overdue --invoices examples/receivables.csv --currency AUD --as-at 2026-09-30
```

The fabricated register has 12 invoices to 3 customers. The command exits `2`
with `BLOCKED`:

| Invoice | Customer | Days overdue | Customer's threshold | Result |
| --- | --- | --- | --- | --- |
| INV-3008 | Falador Freight | 30 | 48.0 | `WITHIN_HISTORY` |
| INV-3009 | Lumbridge Traders | 20 | 12.0 | `OVERDUE_AGAINST_HISTORY` |
| INV-3010 | Draynor Supplies | 15 | 12.0 | `NOT_ENOUGH_HISTORY` |
| INV-3011 | Lumbridge Traders | 10 | 12.0 | `WITHIN_HISTORY` |

Falador Freight has paid 32, 36 and 40 days late, so 30 days is within its
pattern, though a 30-day bucket would list it. Lumbridge Traders has paid 4, 5
and 6 days late, so 20 days is well outside its pattern. Draynor Supplies has
paid one invoice, too few to judge. INV-3011 was paid on 3 October, after the
as-at date, so it was open on 30 September. INV-3012 is not yet due.

## Input

```text
Tenant,InvoiceID,Customer,IssueDate,DueDate,PaidDate,Amount,Currency
```

One row per invoice, with a positive `Amount` in the declared `--currency`.
Leave `PaidDate` blank for an open invoice. Each `InvoiceID` appears once, and
the register must hold one tenant. Customer names match ignoring case and
spacing. An invoice issued after `--as-at`, or with a due or paid date before
its issue date, is refused. Supply every invoice the customers have been
issued over a period long enough to show a pattern, such as 12 months; the
command can only judge a customer by the history it is given.

## The rule

For each customer, the history is every invoice paid on or before `--as-at`,
so a payment after that date is not hindsight. Days late is the paid date less
the due date, negative for an early payment. From those:

- the median days late;
- the spread, the median absolute difference from that median;
- the threshold, the median plus 3 times the spread or 7 days, whichever is
  more.

Each invoice open at `--as-at` and past its due date is listed:

| Result | When |
| --- | --- |
| `OVERDUE_AGAINST_HISTORY` | the customer has 3 or more paid invoices and the days overdue exceed the threshold |
| `WITHIN_HISTORY` | the customer has 3 or more paid invoices and the days overdue are at most the threshold |
| `NOT_ENOUGH_HISTORY` | the customer has fewer than 3 paid invoices |

The status is `BLOCKED` when any overdue invoice has `NOT_ENOUGH_HISTORY`,
since it cannot be judged by this rule: judge it by hand, or supply more
history. Otherwise it is `REVIEW` when any invoice is `OVERDUE_AGAINST_HISTORY`
and `PASS` when none is. Each listed invoice names the paid invoices its
customer's figures come from. The median and spread are exact and given to one
decimal place, as they are always whole or half days.

The constants (3 paid invoices, 3 times the spread, 7 days) are a stated rule
for a reviewer's queue, not a measured probability of non-payment. They are in
the result under `rule`.

## Limits

- A part payment is not seen: `PaidDate` is the date the invoice was paid in
  full. A part-paid invoice is open.
- Credit notes are refused; net them off the invoice they credit first.
- A customer whose payments have slowed steadily has a history that moves with
  them, so the slowdown itself is not listed. Compare the median across runs.
- An invoice within its customer's pattern can still be doubtful. The command
  does not assess recoverability or an expected credit loss.

## Declined alerts

Two related alerts were considered and declined:

- **Lock-date alerts**, such as a period left unlocked after its close or
  entries posted before the lock date. The lock date lives in the ledger's
  settings, which this package does not read; it works from exported files and
  connects to no ledger.
- **Stale-file alerts**, such as an export older than the period it claims to
  cover. A file's age on disk says nothing reliable about its content, and each
  result already records the SHA-256 of the exact bytes it read, which is the
  evidence a reviewer can check.

## What the tests show

The tests run on fabricated registers whose expected results were worked out by
hand from the rule above. They show that the command applies the rule as
written: the medians, spreads, thresholds and results, the status and exit
code, the refusals and that the output is the same on a second run. They are
circular in one respect: the same reading of the rule wrote both the code and
the expected results, so they cannot show that the rule picks out the invoices
that turn out not to be paid. That judgement is the reviewer's.
