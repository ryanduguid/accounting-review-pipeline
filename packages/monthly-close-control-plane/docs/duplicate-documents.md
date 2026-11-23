# Duplicate bills and payments

`close-control duplicates` reads one tenant's register of supplier bills and
payments and lists groups worth a reviewer's look before the period closes:
bills that repeat a supplier's reference, bills or payments to one supplier for
the same amount within a few days of each other, and suppliers billed every
month before the period but not in it. It reads a local file, writes JSON to
standard output and changes nothing. It exits `0` for `PASS`, `2` for `REVIEW`
and `1` for malformed input or options.

From the component's source:

```text
cargo run --locked --release --bin close-control -- duplicates --documents examples/duplicates.csv --currency AUD --period-start 2026-09-01 --period-end 2026-09-30
```

The fabricated register runs from June to September 2026. The command exits
`2` and lists a freight bill entered twice under one reference written 2 ways,
2 supply bills for the same amount 3 days apart, 2 payments of one supply bill
on consecutive days, and a power supplier billed in each of June, July and
August with no bill in September.

## Supply the register to the period end

```text
Tenant,DocumentID,Kind,Supplier,Reference,Date,Amount,Currency
```

`Kind` is `BILL` or `PAYMENT`. Each `DocumentID` appears once, each `Amount` is
positive with at most 2 decimal places, and every row carries the declared
`--currency`. `Reference` may be empty. A register with more than one `Tenant`
is refused: review each tenant separately. A document dated after
`--period-end` is refused, so the result describes the register as it stood at
the period end.

Include at least the 3 calendar months before the period, so that recurring
bills and duplicates of earlier bills can be found. A duplicate group is listed only when
at least one of its documents falls in the period.

## What is listed

Supplier names compare once case and spaces are ignored, so `Falador Freight`
and `falador  freight` are one supplier.

| Code | Listed when |
| --- | --- |
| `DUPLICATE_REFERENCE` | 2 or more bills from one supplier share a reference once case, whitespace and ASCII punctuation are ignored (`FF-7781` and `ff7781`), whatever their amounts and dates. Other characters, such as accents and symbols, must match. A reference that is empty once these are ignored is not compared. |
| `DUPLICATE_BILL` | 2 or more bills from one supplier for the same amount, each within 7 days of the one before. |
| `DUPLICATE_PAYMENT` | the same for payments. |
| `MISSING_RECURRING_BILL` | a supplier was billed in each of the 3 calendar months before the period's first month and has no bill dated in the period. |

Each finding names the supplier and lists the documents in it with their kind,
reference, date and amount. A missing recurring bill lists no document.

The result is `PASS` only when at least one document falls in the period and
nothing is listed. A listed group is not shown to be a duplicate, and a supplier
listed as missing may have stopped supplying or billed under another name. A
register with nothing listed is not shown to be complete or free of
duplicates: the command sees only the documents it is given. A duplicate
under another supplier name is not listed, and one with a different
reference is listed only when it has the same amount as a document no more
than 7 days before or after it.

## Output

The JSON carries `status`, `tenant`, `currency`, `period_start`, `period_end`,
the `boundary` statement, the register's SHA-256 under
`source_sha256.documents`, `document_count`, `documents_in_period`,
`window_days` and `findings`.
