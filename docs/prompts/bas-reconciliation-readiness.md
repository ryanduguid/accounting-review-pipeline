# Prompt: are the balance sheet accounts behind this BAS reconciled?

Use with the [`close-review` skill](../../packages/monthly-close-control-plane/skills/close-review/SKILL.md)
installed and `close-control` on your PATH. It asks the question that comes
before the GST one: does every balance sheet account the BAS period touched,
the GST clearing account included, tie to a reconciliation you hold?

## You supply

- 2 trial balances in the canonical CSV `xero-trial-balance-export` writes: one
  at the last day of the BAS period (current) and one at the last day of the
  period before it (prior), for the same tenant.
- Subledger balances (`Tenant,AccountID,SubledgerBalance`) for each account you have
  reconciled at the period end: bank, receivables, payables, GST and any
  clearing account. [Optional mapping and reconciliation inputs](../../packages/monthly-close-control-plane/README.md#optional-mapping-and-reconciliation-inputs)
  gives the format.
- A new, empty output folder outside any Git checkout.

## Prompt

```text
Use the close-review skill. Run close-control review on these files:
current trial balance <current.csv>, prior trial balance <prior.csv>,
--subledger <subledger_balances.csv>,
--require-control subledger_reconciliation,
--require-control reconciliation_coverage, output folder <new folder>.
Use the tool's default tolerances.

From the verified pack only, tell me:
1. overall_status, the exit code and the count of BLOCKED and REVIEW exceptions;
2. every subledger_reconciliation exception: the account, the difference and
   the reviewer_action, copied exactly;
3. every reconciliation_coverage exception: the accounts with a closing
   balance and no reconciliation supplied;
4. every control in controls_not_run and the state of each required control.
Then list, as open items for me, each account to reconcile or explain before
the BAS is prepared. Do not judge whether a difference is acceptable or suggest
a journal.
```

## What comes back

An answer drawn only from `close-review-pack.json` and `exceptions.csv`. A
subledger row counts as a tie whatever its own quality, so a clean result says
each balance agrees with the figure you supplied, not that the reconciliation
behind it is right. Run the [GST readiness prompt](bas-gst-readiness.md) next.
