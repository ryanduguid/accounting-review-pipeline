# Monthly Close Review Pack

**Overall status: REVIEW**

This pack is a review aid for {{client}}. It does not approve a close, post a journal, make a payment, lodge a return, or lock a period.

## Scope

- Current report date(s): 2026-08-31
- Prior report date(s): 2026-07-31
- Material variance thresholds: $10,000.00 and 10.00%
- Reconciliation tolerance: $0.01
- Exceptions: 4 total; 0 blocked; 4 requiring review.
- Client queries drafted: 2.
- Controls not run: gst_reasonableness.

## Source evidence

- `current_trial_balance`: `{{digest}}`
- `prior_trial_balance`: `{{digest}}`
- `subledger`: `{{digest}}`

## Exceptions

| Status | Control | Account | Current | Prior | Difference | Reason |
| --- | --- | --- | --- | --- | --- | --- |
| REVIEW | period_variance | 090 / Business Bank Account | {{amount}} | {{amount}} | {{amount}} | YTD net balance moved by more than both thresholds. |
| REVIEW | subledger_reconciliation | 610 / Trade Debtors | {{amount}} | | {{amount}} | The trial balance differs from the subledger. |
| REVIEW | account_metadata | 800 / Trade Creditors | {{amount}} | {{amount}} | | AccountName changed since the prior period. |
| REVIEW | period_comparison | 477 / Superannuation Payable | {{amount}} | | | Account present in the current period only. |

## Human acknowledgement

Prepared by {{person}}. Reviewed by {{person}} on 2026-09-04.
