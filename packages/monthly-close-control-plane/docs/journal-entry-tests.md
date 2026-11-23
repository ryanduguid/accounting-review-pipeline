# Journal-entry tests

`close-control jet` reads one tenant's general-ledger listing and lists the
journals and lines that have a characteristic worth a reviewer's look, with a
Benford first-digit test over the line amounts. It reads a local file, writes
JSON to standard output and changes nothing. It exits `0` for `PASS`, `2` for
`REVIEW` and `1` for malformed input or options.

From the component's source:

```text
cargo run --locked --release --bin close-control -- jet --transactions examples/journal_entries.csv --currency AUD --period-end 2026-07-31
```

The fabricated listing has 7 journals for July 2026. The command exits `2` and
lists an unbalanced receipt, a journal dated after the period end, a
round-sum journal on the last day with no description to 2 accounts nothing
else posts to, and an invoice dated on a Saturday.

## Supply every line of each journal

The listing uses the canonical transaction columns:

```text
Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit
```

`TransactionID` names the journal, and every line of a journal shares it. Each
line has exactly one positive `Debit` or `Credit` with at most 2 decimal
places, and every line must carry the `--currency` declared. A journal may post
to an account once, so combine 2 lines to the same account before the run.
A listing with more than one `Tenant` is refused: test each tenant separately.
A line with no account, date or journal is refused as malformed, not listed.

Supply the whole period's journals, including those posted after the period
end. The tests can only describe the listing they are given; they do not show
that it is complete.

## The tests

`post_closing`, `period_end`, `no_description`, `round_amount` and
`seldom_used_account` follow characteristics ASA 240 Appendix 4 (October 2025)
lists: entries 'made to unrelated, unusual, or seldom-used accounts', entries
'recorded at the end of the period or as post-closing entries that have little
or no explanation or description' and entries 'containing round numbers'.
Paragraph 49(c) requires the auditor to select journal entries made at the end
of a reporting period. That standard is operative for financial reporting
periods beginning on or after 15 December 2026. `unbalanced_journal` and
`weekend` are common integrity and timing checks, not taken from it.

| Test | Lists | Per |
| --- | --- | --- |
| `unbalanced_journal` | A journal whose debits and credits differ, with the difference | journal |
| `post_closing` | A journal with a line dated after `--period-end` | journal |
| `period_end` | A journal with a line dated on `--period-end` | journal |
| `no_description` | A journal none of whose lines has a description or reference | journal |
| `weekend` | A journal with a line dated on a Saturday or Sunday | journal |
| `round_amount` | A line of at least 1,000 that is a whole multiple of 1,000 | line |
| `seldom_used_account` | A line to an account that no other journal in the listing posts to, when the listing has at least 2 journals | line |

A journal-level finding gives the journal's total debits as its amount; a
line-level finding gives the line's signed amount, negative for a credit.
Findings are listed in the table's order, then by journal and account.

Not tested: who made an entry, entries without account numbers (refused as
malformed instead), consistent ending numbers other than 000, public holidays,
and whether an entry is appropriate. A listed entry is not evidence of fraud or
error, and an entry not listed is not shown to be appropriate.

## Benford first-digit test

The test counts the first significant digit of every line amount and compares
the proportions with Benford's expected `log10(1 + 1/d)`. The mean absolute
deviation of the 9 proportions is named by Nigrini's empirical first-digit
bands, as published papers state them: up to 0.006 close conformity, up to
0.012 acceptable conformity, up to 0.015 marginal conformity and above 0.015
nonconformity. The bands are rules of thumb, not a statistical test, and other
cutoffs are in use. Only nonconformity affects the result, which it makes
`REVIEW`. The test runs only on 110 or more
amounts, so that every digit expects at least 5; with fewer, `benford.ran` is
`false` with the reason and the counts, and the test does not affect the status.

Nonconformity says the listing's amounts are not distributed as many natural
populations are. A ledger dominated by fixed fees, recurring charges or a price
list can fail it for no wrong reason, and a manipulated ledger can pass it.

## Result

The JSON records `status`, `tenant`, `currency`, `period_end`, the
`source_sha256` of the listing read, `line_count`, `journal_count`, the `tests`
run, the `findings` (each with `test`, `transaction_id`, `account_id` or `null`,
`date`, `amount` and `reason`) and the `benford` result. A listing with no
lines is `REVIEW`. Keep real listings and results outside the checkout.
