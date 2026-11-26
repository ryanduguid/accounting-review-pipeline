# Pre-posting journal review

`close-control prepost` reads one tenant's draft journals and the same tenant's
posted general-ledger listing, and flags each draft that is worth a look before
it is posted. Each flag carries a reason, a weight and a suggested fix, and
each draft gets a risk score. It reads local files, writes JSON to standard
output and changes nothing: it posts, approves and edits no journal, and a
person decides whether to post. It exits `0` for `PASS`, `2` for `REVIEW` or
`BLOCKED`, and `1` for malformed input or options.

From the component's source:

```text
cargo run --locked --release --bin close-control -- prepost --drafts examples/prepost_drafts.csv --posted examples/journal_entries.csv --currency AUD --period-start 2026-08-01 --period-end 2026-08-31
```

The fabricated drafts are 7 journals for August 2026, compared with the
fabricated July listing that [`jet`](journal-entry-tests.md) uses. The command
exits `2` with `BLOCKED`:

| Draft | Score | Flags |
| --- | --- | --- |
| D-1 | 3 | `DUPLICATE_DRAFT` (D-6) |
| D-2 | 3 | `DUPLICATE_OF_POSTED` (J-1007) |
| D-3 | 3 | `UNBALANCED` (debits exceed credits by 18.00) |
| D-4 | 5 | `UNUSUAL_ACCOUNT_PAIR`, `PERIOD_END`, `NO_NARRATION`, `ROUND_AMOUNT` |
| D-5 | 4 | `OUTSIDE_PERIOD`, `WEEKEND` |
| D-6 | 3 | `DUPLICATE_DRAFT` (D-1) |
| D-7 | 0 | none |

D-1 posts the same rent as posted journal J-1001, a month later under a new
reference, so it is not a duplicate of it: a recurring entry is expected.

## Inputs

Both files use the canonical transaction columns `jet` reads:

```text
Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit
```

`TransactionID` names the journal and every line of a journal shares it. Each
line has one positive `Debit` or `Credit` with at most 2 decimal places, in the
declared `--currency`. Each file must hold one tenant, and the posted listing
must be the drafts' tenant. A posted listing with a header and no rows is
accepted; every account pair is then unusual.

Supply the posted history the drafts should be compared with, such as the last
12 months. The command can only compare a draft with the listing it is given.

## The flags

The status is `BLOCKED` when any draft is unbalanced, since it cannot be
posted; `REVIEW` when any other flag is raised; and `PASS` when none is.

| Flag | Weight | Raised when | Suggested fix |
| --- | --- | --- | --- |
| `UNBALANCED` | 3 | the draft's debits do not equal its credits | correct the lines |
| `OUTSIDE_PERIOD` | 3 | a line is dated before `--period-start` or after `--period-end` (cut-off) | date it in the period it belongs to |
| `DUPLICATE_OF_POSTED` | 3 | a posted journal posts the same amounts to the same accounts, on the same date or under the same non-blank reference | check it is not already posted |
| `DUPLICATE_DRAFT` | 3 | another draft does the same | keep one |
| `UNUSUAL_ACCOUNT_PAIR` | 2 | the draft debits one account and credits another that no posted journal pairs | confirm the accounts |
| `PERIOD_END` | 1 | a line is dated on `--period-end` | keep the cut-off evidence |
| `NO_NARRATION` | 1 | no line carries a description or reference | add a narration |
| `ROUND_AMOUNT` | 1 | a line is at least 1,000 and a whole multiple of 1,000 | attach the calculation |
| `WEEKEND` | 1 | a line is dated on a Saturday or Sunday | confirm the date |

`PERIOD_END`, `NO_NARRATION`, `ROUND_AMOUNT` and `WEEKEND` are the tests `jet`
runs on posted journals, applied by the same code. The result lists the
weights it used.

The risk score is the sum of the weights of the flags raised. The band is
`high` from 6, `medium` from 3, `low` from 1 and `none` at 0. The weights and
bands are a stated ordering for a reviewer's queue, not a measured probability
of error: a draft with no flag is not shown to be right, and a flagged draft
may be correct.

## Limits

- A duplicate needs the same accounts and amounts to the cent. A repeat posted
  to a different account, or split differently, is not found.
- A journal with several debit and credit lines is read as every debit account
  paired with every credit account, so a pair posted only inside a larger
  journal counts as posted.
- The command does not check tax codes, account types or whether an account is
  locked, archived or a control account.

## What the tests show

The tests run on fabricated drafts and a fabricated posted listing whose
expected flags were worked out by hand from the rules above. They show that the
command applies those rules as written: each flag, the scores and bands, the
status and exit code, the refusals and that the output is the same on a second
run. They are circular in one respect: the same reading of the rules wrote both
the code and the expected flags, so they cannot show that the rules catch the
errors a real ledger holds or that the weights rank risk well. That judgement
is the reviewer's.
