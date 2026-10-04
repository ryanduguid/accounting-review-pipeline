# Check transaction coding against supplied evidence

`close-control classify` compares original and current posting assignments with
explicit purpose and account assertions. It reads local files, writes JSON to
standard output and changes no source, close finding or pack status.

From the component's locked source environment:

```text
uv run --locked close-control classify --original-transactions examples/classification/transactions.csv --current-transactions examples/classification/transactions.csv --coding-evidence examples/classification/coding-evidence.json --currency AUD
```

The fabricated example has 12 software fees posted to Office Expenses and 12
legitimate office expenses with identical amounts and descriptions. The supplied
purpose assertions distinguish them: the software items require review, while
the office items agree. This command is in development source only.

## Supply the same selected population

Both CSVs use the canonical transaction columns:

```text
Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit
```

Declare one tenant, currency and inclusive date range in the evidence JSON.
Every supplied transaction must fit that scope. Use the same file twice for an
initial review. For a later check, preserve the original file and supply the
current assignments for that same selection. This does not establish that the
selection covers the complete ledger.

The evidence file uses exactly these fields:

```json
{
  "schema_version": 1,
  "tenant": "Synthetic",
  "currency": "AUD",
  "period_start": "2025-01-01",
  "period_end": "2025-12-31",
  "original_sha256": "REPLACE_WITH_ORIGINAL_CSV_SHA256",
  "current_sha256": "REPLACE_WITH_CURRENT_CSV_SHA256",
  "expectations": [
    {
      "original_account_id": "610",
      "transaction_id": "software-1",
      "current_account_id": "610",
      "expected_account_id": "620",
      "purpose": "Software access",
      "evidence_reference": "Synthetic supplier bill, software item 1"
    }
  ]
}
```

Supply hashes of the exact CSV bytes. An assertion identifies an original row by
account and transaction ID. It explicitly maps that ID to a current account;
the tool never joins across accounts using a transaction ID alone. If a human
recodes the software item to 620, update `current_account_id` and the current
digest. Keep `original_account_id` and the original digest unchanged.

## Interpret the result

Exit 0 and `PASS` mean every row in a non-empty selection agrees with complete
supplied assertions, with no unmatched current rows. Exit 2 and `REVIEW` retain
coding differences, missing assertions, missing purpose or references, changed
dates, references, descriptions or signed amounts, reused current rows and population changes.
Malformed schemas, duplicate identities, mismatched hashes and invalid scope
exit 1. Empty selections require review.

Each item retains its original posting and whether it disagreed with the
supplied expectation. Agreement after a recode does not erase that history.
Missing evidence never inherits an observed account as the expected account.

Purpose and reference values are unverified assertions. References are labels;
the command does not open bills or authenticate documents, reviewers or
appropriate accounting treatment. A matching hash binds bytes only. A `PASS`
does not approve a correction, establish completeness or clear a close control.
Keep client inputs and redirected results outside the checkout in the approved
workpaper location.
