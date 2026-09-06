# Balanced trial balance, unfinished review

Synthetic case study, reproduced on 6 September 2026. Review aid, not professional advice; a human decides whether the close is acceptable.

## The review question

The trial balance balances, but the preparer's pack still contains a creditors difference, an unmapped account and a comparison across the financial-year reset. Which items need evidence before the reviewer can sign off?

This case uses the existing demonstration files. It does not represent a client close, a user testimonial or a measured time saving.

## Input and result

The current report date is 31 July 2026; the prior report date is 30 June 2026. Both files use the same synthetic tenant. The supplied account mapping and subledger accompany the trial balances. Absolute and percentage variance thresholds are $10,000 and 10%; reconciliation tolerance is one cent.

| Finding | Existing evidence | Reviewer action |
| --- | --- | --- |
| Creditors reconciliation | Trial balance less supplied subledger is -$250 | Trace the $250 difference and document its cause. |
| Account mapping | Operating Expenses account 6000 has no review-group mapping | Confirm the mapping and who reviews the account. |
| Financial-year reset | June and July are in different Australian financial years | Do not treat P&L YTD changes as ordinary monthly variances. |

The result is REVIEW, with eight exceptions and none blocked. These three rows introduce the pack; the other five are threshold-triggered account movements. Eight exceptions do not establish eight accounting errors.

## Reproduce

From `packages/monthly-close-control-plane`, with uv installed:

```bash
uv run --locked --extra dev close-control review --current examples/current_trial_balance.csv --prior examples/prior_trial_balance.csv --mapping examples/account_mapping.csv --subledger examples/subledger_balances.csv --absolute-threshold 10000 --percentage-threshold 0.10 --reconciliation-tolerance 0.01 --review-note examples/review_note.json --output outputs/demo
```

Exit 2 is the expected attention result. Open `outputs/demo/close-summary.md`, inspect `exceptions.csv`, and use `close-review-pack.json` to check the recorded thresholds and input hashes. [The existing worked example](../README.md#worked-example) shows the complete output contract.

## Interpretation

The creditors difference requires a reconciliation explanation, not an automatic adjustment. An unmapped account needs an explicit review destination; adding a label alone does not prove the balance is right. The year-reset warning limits how P&L YTD differences can be interpreted. The reviewer needs a suitable period comparison or supporting workpaper rather than simply accepting the variance labels.

The included review note is also synthetic. It says only that fabricated exceptions were reviewed. Its presence cannot change REVIEW to PASS, approve a journal or establish that a period was closed. A reviewer may record an acknowledgement while several accounting questions remain outstanding.

For a walkthrough, ask the manager to identify the first item they would send back to the preparer and the evidence they would request. Record their actual answer separately; no outside response is claimed in this case study.

## Human decision

Have I obtained and documented enough evidence to resolve the creditors difference, mapping gap and comparison limitations to sign off this close, or must it return to the preparer?
