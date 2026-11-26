# Prompt: is the GST in this BAS period ready to prepare?

Use with the [`close-review` skill](../../packages/monthly-close-control-plane/skills/close-review/SKILL.md)
installed and `close-control` on your PATH. It asks one question before a BAS
is prepared: does the GST recorded this period sit where the period's sales and
purchases say it should, after the settlements you declare?

## You supply

- 2 trial balances in the canonical CSV `xero-trial-balance-export` writes: one
  at the last day of the BAS period (current) and one at the last day of the
  period before it (prior), for the same tenant.
- A GST mapping (`AccountID,GSTClass`) classing each account that moves, and a
  settlements declaration listing each BAS payment, refund and clearing journal
  posted to the GST accounts in the period, or a header alone if there were
  none. [GST reasonableness](../../packages/monthly-close-control-plane/README.md#gst-reasonableness)
  gives both formats.
- A new, empty output folder outside any Git checkout.

## Prompt

```text
Use the close-review skill. Run close-control review on these files:
current trial balance <current.csv>, prior trial balance <prior.csv>,
--gst-mapping <gst_mapping.csv>, --gst-settlements <gst_settlements.csv>,
--require-control gst_reasonableness, output folder <new folder>.
Use the tool's default tolerances.

From the verified pack only, tell me:
1. overall_status, the exit code and the count of BLOCKED and REVIEW exceptions;
2. every gst_reasonableness exception with its reason and reviewer_action,
   copied exactly;
3. any trial_balance_integrity or period_comparison exception, since either
   means the GST figure rests on an incomplete ledger;
4. every control in controls_not_run and the state of each required control.
Then list, as open items for me, what must be explained before the BAS is
prepared. Do not compute BAS labels, judge GST treatment or suggest a journal.
```

## What comes back

An answer drawn only from `close-review-pack.json` and `exceptions.csv`. A
`gst_reasonableness` exception is a lead, not an error: the control compares
net GST movement with 1/10 of taxable sales less 1/10 of creditable purchases,
and it relies on your mapping and declaration, which it cannot verify. A clean
result says the recorded GST is consistent with those inputs, not that the BAS
is right. Drafting the labels is a separate step:
[`close-control bas`](../../packages/monthly-close-control-plane/docs/bas-workpaper.md).
