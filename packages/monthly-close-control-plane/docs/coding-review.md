# Account and tax-code coding review

`close-control coding` reads one tenant's coded lines (bills, receipts or
journal lines, one row each) and lists the period's lines coded away from the
coding used before the period: a contact posted to a different account, or a
line given a different tax code from the one its contact or its account
usually carries. It reads a local file, writes JSON to standard output and
changes nothing. It exits `0` for `PASS`, `2` for `REVIEW` and `1` for
malformed input or options.

From the component's source:

```text
cargo run --locked --release --bin close-control -- coding --lines examples/coding_lines.csv --currency AUD --period-start 2026-09-01 --period-end 2026-09-30
```

The fabricated listing runs from June to September 2026. The command exits
`2` and lists a power bill posted to repairs instead of electricity, a bank fee
given a GST code after 3 months without one, and a new courier's freight line
given a tax code the freight account has not carried.

## Supply the lines to the period end

```text
Tenant,LineID,Date,Contact,AccountID,TaxCode,Amount,Currency
```

Each `LineID` appears once, each `Amount` has at most 2 decimal places, and
every row carries the declared `--currency`. `Contact` may be empty, as for a
manual journal. A listing with more than one `Tenant` is refused, and a line
dated after `--period-end` is refused, so the result describes the coding as
it stood at the period end. Tax codes are compared exactly as the ledger
writes them; the command does not know what any code means.

Lines dated before `--period-start` are the history. Include enough of it,
such as the 3 months before the period, for usual codings to show.

## What is listed

A coding is usual for a contact, a contact and account, or an account when it
carries at least 3 of the earlier lines and at least 80% of them. Contact names
compare once case and spaces are ignored.

| Code | Listed when |
| --- | --- |
| `UNUSUAL_ACCOUNT` | a period line's contact has a usual account and the line is posted to another. |
| `UNUSUAL_TAX_CODE` | the line's contact has a usual tax code on that account and the line carries another. |
| `UNUSUAL_ACCOUNT_TAX_CODE` | the line's contact has no usual tax code on that account (a new contact, or no contact), the account has a usual tax code across all contacts, and the line carries another. |

Each finding names the line, its date, contact, account, tax code and amount,
the `usual` coding, and how many of the `earlier_lines` carry it
(`usual_lines`).

The result is `PASS` only when at least one line falls in the period and
nothing is listed. A listed line is not shown to be miscoded: a supplier can
start supplying something else. A listing with nothing listed is not shown to
be coded correctly, since a contact or account with too little or too mixed a
history has no usual coding, and a coding wrong in every earlier line is the
usual one.

## Output

The JSON carries `status`, `tenant`, `currency`, `period_start`, `period_end`,
the `boundary` statement, the listing's SHA-256 under `source_sha256.lines`,
`line_count`, `lines_in_period`, `minimum_history`, `usual_share_percent` and
`findings`.
